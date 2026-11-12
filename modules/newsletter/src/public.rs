//! Phần công khai (người lạ, website gọi): đăng ký, xác nhận, huỷ một chạm, trang lựa chọn.
//!
//! Đăng ký theo hợp đồng chung (`docs/prompts/reader-input-contract.md`):
//!
//! 1. Kiểm dữ liệu (báo mọi lỗi một lần), rồi `SpamGuard::check` — NGOÀI transaction.
//! 2. `Discard` → không ghi gì. `Accept` / `Quarantine` → một transaction: tạo hoặc giữ người
//!    đăng ký (email unique, chèn `ON CONFLICT DO NOTHING` nên hai request cùng email ra một
//!    dòng), đặt đăng ký, phát token xác nhận, event bằng `Actor::public()`, rồi chèn nonce của
//!    token form (`Checked::claim`). Nonce đã có → rollback (gửi lại cùng token = cùng kết quả).
//! 3. Mail xác nhận xếp hàng SAU commit.
//!
//! Câu trả lời luôn GIỐNG HỆT (202, cùng thân) dù email mới, đang chờ, đã xác nhận, bị chặn hay
//! bị bỏ vì spam: không ai dò được địa chỉ nào đã có trong danh sách. `Quarantine` (gửi quá
//! nhanh…) không bị bỏ mà bị buộc xác nhận qua mail, kể cả ở danh sách không bật double opt-in —
//! người thật bấm link là xong, bot thì không.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::antispam::{Claim, Limits, Submission as Check, Verdict};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::throttle::Throttle;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{campaign, delivery, subscriber, subscription, suppression};
use crate::events::{self, Change};
use crate::lists::List;
use crate::service::{MODULE, NewsletterService, now, to_json};
use crate::subscribers::{
    ImportPerson, SubscriberStatus, SubscriptionStatus, Wanted, set_subscription,
    subscription_summary, upsert_subscriber,
};
use crate::text::{self, Locale};

/// Phạm vi của token form cho đăng ký (`GET /api/v1/antispam/token?scope=newsletter:subscribe`).
pub(crate) const SCOPE: &str = "newsletter:subscribe";
/// Mục đích của token xác nhận trong `newsletter_tokens`.
const CONFIRM: &str = "confirm";
const HOUR: Duration = Duration::from_secs(3600);
const DAY: Duration = Duration::from_secs(24 * 3600);
const LISTS_MAX: usize = 20;

/// Một yêu cầu đăng ký, như client gửi lên.
#[derive(Debug, Clone, Default)]
pub(crate) struct SubscribeRequest {
    pub email: Option<String>,
    pub name: Option<String>,
    pub locale: Option<String>,
    pub lists: Vec<String>,
    pub consent_text: Option<String>,
    pub consent_version: Option<String>,
    pub source: Option<String>,
    pub form_token: Option<String>,
    pub trap: Option<String>,
    pub challenge: Option<String>,
}

/// Người có token (từ một mail chiến dịch) xem gì ở trang lựa chọn.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Preferences {
    pub subscriber: subscriber::Model,
    /// Danh sách công khai và danh sách người này có đăng ký, theo `key`, kèm trạng thái.
    pub lists: Vec<(List, Option<SubscriptionStatus>)>,
}

/// Thay đổi ở trang lựa chọn.
#[derive(Debug, Clone, Default)]
pub(crate) struct PreferencesChange {
    pub name: Option<Option<String>>,
    pub locale: Option<String>,
    /// (`key`, có nhận hay không).
    pub lists: Vec<(String, bool)>,
    pub unsubscribe_all: bool,
    pub consent_text: Option<String>,
    pub consent_version: Option<String>,
}

