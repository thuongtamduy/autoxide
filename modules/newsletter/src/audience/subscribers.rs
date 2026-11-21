//! Người đăng ký phía quản trị: tìm, đọc, nhập nhiều người (kèm nguồn đồng ý), sửa, chặn, xoá;
//! xuất / xoá mọi thứ về một email (quyền của chủ dữ liệu).

use autoxide_kernel::time::Timestamp;
use std::collections::{HashMap, HashSet};

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page, fetch_page_by};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::{OnConflict, Query};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::audience::lists::List;
use crate::entity::{delivery, subscriber, subscription, suppression};
use crate::events::{self, Change};
use crate::permissions::{NEWSLETTER_EXPORT, NEWSLETTER_MANAGE, NEWSLETTER_READ};
use crate::service::{NewsletterService, from_json, now, subscriber_not_found, to_json};
use crate::text::{self, Locale};

const NAME_MAX: usize = 200;
const ATTRIBUTES_MAX: usize = 16_384;
const IMPORT_MAX: usize = 1000;
/// Số người mỗi transaction khi nhập (SQLite chỉ có một người ghi: giữ transaction ngắn).
const IMPORT_CHUNK: usize = 100;

/// Where a subscriber stands on one list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SubscriptionStatus {
    /// Asked to join; waits for the confirmation link. Gets no campaign.
    Unconfirmed,
    /// Gets the list's campaigns.
    Confirmed,
    /// Left the list. Never put back by an import; only the person can join again.
    Unsubscribed,
}

impl SubscriptionStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            SubscriptionStatus::Unconfirmed => "unconfirmed",
            SubscriptionStatus::Confirmed => "confirmed",
            SubscriptionStatus::Unsubscribed => "unsubscribed",
        }
    }

    pub(crate) fn stored(raw: &str) -> SubscriptionStatus {
        match raw {
            "confirmed" => SubscriptionStatus::Confirmed,
            "unsubscribed" => SubscriptionStatus::Unsubscribed,
            _ => SubscriptionStatus::Unconfirmed,
        }
    }
}

/// Whether a subscriber may get mail at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SubscriberStatus {
    /// Gets the campaigns of the lists they confirmed.
    Active,
    /// Gets nothing, whatever their subscriptions; public sign-ups change nothing.
    Blocked,
}

impl SubscriberStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            SubscriberStatus::Active => "active",
            SubscriberStatus::Blocked => "blocked",
        }
    }
}

/// Một đăng ký, kèm `key` của danh sách (khi danh sách còn).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Membership {
    pub row: subscription::Model,
    pub list_key: Option<String>,
    pub consent: Option<Value>,
}

/// Một người đăng ký, kèm các đăng ký và lý do bị chặn gửi (nếu có).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Subscriber {
    pub row: subscriber::Model,
    pub attributes: Map<String, Value>,
    pub subscriptions: Vec<Membership>,
    /// Lý do trong danh sách chặn (`hard_bounce`…), nếu địa chỉ bị chặn.
    pub suppressed: Option<String>,
}

impl Subscriber {
    pub(crate) fn status(&self) -> SubscriberStatus {
        if self.row.status == "blocked" {
            SubscriberStatus::Blocked
        } else {
            SubscriberStatus::Active
        }
    }
}

/// Tóm tắt người đăng ký cho event: không có email, tên hay thuộc tính.
pub(crate) fn summary(row: &subscriber::Model) -> Value {
    json!({"id": row.id, "status": row.status, "locale": row.locale})
}

/// Tóm tắt một đăng ký cho event.
pub(crate) fn subscription_summary(row: &subscription::Model, list_key: &str) -> Value {
    json!({
        "subscriber_id": row.subscriber_id,
        "list_id": row.list_id,
        "list_key": list_key,
        "status": row.status,
    })
}

/// Bộ lọc tìm người đăng ký.
#[derive(Debug, Clone, Default)]
pub(crate) struct SubscriberFilter {
    /// Một phần địa chỉ email (không phân biệt hoa thường).
    pub email: Option<String>,
    pub list_id: Option<Uuid>,
    /// Trạng thái đăng ký (ở `list_id`, hoặc ở danh sách bất kỳ).
    pub status: Option<SubscriptionStatus>,
    /// `true`: chỉ người bị chặn; `false`: chỉ người không bị chặn.
    pub blocked: Option<bool>,
    /// Sắp theo gì (chiều là `order` của trang).
    pub sort: SubscriberSort,
}