impl NewsletterService {
    /// Nhận một yêu cầu đăng ký. Trả `Ok(())` như nhau cho mọi trường hợp không phải lỗi dữ
    /// liệu / token / cấu hình.
    pub(crate) async fn subscribe(
        &self,
        principal: Option<&Principal>,
        client: &ClientInfo,
        input: SubscribeRequest,
    ) -> Result<(), Error> {
        let mut problems = Vec::new();
        let email = match input.email.as_deref() {
            Some(email) => text::checked_email(email, "email", &mut problems),
            None => {
                problems.push(FieldError::new("email", "required", "is required"));
                None
            }
        };
        let name = text::line(input.name.as_deref(), "name", 200, &mut problems);
        let locale = text::locale_param(input.locale.as_deref(), "locale", &mut problems);
        let consent_text =
            text::text(input.consent_text.as_deref(), "consent_text", 2000, &mut problems);
        let version =
            text::line(input.consent_version.as_deref(), "consent_version", 50, &mut problems);
        let source = text::line(input.source.as_deref(), "source", 100, &mut problems);
        let public: BTreeMap<String, List> = self
            .public_lists()
            .await?
            .into_iter()
            .map(|list| (list.row.key.clone(), list))
            .collect();
        let mut lists: Vec<&List> = Vec::new();
        if input.lists.is_empty() || input.lists.len() > LISTS_MAX {
            problems.push(FieldError::new(
                "lists",
                "length",
                format!("must have 1 to {LISTS_MAX} list keys"),
            ));
        }
        for (index, key) in input.lists.iter().enumerate() {
            match public.get(key.trim()) {
                Some(list) if !lists.iter().any(|seen| seen.row.id == list.row.id) => {
                    lists.push(list)
                }
                Some(_) => {}
                None => problems.push(FieldError::new(
                    format!("lists[{index}]"),
                    "invalid",
                    "no public list has this key",
                )),
            }
        }
        let Some(email) = email.filter(|_| problems.is_empty()) else {
            return Err(Error::validation(problems));
        };
        if lists.iter().any(|list| list.row.double_opt_in) {
            self.confirm_url()?;
        }

        // Pha một: chống spam, ngoài transaction.
        let checked = self
            .guard
            .check(
                &Check {
                    scope: SCOPE,
                    principal,
                    client,
                    honeypot: input.trap.as_deref(),
                    form_token: input.form_token.as_deref(),
                    challenge: input.challenge.as_deref(),
                    email: Some(&email),
                    content: None,
                    duplicate: false,
                },
                &self.limits()?,
            )
            .await?;
        let hold = match &checked.verdict {
            Verdict::Discard { reason } => {
                tracing::info!(reason, "newsletter subscription dropped");
                return Ok(());
            }
            Verdict::Accept => None,
            Verdict::Quarantine { reasons } => Some(reasons.join(",")),
        };
        // Cần xác nhận khi danh sách bật double opt-in hoặc bị giữ lại. Đếm số mail xác nhận
        // gửi tới địa chỉ này TRƯỚC transaction (bộ đếm ghi bằng kết nối riêng).
        let needs_mail = (hold.is_some() || lists.iter().any(|list| list.row.double_opt_in))
            && !self.all_confirmed(&email, &lists).await?;
        let mail_allowed = needs_mail
            && Throttle::new(
                self.database.clone(),
                MODULE,
                self.config.confirm_mails_per_day,
                DAY,
            )?
            .hit(&format!("confirm:{email}"))
            .await?
            .is_allowed();
        let ip_fingerprint = match client.ip {
            Some(ip) => Some(autoxide_kernel::ip_hash::fingerprint(&self.secret, ip).await?),
            None => None,
        };
        let now = now();
        let consent = to_json(&json!({
            "text": consent_text,
            "version": version,
            "at": now,
            "source": source.unwrap_or_else(|| SCOPE.to_owned()),
            "ip_fingerprint": ip_fingerprint,
        }))?;

        // Pha hai: một transaction ngắn.
        let transaction = self.database.begin_write().await?;
        let person = ImportPerson { email: email.clone(), name, locale, attributes: None };
        let (row, _) = upsert_subscriber(&transaction, &person, now).await?;
        let suppressed = suppression::Entity::find()
            .filter(suppression::Column::Email.eq(email.as_str()))
            .one(&transaction)
            .await?
            .is_some();
        let actor = principal.map_or_else(Actor::public, Actor::from);
        let mut to_confirm: Vec<&List> = Vec::new();
        if row.status != SubscriberStatus::Blocked.as_str() && !suppressed {
            for list in &lists {
                let existing = subscription::Entity::find_by_id((row.id, list.row.id))
                    .one(&transaction)
                    .await?;
                let current = existing.as_ref().map(|row| SubscriptionStatus::stored(&row.status));
                if current == Some(SubscriptionStatus::Confirmed) {
                    continue;
                }
                let double = list.row.double_opt_in || hold.is_some();
                let status = if double {
                    SubscriptionStatus::Unconfirmed
                } else {
                    SubscriptionStatus::Confirmed
                };
                let wanted =
                    Wanted { status, consent: Some(consent.clone()), hold_reason: hold.clone() };
                let saved =
                    set_subscription(&transaction, existing, (row.id, list.row.id), wanted, now)
                        .await?;
                if double {
                    to_confirm.push(list);
                }
                if current != Some(status) {
                    let change = Change {
                        before: None,
                        after: Some(subscription_summary(&saved, &list.row.key)),
                    };
                    let subject = events::subscriber_subject(row.id);
                    if double {
                        let event = events::SubscriptionRequested(change);
                        self.outbox.record(&transaction, actor, subject, &event).await?;
                    } else {
                        let event = events::SubscriptionConfirmed(change);
                        self.outbox.record(&transaction, actor, subject, &event).await?;
                    }
                }
            }
        }
        let issued = if !to_confirm.is_empty() && mail_allowed {
            let ids: Vec<String> = to_confirm.iter().map(|list| list.row.id.to_string()).collect();
            let ttl = chrono::Duration::hours(i64::from(self.config.confirm_hours));
            let subject = row.id.to_string();
            Some(
                self.tokens
                    .issue(&transaction, CONFIRM, &subject, Some(&ids.join(",")), ttl)
                    .await?,
            )
        } else {
            None
        };
        if let Claim::Replay { .. } =
            checked.claim(&transaction, MODULE, Some(&row.id.to_string())).await?
        {
            transaction.rollback().await?;
            return Ok(());
        }
        transaction.commit().await?;
        tracing::info!(subscriber_id = %row.id, lists = lists.len(), confirming = to_confirm.len(), "newsletter subscription request stored");
        if let Some(issued) = issued {
            let locale = locale.unwrap_or_else(|| Locale::stored(&row.locale));
            let mail =
                self.confirmation_mail(&email, locale, &to_confirm, &issued.token, issued.id)?;
            self.mailer.send(mail).await?;
        }
        Ok(())
    }

    /// Người có địa chỉ `email` đã `confirmed` ở mọi danh sách `lists` chưa (đọc trước pha hai).
    async fn all_confirmed(&self, email: &str, lists: &[&List]) -> Result<bool, Error> {
        let read = self.database.read();
        let Some(row) = subscriber::Entity::find()
            .filter(subscriber::Column::Email.eq(email))
            .one(read)
            .await?
        else {
            return Ok(false);
        };
        let confirmed = subscription::Entity::find()
            .filter(subscription::Column::SubscriberId.eq(row.id))
            .filter(subscription::Column::Status.eq(SubscriptionStatus::Confirmed.as_str()))
            .all(read)
            .await?;
        Ok(lists.iter().all(|list| confirmed.iter().any(|row| row.list_id == list.row.id)))
    }

    /// Giới hạn số lần đăng ký (theo IP băm, email, người dùng), trên bảng `newsletter_throttle`.
    fn limits(&self) -> Result<Limits, Error> {
        let throttle = |limit: u32| Throttle::new(self.database.clone(), MODULE, limit, HOUR);
        Ok(Limits {
            ip: Some(throttle(self.config.per_ip_per_hour)?),
            email: Some(throttle(self.config.per_email_per_hour)?),
            user: Some(throttle(self.config.per_user_per_hour)?),
        })
    }