/// How to sort subscribers; the direction is `order`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubscriberSort {
    /// When they were added (the default).
    #[default]
    CreatedAt,
    /// Email address, byte by byte (the same order on every database).
    Email,
    /// Status of the person: `active` or `blocked`.
    Status,
}

/// Một người cần nhập.
#[derive(Debug, Clone, Default)]
pub(crate) struct ImportPerson {
    pub email: String,
    pub name: Option<String>,
    pub locale: Option<Locale>,
    pub attributes: Option<Map<String, Value>>,
}

/// Một lần nhập: mọi người vào mọi danh sách `list_ids`, đã xác nhận, kèm nguồn đồng ý.
#[derive(Debug, Clone, Default)]
pub(crate) struct ImportInput {
    pub people: Vec<ImportPerson>,
    pub list_ids: Vec<Uuid>,
    pub consent_source: Option<String>,
    pub consent_text: Option<String>,
    pub consent_version: Option<String>,
}

/// Result of an import.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ImportReport {
    /// New subscribers (addresses not known before).
    pub created: u64,
    /// Subscriptions created or confirmed.
    pub subscribed: u64,
    /// Subscriptions already confirmed.
    pub unchanged: u64,
    /// Subscriptions left as they are because the person unsubscribed from that list.
    pub kept_unsubscribed: u64,
}

/// Thay đổi của quản trị viên; `None` giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct SubscriberChange {
    /// `Some(None)`: bỏ tên.
    pub name: Option<Option<String>>,
    pub locale: Option<Locale>,
    pub status: Option<SubscriberStatus>,
    pub blocked_reason: Option<Option<String>>,
    pub attributes: Option<Map<String, Value>>,
    /// Huỷ đăng ký ở các danh sách này.
    pub unsubscribe_list_ids: Vec<Uuid>,
}

/// Mọi thứ về một email (bản xuất của chủ dữ liệu).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EmailData {
    pub subscriber: Option<Subscriber>,
    pub deliveries: Vec<delivery::Model>,
    pub suppression: Option<suppression::Model>,
}

impl NewsletterService {
    /// Một trang người đăng ký, mới trước (mặc định). Cần `newsletter.read`.
    pub(crate) async fn list_subscribers(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &SubscriberFilter,
        page: &PageRequest,
    ) -> Result<Page<Subscriber>, Error> {
        principal.require(&NEWSLETTER_READ)?;
        let mut select = subscriber::Entity::find().scoped(tenant);
        if let Some(email) =
            filter.email.as_deref().map(str::trim).filter(|email| !email.is_empty())
        {
            select =
                select.filter(autoxide_kernel::search::contains(subscriber::Column::Email, email));
        }
        if let Some(blocked) = filter.blocked {
            let status = subscriber::Column::Status;
            select =
                select.filter(if blocked { status.eq("blocked") } else { status.ne("blocked") });
        }
        if filter.list_id.is_some() || filter.status.is_some() {
            let mut inner = Query::select();
            inner
                .column(subscription::Column::SubscriberId)
                .from(subscription::Entity)
                .and_where(subscription::Column::TenantId.eq(tenant.id().as_uuid()));
            if let Some(list_id) = filter.list_id {
                inner.and_where(subscription::Column::ListId.eq(list_id));
            }
            if let Some(status) = filter.status {
                inner.and_where(subscription::Column::Status.eq(status.as_str()));
            }
            select = select.filter(subscriber::Column::Id.in_subquery(inner.to_owned()));
        }
        let read = tenant.database().read();
        let id = |row: &subscriber::Model| row.id;
        let found = match filter.sort {
            SubscriberSort::CreatedAt => {
                fetch_page(select, subscriber::Column::Id, id, page, read).await?
            }
            SubscriberSort::Email => {
                let key = |row: &subscriber::Model| SortValue::Text(row.email.clone());
                let column = subscriber::Column::Email;
                fetch_page_by(select, column, key, subscriber::Column::Id, id, page, read).await?
            }
            SubscriberSort::Status => {
                let key = |row: &subscriber::Model| SortValue::Text(row.status.clone());
                let column = subscriber::Column::Status;
                fetch_page_by(select, column, key, subscriber::Column::Id, id, page, read).await?
            }
        };
        let Page { data, pagination } = found;
        let data = self.describe(tenant.database().read(), tenant, data).await?;
        Ok(Page { data, pagination })
    }