    /// Xác nhận bằng token trong mail: các đăng ký đang chờ thành `confirmed`. Trả các danh sách
    /// vừa (hay đã) được xác nhận.
    pub(crate) async fn confirm(&self, token: &str) -> Result<Vec<List>, Error> {
        let transaction = self.database.begin_write().await?;
        let Some(consumed) = self.tokens.consume(&transaction, CONFIRM, token.trim()).await? else {
            transaction.rollback().await?;
            return Err(Error::bad_request(
                "newsletter.confirmation_invalid",
                "This confirmation link is invalid, has expired or was already used. Subscribe \
                 again to get a new one.",
            ));
        };
        let subscriber_id: Uuid = consumed.subject.parse().map_err(Error::internal)?;
        let list_ids: Vec<Uuid> =
            consumed.data.unwrap_or_default().split(',').filter_map(|id| id.parse().ok()).collect();
        let lists = self.lists_by_id(&transaction, list_ids.iter().copied()).await?;
        let now = now();
        let mut confirmed = Vec::new();
        for id in &list_ids {
            let Some(list) = lists.get(id) else { continue };
            let Some(existing) =
                subscription::Entity::find_by_id((subscriber_id, *id)).one(&transaction).await?
            else {
                continue;
            };
            match SubscriptionStatus::stored(&existing.status) {
                SubscriptionStatus::Unconfirmed => {
                    let mut row = existing.into_active_model();
                    row.status = Set(SubscriptionStatus::Confirmed.as_str().to_owned());
                    row.confirmed_at = Set(Some(now));
                    row.updated_at = Set(now);
                    let saved = row.update(&transaction).await?;
                    let change = Change {
                        before: None,
                        after: Some(subscription_summary(&saved, &list.row.key)),
                    };
                    self.outbox
                        .record(
                            &transaction,
                            Actor::public(),
                            events::subscriber_subject(subscriber_id),
                            &events::SubscriptionConfirmed(change),
                        )
                        .await?;
                    confirmed.push(list.clone());
                }
                SubscriptionStatus::Confirmed => confirmed.push(list.clone()),
                // Đã huỷ sau khi xin đăng ký: không tự đăng ký lại.
                SubscriptionStatus::Unsubscribed => {}
            }
        }
        transaction.commit().await?;
        tracing::info!(subscriber_id = %subscriber_id, lists = confirmed.len(), "newsletter subscription confirmed");
        Ok(confirmed)
    }

    /// Lượt gửi có token huỷ đăng ký `token`, kèm chiến dịch.
    async fn by_token(
        &self,
        token: &str,
    ) -> Result<Option<(delivery::Model, campaign::Model)>, Error> {
        let token = token.trim();
        if token.is_empty() || token.len() > 200 {
            return Ok(None);
        }
        let read = self.database.read();
        let Some(found) = delivery::Entity::find()
            .filter(
                delivery::Column::UnsubscribeTokenHash.eq(autoxide_kernel::secret::sha256(token)),
            )
            .one(read)
            .await?
        else {
            return Ok(None);
        };
        let campaign = campaign::Entity::find_by_id(found.campaign_id).one(read).await?;
        Ok(campaign.map(|campaign| (found, campaign)))
    }

    /// Huỷ một chạm (RFC 8058): huỷ đăng ký danh sách của mail mang token. Token lạ cũng trả
    /// `Ok` (không lộ gì). Có hiệu lực ngay.
    pub(crate) async fn unsubscribe(&self, token: &str) -> Result<(), Error> {
        let Some((found, campaign)) = self.by_token(token).await? else {
            return Ok(());
        };
        let Ok(list) = self.find_list(campaign.list_id).await else {
            return Ok(());
        };
        let transaction = self.database.begin_write().await?;
        let changed = self
            .unsubscribe_in(&transaction, Actor::public(), found.subscriber_id, &list, now())
            .await?;
        transaction.commit().await?;
        if changed {
            tracing::info!(subscriber_id = %found.subscriber_id, list_id = %list.row.id, "newsletter one-click unsubscribe");
        }
        Ok(())
    }

    /// Trang lựa chọn: người mang token, các danh sách công khai và danh sách người đó có đăng ký.
    pub(crate) async fn preferences(&self, token: &str) -> Result<Preferences, Error> {
        let (found, _) = self.by_token(token).await?.ok_or_else(link_invalid)?;
        let row = subscriber::Entity::find_by_id(found.subscriber_id)
            .one(self.database.read())
            .await?
            .ok_or_else(link_invalid)?;
        self.preferences_of(row).await
    }

    async fn preferences_of(&self, row: subscriber::Model) -> Result<Preferences, Error> {
        let memberships: HashMap<Uuid, SubscriptionStatus> = subscription::Entity::find()
            .filter(subscription::Column::SubscriberId.eq(row.id))
            .all(self.database.read())
            .await?
            .into_iter()
            .map(|row| (row.list_id, SubscriptionStatus::stored(&row.status)))
            .collect();
        let mut lists: BTreeMap<String, List> = self
            .public_lists()
            .await?
            .into_iter()
            .map(|list| (list.row.key.clone(), list))
            .collect();
        for list in
            self.lists_by_id(self.database.read(), memberships.keys().copied()).await?.into_values()
        {
            lists.insert(list.row.key.clone(), list);
        }
        let lists = lists
            .into_values()
            .map(|list| {
                let status = memberships.get(&list.row.id).copied();
                (list, status)
            })
            .collect();
        Ok(Preferences { subscriber: row, lists })
    }