    /// Người đăng ký `id`. Cần `newsletter.read`.
    pub(crate) async fn get_subscriber(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Subscriber, Error> {
        principal.require(&NEWSLETTER_READ)?;
        self.find_subscriber(tenant, id).await
    }

    pub(crate) async fn find_subscriber(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Subscriber, Error> {
        let row = subscriber::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(subscriber_not_found)?;
        self.describe(tenant.database().read(), tenant, vec![row])
            .await?
            .pop()
            .ok_or_else(subscriber_not_found)
    }

    /// Đọc kèm đăng ký, `key` danh sách và lý do bị chặn cho `rows` (giữ thứ tự).
    pub(crate) async fn describe<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        rows: Vec<subscriber::Model>,
    ) -> Result<Vec<Subscriber>, Error> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let memberships = subscription::Entity::find()
            .scoped(tenant)
            .filter(subscription::Column::SubscriberId.is_in(ids))
            .all(connection)
            .await?;
        let lists = self
            .lists_by_id(
                connection,
                tenant,
                memberships.iter().map(|row| row.list_id).collect::<HashSet<_>>(),
            )
            .await?;
        let emails: Vec<String> = rows.iter().map(|row| row.email.clone()).collect();
        let suppressed: HashMap<String, String> = suppression::Entity::find()
            .scoped(tenant)
            .filter(suppression::Column::Email.is_in(emails))
            .all(connection)
            .await?
            .into_iter()
            .map(|row| (row.email, row.reason))
            .collect();
        let mut by_subscriber: HashMap<Uuid, Vec<Membership>> = HashMap::new();
        for row in memberships {
            let consent = row.consent.as_deref().map(from_json).transpose()?;
            let list_key = lists.get(&row.list_id).map(|list: &List| list.row.key.clone());
            by_subscriber.entry(row.subscriber_id).or_default().push(Membership {
                row,
                list_key,
                consent,
            });
        }
        rows.into_iter()
            .map(|row| {
                let mut subscriptions = by_subscriber.remove(&row.id).unwrap_or_default();
                subscriptions.sort_by(|a, b| a.list_key.cmp(&b.list_key));
                Ok(Subscriber {
                    attributes: from_json(&row.attributes)?,
                    suppressed: suppressed.get(&row.email).cloned(),
                    subscriptions,
                    row,
                })
            })
            .collect()
    }

    /// Nhập nhiều người vào các danh sách, ở trạng thái `confirmed` — cần nguồn đồng ý
    /// (`consent_source`). Người đã huỷ đăng ký ở một danh sách thì giữ nguyên. Cần
    /// `newsletter.manage`.
    pub(crate) async fn import_subscribers(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: ImportInput,
    ) -> Result<ImportReport, Error> {
        principal.require(&NEWSLETTER_MANAGE)?;
        let mut problems = Vec::new();
        if input.people.is_empty() || input.people.len() > IMPORT_MAX {
            problems.push(
                FieldError::new("subscribers", "length", "must have 1 to {import_max} people")
                    .with_param("import_max", IMPORT_MAX),
            );
        }
        let mut people = Vec::with_capacity(input.people.len());
        let mut seen = HashSet::new();
        for (index, person) in input.people.into_iter().enumerate() {
            let field = |name: &str| format!("subscribers[{index}].{name}");
            let email =
                autoxide_kernel::email::checked(&person.email, &field("email"), &mut problems);
            let name = text::line(person.name.as_deref(), &field("name"), NAME_MAX, &mut problems);
            if let Some(attributes) = &person.attributes {
                check_attributes(attributes, &field("attributes"), &mut problems)?;
            }
            if let Some(email) = email
                && seen.insert(email.clone())
            {
                people.push(ImportPerson { email, name, ..person });
            }
        }
        if input.list_ids.is_empty() {
            problems.push(FieldError::new("list_ids", "required", "needs at least one list"));
        }
        let lists = self
            .lists_by_id(tenant.database().read(), tenant, input.list_ids.iter().copied())
            .await?;
        for (index, id) in input.list_ids.iter().enumerate() {
            if !lists.contains_key(id) {
                problems.push(FieldError::new(
                    format!("list_ids[{index}]"),
                    "invalid",
                    "no list has this id",
                ));
            }
        }
        let source =
            text::line(input.consent_source.as_deref(), "consent_source", 100, &mut problems);
        if source.is_none() {
            problems.push(FieldError::new(
                "consent_source",
                "required",
                "say where these people agreed to get mail (e.g. `signup-form-2024`)",
            ));
        }
        let consent_text =
            text::text(input.consent_text.as_deref(), "consent_text", 2000, &mut problems);
        let version =
            text::line(input.consent_version.as_deref(), "consent_version", 50, &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let now = now();
        let consent = to_json(&json!({
            "text": consent_text,
            "version": version,
            "at": now,
            "source": source,
            "ip_fingerprint": null,
        }))?;
        let mut list_ids: Vec<Uuid> = lists.keys().copied().collect();
        list_ids.sort();
        let mut report = ImportReport::default();
        for chunk in people.chunks(IMPORT_CHUNK) {
            let mut part = ImportReport::default();
            let transaction = tenant.database().begin_write().await?;
            for person in chunk {
                let (row, created) = upsert_subscriber(&transaction, tenant, person, now).await?;
                if created {
                    part.created += 1;
                } else {
                    let mut active = row.clone().into_active_model();
                    if let Some(name) = &person.name {
                        active.name = Set(Some(name.clone()));
                    }
                    if let Some(locale) = person.locale {
                        active.locale = Set(locale.as_str().to_owned());
                    }
                    if let Some(attributes) = &person.attributes {
                        active.attributes = Set(to_json(attributes)?);
                    }
                    if active.is_changed() {
                        active.updated_at = Set(now);
                        subscriber::Entity::update(active)
                            .validate()?
                            .scoped(tenant)
                            .exec(&transaction)
                            .await?;
                    }
                }
                for list_id in &list_ids {
                    let existing =
                        subscription::Entity::find_by_id((tenant.id().as_uuid(), row.id, *list_id))
                            .scoped(tenant)
                            .one(&transaction)
                            .await?;
                    match existing.as_ref().map(|row| SubscriptionStatus::stored(&row.status)) {
                        Some(SubscriptionStatus::Confirmed) => part.unchanged += 1,
                        Some(SubscriptionStatus::Unsubscribed) => part.kept_unsubscribed += 1,
                        Some(SubscriptionStatus::Unconfirmed) | None => {
                            let wanted = Wanted {
                                status: SubscriptionStatus::Confirmed,
                                consent: Some(consent.clone()),
                                hold_reason: None,
                            };
                            set_subscription(
                                &transaction,
                                tenant,
                                existing,
                                (row.id, *list_id),
                                wanted,
                                now,
                            )
                            .await?;
                            part.subscribed += 1;
                        }
                    }
                }
            }
            let event = events::SubscribersImported(Change {
                before: None,
                after: Some(json!({
                    "list_ids": list_ids,
                    "created": part.created,
                    "subscribed": part.subscribed,
                    "unchanged": part.unchanged,
                    "kept_unsubscribed": part.kept_unsubscribed,
                })),
            });
            self.outbox.record(&transaction, tenant, Actor::from(principal), None, &event).await?;
            transaction.commit().await?;
            report.created += part.created;
            report.subscribed += part.subscribed;
            report.unchanged += part.unchanged;
            report.kept_unsubscribed += part.kept_unsubscribed;
        }
        tracing::info!(
            created = report.created,
            subscribed = report.subscribed,
            "newsletter subscribers imported"
        );
        Ok(report)
    }

    /// Sửa người đăng ký `id`. Cần `newsletter.manage`.
    pub(crate) async fn update_subscriber(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        change: SubscriberChange,
        expected: Option<Timestamp>,
    ) -> Result<Subscriber, Error> {
        principal.require(&NEWSLETTER_MANAGE)?;
        let before = self.find_subscriber(tenant, id).await?;
        if expected.is_some_and(|expected| expected != before.row.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let mut problems = Vec::new();
        let name =
            change.name.map(|name| text::line(name.as_deref(), "name", NAME_MAX, &mut problems));
        let reason = change
            .blocked_reason
            .map(|reason| text::line(reason.as_deref(), "blocked_reason", 255, &mut problems));
        if let Some(attributes) = &change.attributes {
            check_attributes(attributes, "attributes", &mut problems)?;
        }
        let lists = self
            .lists_by_id(
                tenant.database().read(),
                tenant,
                change.unsubscribe_list_ids.iter().copied(),
            )
            .await?;
        for (index, list_id) in change.unsubscribe_list_ids.iter().enumerate() {
            if !lists.contains_key(list_id) {
                problems.push(FieldError::new(
                    format!("unsubscribe_list_ids[{index}]"),
                    "invalid",
                    "no list has this id",
                ));
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let now = now();
        let mut row = before.row.clone().into_active_model();
        if let Some(name) = name {
            row.name = Set(name);
        }
        if let Some(locale) = change.locale {
            row.locale = Set(locale.as_str().to_owned());
        }
        if let Some(status) = change.status {
            row.status = Set(status.as_str().to_owned());
            if status == SubscriberStatus::Active {
                row.blocked_reason = Set(None);
            }
        }
        if let Some(reason) = reason {
            row.blocked_reason = Set(reason);
        }
        if let Some(attributes) = &change.attributes {
            row.attributes = Set(to_json(attributes)?);
        }
        row.updated_at = Set(now);
        let transaction = tenant.database().begin_write().await?;
        let update = subscriber::Entity::update(row)
            .validate()?
            .scoped(tenant)
            .filter(subscriber::Column::UpdatedAt.eq(expected.unwrap_or(before.row.updated_at)));
        let after = match update.exec(&transaction).await {
            Ok(after) => after,
            Err(sea_orm::DbErr::RecordNotUpdated) => {
                return Err(autoxide_kernel::version::changed());
            }
            Err(error) => return Err(error.into()),
        };
        let actor = Actor::from(principal);
        for list in lists.values() {
            self.unsubscribe_in(&transaction, tenant, actor, id, list, now).await?;
        }
        let event = events::SubscriberUpdated(Change {
            before: Some(summary(&before.row)),
            after: Some(summary(&after)),
        });
        self.outbox
            .record(&transaction, tenant, actor, events::subscriber_subject(id), &event)
            .await?;
        transaction.commit().await?;
        self.sync_consents_quietly(tenant, id).await;
        self.find_subscriber(tenant, id).await
    }

    /// Huỷ đăng ký `subscriber_id` ở `list` (chưa huỷ thì thôi), kèm event. Trả `true` nếu có đổi.
    pub(crate) async fn unsubscribe_in<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        actor: Actor,
        subscriber_id: Uuid,
        list: &List,
        now: Timestamp,
    ) -> Result<bool, Error> {
        let updated = subscription::Entity::update_many()
            .scoped(tenant)
            .col_expr(
                subscription::Column::Status,
                sea_orm::sea_query::Expr::value(SubscriptionStatus::Unsubscribed.as_str()),
            )
            .col_expr(
                subscription::Column::UnsubscribedAt,
                sea_orm::sea_query::Expr::value(Some(now)),
            )
            .col_expr(subscription::Column::UpdatedAt, sea_orm::sea_query::Expr::value(now))
            .filter(subscription::Column::SubscriberId.eq(subscriber_id))
            .filter(subscription::Column::ListId.eq(list.row.id))
            .filter(subscription::Column::Status.ne(SubscriptionStatus::Unsubscribed.as_str()))
            .exec(connection)
            .await?;
        if updated.rows_affected == 0 {
            return Ok(false);
        }
        let event = events::SubscriptionUnsubscribed(Change {
            before: None,
            after: Some(json!({
                "subscriber_id": subscriber_id,
                "list_id": list.row.id,
                "list_key": list.row.key,
                "status": SubscriptionStatus::Unsubscribed.as_str(),
            })),
        });
        self.outbox
            .record(connection, tenant, actor, events::subscriber_subject(subscriber_id), &event)
            .await?;
        Ok(true)
    }

    /// Xoá hẳn người đăng ký `id` (đăng ký, lượt gửi, token). Cần `newsletter.manage`.
    pub(crate) async fn delete_subscriber(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&NEWSLETTER_MANAGE)?;
        let transaction = tenant.database().begin_write().await?;
        let row = subscriber::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(subscriber_not_found)?;
        self.erase(&transaction, tenant, Actor::from(principal), &row).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Mọi thứ module giữ về `email` (yêu cầu xuất của chủ dữ liệu). Cần `newsletter.export`.
    pub(crate) async fn email_data(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        email: &str,
    ) -> Result<EmailData, Error> {
        principal.require(&NEWSLETTER_EXPORT)?;
        let email = required_email(email)?;
        let read = tenant.database().read();
        let row = subscriber::Entity::find()
            .scoped(tenant)
            .filter(subscriber::Column::Email.eq(email.as_str()))
            .one(read)
            .await?;
        let deliveries = match &row {
            Some(row) => {
                let mut rows = delivery::Entity::find()
                    .scoped(tenant)
                    .filter(delivery::Column::SubscriberId.eq(row.id))
                    .all(read)
                    .await?;
                rows.sort_by_key(|row| row.id);
                rows
            }
            None => Vec::new(),
        };
        let subscriber = match row {
            Some(row) => self.describe(read, tenant, vec![row]).await?.pop(),
            None => None,
        };
        let suppression = suppression::Entity::find()
            .scoped(tenant)
            .filter(suppression::Column::Email.eq(email.as_str()))
            .one(read)
            .await?;
        Ok(EmailData { subscriber, deliveries, suppression })
    }

    /// Xoá hẳn mọi thứ về `email` (yêu cầu xoá của chủ dữ liệu), trừ dòng của danh sách chặn
    /// (giữ để không bao giờ gửi lại cho địa chỉ đã bounce / khiếu nại / bị chặn tay). Cần
    /// `newsletter.export`. Trả `true` nếu có người đăng ký bị xoá.
    pub(crate) async fn erase_email(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        email: &str,
    ) -> Result<bool, Error> {
        principal.require(&NEWSLETTER_EXPORT)?;
        let email = required_email(email)?;
        let transaction = tenant.database().begin_write().await?;
        let row = subscriber::Entity::find()
            .scoped(tenant)
            .filter(subscriber::Column::Email.eq(email.as_str()))
            .one(&transaction)
            .await?;
        let Some(row) = row else {
            return Ok(false);
        };
        self.erase(&transaction, tenant, Actor::from(principal), &row).await?;
        transaction.commit().await?;
        tracing::info!(subscriber_id = %row.id, "newsletter data of an email address erased");
        Ok(true)
    }

    /// Xoá người đăng ký `row` cùng đăng ký, lượt gửi, token, kèm event (không có email).
    async fn erase<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        actor: Actor,
        row: &subscriber::Model,
    ) -> Result<(), Error> {
        let subscriptions = subscription::Entity::delete_many()
            .scoped(tenant)
            .filter(subscription::Column::SubscriberId.eq(row.id))
            .exec(connection)
            .await?;
        let deliveries = delivery::Entity::delete_many()
            .scoped(tenant)
            .filter(delivery::Column::SubscriberId.eq(row.id))
            .exec(connection)
            .await?;
        self.tokens.forget(connection, tenant, &row.id.to_string()).await?;
        subscriber::Entity::delete_by_id(row.id).scoped(tenant).exec(connection).await?;
        let event = events::SubscriberErased(Change {
            before: Some(json!({
                "id": row.id,
                "subscriptions": subscriptions.rows_affected,
                "deliveries": deliveries.rows_affected,
            })),
            after: None,
        });
        self.outbox
            .record(connection, tenant, actor, events::subscriber_subject(row.id), &event)
            .await?;
        Ok(())
    }
}

/// Chèn người đăng ký `person` nếu email chưa có (an toàn khi nhiều request cùng lúc), rồi đọc
/// lại. Trả dòng và `true` nếu vừa tạo.
pub(crate) async fn upsert_subscriber<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    person: &ImportPerson,
    now: Timestamp,
) -> Result<(subscriber::Model, bool), Error> {
    let attributes = person.attributes.clone().unwrap_or_default();
    let fresh = subscriber::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        email: Set(person.email.clone()),
        name: Set(person.name.clone()),
        locale: Set(person.locale.unwrap_or_default().as_str().to_owned()),
        status: Set(SubscriberStatus::Active.as_str().to_owned()),
        blocked_reason: Set(None),
        attributes: Set(to_json(&attributes)?),
        soft_bounces: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let inserted = subscriber::Entity::insert(fresh)
        .on_conflict(
            OnConflict::columns([subscriber::Column::TenantId, subscriber::Column::Email])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(connection)
        .await?;
    let row = subscriber::Entity::find()
        .scoped(tenant)
        .filter(subscriber::Column::Email.eq(person.email.as_str()))
        .one(connection)
        .await?
        .ok_or_else(subscriber_not_found)?;
    Ok((row, inserted > 0))
}

/// Trạng thái muốn đặt cho một đăng ký.
#[derive(Debug, Clone)]
pub(crate) struct Wanted {
    pub status: SubscriptionStatus,
    /// JSON bằng chứng đồng ý mới; `None` giữ bằng chứng cũ.
    pub consent: Option<String>,
    pub hold_reason: Option<String>,
}

/// Đặt đăng ký `(subscriber_id, list_id)` theo `wanted`: chèn nếu chưa có (`existing` là
/// `None`; đụng dòng vừa được chèn song song thì cập nhật dòng đó), không thì cập nhật.
pub(crate) async fn set_subscription<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    existing: Option<subscription::Model>,
    (subscriber_id, list_id): (Uuid, Uuid),
    wanted: Wanted,
    now: Timestamp,
) -> Result<subscription::Model, Error> {
    let Wanted { status, consent, hold_reason } = wanted;
    let confirmed_at = (status == SubscriptionStatus::Confirmed).then_some(now);
    let existing = match existing {
        Some(row) => row,
        None => {
            let fresh = subscription::ActiveModel {
                tenant_id: Set(tenant.id().as_uuid()),
                subscriber_id: Set(subscriber_id),
                list_id: Set(list_id),
                status: Set(status.as_str().to_owned()),
                consent: Set(consent.clone()),
                hold_reason: Set(hold_reason.clone()),
                requested_at: Set(now),
                confirmed_at: Set(confirmed_at),
                unsubscribed_at: Set(None),
                updated_at: Set(now),
                ledger_state: Set(String::new()),
            };
            let inserted = subscription::Entity::insert(fresh)
                .on_conflict(
                    OnConflict::columns([
                        subscription::Column::TenantId,
                        subscription::Column::SubscriberId,
                        subscription::Column::ListId,
                    ])
                    .do_nothing()
                    .to_owned(),
                )
                .exec_without_returning(connection)
                .await?;
            let row =
                subscription::Entity::find_by_id((tenant.id().as_uuid(), subscriber_id, list_id))
                    .scoped(tenant)
                    .one(connection)
                    .await?
                    .ok_or_else(subscriber_not_found)?;
            if inserted > 0 {
                return Ok(row);
            }
            row
        }
    };
    let mut row = existing.into_active_model();
    row.status = Set(status.as_str().to_owned());
    if consent.is_some() {
        row.consent = Set(consent);
    }
    row.hold_reason = Set(hold_reason);
    if status == SubscriptionStatus::Confirmed {
        row.confirmed_at = Set(confirmed_at);
    } else {
        row.requested_at = Set(now);
    }
    row.unsubscribed_at = Set(None);
    row.updated_at = Set(now);
    Ok(subscription::Entity::update(row).validate()?.scoped(tenant).exec(connection).await?)
}

/// Thuộc tính: JSON object, tối đa 16 KiB.
fn check_attributes(
    attributes: &Map<String, Value>,
    field: &str,
    problems: &mut Vec<FieldError>,
) -> Result<(), Error> {
    if to_json(attributes)?.len() > ATTRIBUTES_MAX {
        problems.push(
            FieldError::new(field, "length", "must be at most {attributes_max} bytes of JSON")
                .with_param("attributes_max", ATTRIBUTES_MAX),
        );
    }
    Ok(())
}

/// Email của yêu cầu xuất / xoá, đã chuẩn hoá; sai dạng thì 422.
fn required_email(email: &str) -> Result<String, Error> {
    let mut problems = Vec::new();
    autoxide_kernel::email::checked(email, "email", &mut problems)
        .ok_or_else(|| Error::validation(problems))
}