    /// Sửa lựa chọn bằng token. Nhận lại một danh sách công khai thì thành `confirmed` ngay (token
    /// đến từ hộp thư của chính người đó), kèm bằng chứng đồng ý nguồn `preferences`.
    pub(crate) async fn update_preferences(
        &self,
        token: &str,
        change: PreferencesChange,
    ) -> Result<Preferences, Error> {
        let before = self.preferences(token).await?;
        let mut problems = Vec::new();
        let name = change.name.map(|name| text::line(name.as_deref(), "name", 200, &mut problems));
        let locale = text::locale_param(change.locale.as_deref(), "locale", &mut problems);
        let consent_text =
            text::text(change.consent_text.as_deref(), "consent_text", 2000, &mut problems);
        let version =
            text::line(change.consent_version.as_deref(), "consent_version", 50, &mut problems);
        let mut wanted: Vec<(&List, Option<SubscriptionStatus>, bool)> = Vec::new();
        for (index, (key, subscribed)) in change.lists.iter().enumerate() {
            match before.lists.iter().find(|(list, _)| list.row.key == key.trim()) {
                Some((list, status)) => {
                    if *subscribed
                        && !list.row.public
                        && *status != Some(SubscriptionStatus::Confirmed)
                    {
                        problems.push(FieldError::new(
                            format!("lists[{index}].subscribed"),
                            "invalid",
                            "this list cannot be joined from the preferences page",
                        ));
                    }
                    wanted.push((list, *status, *subscribed));
                }
                None => problems.push(FieldError::new(
                    format!("lists[{index}].key"),
                    "invalid",
                    "no list has this key",
                )),
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let now = now();
        let id = before.subscriber.id;
        let transaction = self.database.begin_write().await?;
        let mut row = before.subscriber.clone().into_active_model();
        if let Some(name) = name {
            row.name = Set(name);
        }
        if let Some(locale) = locale {
            row.locale = Set(locale.as_str().to_owned());
        }
        if row.is_changed() {
            row.updated_at = Set(now);
            row.update(&transaction).await?;
        }
        let consent = to_json(&json!({
            "text": consent_text,
            "version": version,
            "at": now,
            "source": "preferences",
            "ip_fingerprint": null,
        }))?;
        let subject = events::subscriber_subject(id);
        for (list, status, subscribed) in wanted {
            if change.unsubscribe_all || !subscribed {
                self.unsubscribe_in(&transaction, Actor::public(), id, list, now).await?;
            } else if status != Some(SubscriptionStatus::Confirmed) {
                let existing =
                    subscription::Entity::find_by_id((id, list.row.id)).one(&transaction).await?;
                let wanted = Wanted {
                    status: SubscriptionStatus::Confirmed,
                    consent: Some(consent.clone()),
                    hold_reason: None,
                };
                let saved =
                    set_subscription(&transaction, existing, (id, list.row.id), wanted, now)
                        .await?;
                let change = Change {
                    before: None,
                    after: Some(subscription_summary(&saved, &list.row.key)),
                };
                self.outbox
                    .record(
                        &transaction,
                        Actor::public(),
                        subject.clone(),
                        &events::SubscriptionConfirmed(change),
                    )
                    .await?;
            }
        }
        if change.unsubscribe_all {
            for (list, status) in &before.lists {
                if status.is_some() {
                    self.unsubscribe_in(&transaction, Actor::public(), id, list, now).await?;
                }
            }
        }
        transaction.commit().await?;
        let row = subscriber::Entity::find_by_id(id)
            .one(self.database.read())
            .await?
            .ok_or_else(link_invalid)?;
        self.preferences_of(row).await
    }
}

fn link_invalid() -> Error {
    Error::not_found(
        "newsletter.link_invalid",
        "This link does not match any subscription. Use the link of a more recent newsletter.",
    )
}
