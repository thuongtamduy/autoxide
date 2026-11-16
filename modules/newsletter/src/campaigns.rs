//! Chiến dịch: soạn (nháp), sửa, xoá; gửi, hẹn giờ, tạm dừng, tiếp tục, huỷ; gửi thử. Gửi thật
//! nằm ở `campaigns/sending.rs` (job chụp người nhận, job gửi theo lô).
//!
//! Đổi trạng thái là MỘT câu `UPDATE … WHERE status = <trạng thái đã đọc> AND updated_at = <phiên
//! bản>`: hai lệnh cùng lúc thì một bên nhận 412, trên cả hai database. Mỗi lần gửi / hẹn giờ /
//! tiếp tục tăng `run`; job của lượt cũ thấy khác số thì dừng — tạm dừng rồi tiếp tục không để
//! hai chuỗi job cùng chạy.

pub(crate) mod compose;
pub(crate) mod new_posts;
pub(crate) mod sending;

use std::collections::HashMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::EnqueueOptions;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::campaigns::sending::{DeliveryStatus, PrepareJob, SendJob};
use crate::entity::{campaign, delivery};
use crate::events::{self, Change};
use crate::permissions::{NEWSLETTER_MANAGE, NEWSLETTER_READ, NEWSLETTER_SEND};
use crate::service::{NewsletterService, campaign_not_found, now};
use crate::text::{self, Locale};

const SUBJECT_MAX: usize = 200;
const PREHEADER_MAX: usize = 200;
const TEXT_MAX: usize = 200_000;
const HTML_MAX: usize = 500_000;
/// Hẹn giờ xa nhất.
const SCHEDULE_MAX_DAYS: i64 = 366;

/// Where a campaign is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CampaignStatus {
    /// Being written; can be changed, sent, scheduled or deleted.
    Draft,
    /// Starts sending at `scheduled_at`; can still be sent at once. Changing it moves it back
    /// to `draft` (the schedule is dropped): schedule or send it again.
    Scheduled,
    /// Mails are being handed to the mail queue, batch by batch.
    Sending,
    /// Sending stopped for now; `resume` sends to the recipients left.
    Paused,
    /// Every recipient was handled.
    Sent,
    /// Stopped for good; mails not sent yet were skipped.
    Cancelled,
}

impl CampaignStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            CampaignStatus::Draft => "draft",
            CampaignStatus::Scheduled => "scheduled",
            CampaignStatus::Sending => "sending",
            CampaignStatus::Paused => "paused",
            CampaignStatus::Sent => "sent",
            CampaignStatus::Cancelled => "cancelled",
        }
    }

    pub(crate) fn stored(raw: &str) -> CampaignStatus {
        match raw {
            "scheduled" => CampaignStatus::Scheduled,
            "sending" => CampaignStatus::Sending,
            "paused" => CampaignStatus::Paused,
            "sent" => CampaignStatus::Sent,
            "cancelled" => CampaignStatus::Cancelled,
            _ => CampaignStatus::Draft,
        }
    }
}

/// How far a campaign got, by delivery status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub(crate) struct DeliveryCounts {
    /// Recipients taken so far (every confirmed subscriber of the list when sending started).
    pub total: u64,
    /// Not handled yet.
    pub pending: u64,
    /// Handed to the mail queue.
    pub sent: u64,
    /// Refused for good by the receiving server, or the outcome is unknown.
    pub failed: u64,
    /// Not sent: suppressed, blocked, unsubscribed meanwhile, over the advertising limit,
    /// or the campaign was cancelled.
    pub skipped: u64,
    /// Sent mails the receiving server bounced for good (counted in `failed` too).
    pub bounced: u64,
    /// Recipients who unsubscribed with the link of this campaign's mail (one-click or the
    /// preferences page).
    pub unsubscribed: u64,
}

/// Một chiến dịch kèm tiến độ.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Campaign {
    pub row: campaign::Model,
    pub counts: DeliveryCounts,
}

impl Campaign {
    pub(crate) fn status(&self) -> CampaignStatus {
        CampaignStatus::stored(&self.row.status)
    }
}

/// Tóm tắt cho event: không có nội dung thư.
pub(crate) fn summary(row: &campaign::Model) -> Value {
    json!({
        "id": row.id,
        "list_id": row.list_id,
        "status": row.status,
        "advertising": row.advertising,
        "scheduled_at": row.scheduled_at,
        "source": row.source,
        "source_ref": row.source_ref,
    })
}

/// Các trường sửa được; `None` giữ nguyên (khi tạo: giá trị mặc định).
#[derive(Debug, Clone, Default)]
pub(crate) struct CampaignInput {
    pub list_id: Option<Uuid>,
    pub subject: Option<String>,
    /// `Some(None)`: bỏ.
    pub preheader: Option<Option<String>>,
    pub body_text: Option<String>,
    pub body_html: Option<Option<String>>,
    pub locale: Option<Locale>,
    pub advertising: Option<bool>,
}

/// Bộ lọc danh sách chiến dịch.
#[derive(Debug, Clone, Default)]
pub(crate) struct CampaignFilter {
    pub status: Option<CampaignStatus>,
    pub list_id: Option<Uuid>,
}

/// Lệnh đổi trạng thái.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Send,
    Schedule(DateTime<Utc>),
    Pause,
    Resume,
    Cancel,
}

impl Action {
    fn name(self) -> &'static str {
        match self {
            Action::Send => "sent",
            Action::Schedule(_) => "scheduled",
            Action::Pause => "paused",
            Action::Resume => "resumed",
            Action::Cancel => "cancelled",
        }
    }

    fn allowed_from(self) -> &'static [CampaignStatus] {
        match self {
            Action::Send | Action::Schedule(_) => {
                &[CampaignStatus::Draft, CampaignStatus::Scheduled]
            }
            Action::Pause => &[CampaignStatus::Sending],
            Action::Resume => &[CampaignStatus::Paused],
            Action::Cancel => {
                &[CampaignStatus::Scheduled, CampaignStatus::Sending, CampaignStatus::Paused]
            }
        }
    }
}

impl NewsletterService {
    /// Soạn chiến dịch nháp. Cần `newsletter.manage`.
    pub(crate) async fn create_campaign(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: CampaignInput,
    ) -> Result<Campaign, Error> {
        principal.require(&NEWSLETTER_MANAGE)?;
        let mut problems = Vec::new();
        for (name, missing) in [
            ("list_id", input.list_id.is_none()),
            ("subject", input.subject.is_none()),
            ("body_text", input.body_text.is_none()),
        ] {
            if missing {
                problems.push(FieldError::new(name, "required", "is required"));
            }
        }
        let now = now();
        let mut row = blank(tenant, Uuid::nil(), now);
        row.created_by = Some(principal.user_id());
        self.apply(tenant, &mut row, input, &mut problems).await?;
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let transaction = tenant.database().begin_write().await?;
        let row = row.into_active_model().insert(&transaction).await?;
        let event = events::CampaignCreated(Change { before: None, after: Some(summary(&row)) });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::campaign_subject(row.id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(campaign_id = %row.id, "newsletter campaign created");
        Ok(Campaign { row, counts: DeliveryCounts::default() })
    }

    /// Chiến dịch `id` kèm tiến độ. Cần `newsletter.read`.
    pub(crate) async fn get_campaign(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Campaign, Error> {
        principal.require(&NEWSLETTER_READ)?;
        self.find_campaign(tenant, id).await
    }

    pub(crate) async fn find_campaign(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Campaign, Error> {
        let row = campaign::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(campaign_not_found)?;
        let mut counts = self.delivery_counts(tenant, &[id]).await?;
        Ok(Campaign { row, counts: counts.remove(&id).unwrap_or_default() })
    }

    /// Một trang chiến dịch, mới trước (mặc định). Cần `newsletter.read`.
    pub(crate) async fn list_campaigns(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &CampaignFilter,
        page: &PageRequest,
    ) -> Result<Page<Campaign>, Error> {
        principal.require(&NEWSLETTER_READ)?;
        let mut select = campaign::Entity::find().scoped(tenant);
        if let Some(status) = filter.status {
            select = select.filter(campaign::Column::Status.eq(status.as_str()));
        }
        if let Some(list_id) = filter.list_id {
            select = select.filter(campaign::Column::ListId.eq(list_id));
        }
        let found =
            fetch_page(select, campaign::Column::Id, |row| row.id, page, tenant.database().read())
                .await?;
        let ids: Vec<Uuid> = found.data.iter().map(|row| row.id).collect();
        let mut counts = self.delivery_counts(tenant, &ids).await?;
        Ok(found.map(|row| Campaign { counts: counts.remove(&row.id).unwrap_or_default(), row }))
    }

    /// Số lượt gửi theo trạng thái của từng chiến dịch trong `ids`.
    pub(crate) async fn delivery_counts(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, DeliveryCounts>, Error> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        // Nhóm thêm theo `error` để đếm bounce trong cùng câu; số huỷ đăng ký là câu thứ hai.
        let rows: Vec<(Uuid, String, Option<String>, i64)> = delivery::Entity::find()
            .scoped(tenant)
            .select_only()
            .column(delivery::Column::CampaignId)
            .column(delivery::Column::Status)
            .column(delivery::Column::Error)
            .column_as(Expr::col(delivery::Column::Id).count(), "count")
            .filter(delivery::Column::CampaignId.is_in(ids.to_vec()))
            .group_by(delivery::Column::CampaignId)
            .group_by(delivery::Column::Status)
            .group_by(delivery::Column::Error)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        let mut counts: HashMap<Uuid, DeliveryCounts> = HashMap::new();
        for (id, status, error, count) in rows {
            let entry = counts.entry(id).or_default();
            let count = u64::try_from(count).unwrap_or_default();
            entry.total += count;
            match DeliveryStatus::stored(&status) {
                DeliveryStatus::Pending => entry.pending += count,
                DeliveryStatus::Sent => entry.sent += count,
                DeliveryStatus::Failed => entry.failed += count,
                DeliveryStatus::Skipped => entry.skipped += count,
            }
            if DeliveryStatus::stored(&status) == DeliveryStatus::Failed
                && error.as_deref() == Some("hard_bounce")
            {
                entry.bounced += count;
            }
        }
        let unsubscribed: Vec<(Uuid, i64)> = delivery::Entity::find()
            .scoped(tenant)
            .select_only()
            .column(delivery::Column::CampaignId)
            .column_as(Expr::col(delivery::Column::Id).count(), "count")
            .filter(delivery::Column::CampaignId.is_in(ids.to_vec()))
            .filter(delivery::Column::UnsubscribedAt.is_not_null())
            .group_by(delivery::Column::CampaignId)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        for (id, count) in unsubscribed {
            counts.entry(id).or_default().unsubscribed = u64::try_from(count).unwrap_or_default();
        }
        Ok(counts)
    }

    /// Sửa chiến dịch `id` (chỉ khi còn nháp hay đang hẹn giờ). Cần `newsletter.manage`.
    ///
    /// Sửa (thật sự đổi gì đó) một chiến dịch đang hẹn giờ thì nó về `draft`, bỏ giờ hẹn và tăng
    /// `run`: job chụp người nhận đã xếp hàng của lượt cũ thấy khác số thì dừng, và nội dung mới
    /// phải được người có `newsletter.send` hẹn giờ / gửi lại — người chỉ có `newsletter.manage`
    /// không đổi được thư sắp gửi sau lưng người đã duyệt.
    pub(crate) async fn update_campaign(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        input: CampaignInput,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Campaign, Error> {
        principal.require(&NEWSLETTER_MANAGE)?;
        let before = self.find_campaign(tenant, id).await?;
        if expected.is_some_and(|expected| expected != before.row.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        if !matches!(before.status(), CampaignStatus::Draft | CampaignStatus::Scheduled) {
            return Err(not_editable(before.status()));
        }
        let mut problems = Vec::new();
        let mut next = before.row.clone();
        self.apply(tenant, &mut next, input, &mut problems).await?;
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let now = now();
        let unscheduled =
            before.status() == CampaignStatus::Scheduled && content_changed(&before.row, &next);
        let mut row = before.row.clone().into_active_model();
        row.list_id = Set(next.list_id);
        row.subject = Set(next.subject);
        row.preheader = Set(next.preheader);
        row.body_text = Set(next.body_text);
        row.body_html = Set(next.body_html);
        row.locale = Set(next.locale);
        row.advertising = Set(next.advertising);
        row.updated_at = Set(now);
        if unscheduled {
            row.status = Set(CampaignStatus::Draft.as_str().to_owned());
            row.scheduled_at = Set(None);
            row.run = Set(before.row.run + 1);
        }
        let transaction = tenant.database().begin_write().await?;
        let update = campaign::Entity::update(row)
            .validate()?
            .scoped(tenant)
            .filter(campaign::Column::UpdatedAt.eq(expected.unwrap_or(before.row.updated_at)))
            .filter(campaign::Column::Status.eq(before.row.status.as_str()));
        let after = match update.exec(&transaction).await {
            Ok(after) => after,
            Err(sea_orm::DbErr::RecordNotUpdated) => {
                return Err(autoxide_kernel::version::changed());
            }
            Err(error) => return Err(error.into()),
        };
        let event = events::CampaignUpdated(Change {
            before: Some(summary(&before.row)),
            after: Some(summary(&after)),
        });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::campaign_subject(id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        if unscheduled {
            tracing::info!(campaign_id = %id, "scheduled newsletter campaign changed: back to draft");
        }
        Ok(Campaign { row: after, counts: before.counts })
    }

    /// Xoá chiến dịch `id` chưa từng gửi (nháp, hẹn giờ, hay huỷ trước khi gửi). Chiến dịch đã
    /// gửi thì giữ: link huỷ đăng ký trong mail đã gửi trỏ vào nó. Cần `newsletter.manage`.
    pub(crate) async fn delete_campaign(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
    ) -> Result<(), Error> {
        principal.require(&NEWSLETTER_MANAGE)?;
        let before = self.find_campaign(tenant, id).await?;
        if expected.is_some_and(|expected| expected != before.row.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let deletable = matches!(
            before.status(),
            CampaignStatus::Draft | CampaignStatus::Scheduled | CampaignStatus::Cancelled
        ) && before.counts.total == 0;
        if !deletable {
            return Err(Error::conflict(
                "newsletter.campaign_not_deletable",
                "Only a campaign that never sent can be deleted. Mails already sent link back to \
                 this one.",
            ));
        }
        let transaction = tenant.database().begin_write().await?;
        let deleted = campaign::Entity::delete_many()
            .scoped(tenant)
            .filter(campaign::Column::Id.eq(id))
            .filter(campaign::Column::UpdatedAt.eq(expected.unwrap_or(before.row.updated_at)))
            .filter(campaign::Column::Status.eq(before.row.status.as_str()))
            .exec(&transaction)
            .await?;
        if deleted.rows_affected == 0 {
            return Err(autoxide_kernel::version::changed());
        }
        let event =
            events::CampaignDeleted(Change { before: Some(summary(&before.row)), after: None });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::campaign_subject(id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Đổi trạng thái chiến dịch `id`. Cần `newsletter.send`.
    pub(crate) async fn change_state(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        action: Action,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Campaign, Error> {
        principal.require(&NEWSLETTER_SEND)?;
        let before = self.find_campaign(tenant, id).await?;
        if expected.is_some_and(|expected| expected != before.row.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        if !action.allowed_from().contains(&before.status()) {
            return Err(Error::conflict(
                "newsletter.campaign_invalid_transition",
                "The campaign is `{status}`; it cannot be {action}.",
            )
            .with_param("status", before.status().as_str())
            .with_param("action", action.name()));
        }
        let now = now();
        if let Action::Schedule(at) = action
            && (at <= now || at > now + chrono::Duration::days(SCHEDULE_MAX_DAYS))
        {
            return Err(Error::validation(vec![
                FieldError::new(
                    "at",
                    "range",
                    "must be in the future, at most {schedule_max_days} days ahead",
                )
                .with_param("schedule_max_days", SCHEDULE_MAX_DAYS),
            ]));
        }
        if matches!(action, Action::Send | Action::Schedule(_) | Action::Resume) {
            // Thiếu cấu hình thì báo NGAY, không để job hỏng lặng lẽ.
            self.check_ready(&before.row)?;
            self.find_list(tenant, before.row.list_id).await?;
        }
        let mut next = before.row.clone();
        next.updated_at = now;
        let started = matches!(action, Action::Send);
        match action {
            Action::Send => {
                next.status = CampaignStatus::Sending.as_str().to_owned();
                next.started_at = Some(now);
                next.scheduled_at = None;
                next.run += 1;
            }
            Action::Schedule(at) => {
                next.status = CampaignStatus::Scheduled.as_str().to_owned();
                next.scheduled_at = Some(at);
                next.run += 1;
            }
            Action::Pause => next.status = CampaignStatus::Paused.as_str().to_owned(),
            Action::Resume => {
                next.status = CampaignStatus::Sending.as_str().to_owned();
                next.run += 1;
            }
            Action::Cancel => {
                next.status = CampaignStatus::Cancelled.as_str().to_owned();
                next.finished_at = Some(now);
            }
        }
        let transaction = tenant.database().begin_write().await?;
        let updated = campaign::Entity::update_many()
            .scoped(tenant)
            .col_expr(campaign::Column::Status, Expr::value(next.status.clone()))
            .col_expr(campaign::Column::ScheduledAt, Expr::value(next.scheduled_at))
            .col_expr(campaign::Column::StartedAt, Expr::value(next.started_at))
            .col_expr(campaign::Column::FinishedAt, Expr::value(next.finished_at))
            .col_expr(campaign::Column::Run, Expr::value(next.run))
            .col_expr(campaign::Column::UpdatedAt, Expr::value(now))
            .filter(campaign::Column::Id.eq(id))
            .filter(campaign::Column::Status.eq(before.row.status.as_str()))
            .filter(campaign::Column::UpdatedAt.eq(expected.unwrap_or(before.row.updated_at)))
            .exec(&transaction)
            .await?;
        if updated.rows_affected == 0 {
            return Err(autoxide_kernel::version::changed());
        }
        match action {
            Action::Send | Action::Schedule(_) => {
                let run_at = next.scheduled_at;
                self.enqueue_prepare(&transaction, tenant, id, next.run, None, run_at).await?;
            }
            Action::Resume if next.prepared_at.is_some() => {
                self.enqueue_send(&transaction, tenant, id, next.run, 0, None).await?;
            }
            Action::Resume => {
                self.enqueue_prepare(&transaction, tenant, id, next.run, None, None).await?
            }
            Action::Cancel => {
                // Lượt chưa gửi (và chưa có worker nào nhận) thành `skipped`.
                delivery::Entity::update_many()
                    .scoped(tenant)
                    .col_expr(
                        delivery::Column::Status,
                        Expr::value(DeliveryStatus::Skipped.as_str()),
                    )
                    .col_expr(delivery::Column::Error, Expr::value("cancelled"))
                    .filter(delivery::Column::CampaignId.eq(id))
                    .filter(delivery::Column::Status.eq(DeliveryStatus::Pending.as_str()))
                    .filter(delivery::Column::UnsubscribeTokenHash.is_null())
                    .exec(&transaction)
                    .await?;
            }
            Action::Pause => {}
        }
        let change = Change { before: Some(summary(&before.row)), after: Some(summary(&next)) };
        let actor = Actor::from(principal);
        let subject = events::campaign_subject(id);
        match action {
            Action::Send => {
                self.outbox
                    .record(&transaction, tenant, actor, subject, &events::CampaignStarted(change))
                    .await?
            }
            Action::Schedule(_) => {
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        actor,
                        subject,
                        &events::CampaignScheduled(change),
                    )
                    .await?
            }
            Action::Pause => {
                self.outbox
                    .record(&transaction, tenant, actor, subject, &events::CampaignPaused(change))
                    .await?
            }
            Action::Resume => {
                self.outbox
                    .record(&transaction, tenant, actor, subject, &events::CampaignResumed(change))
                    .await?
            }
            Action::Cancel => {
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        actor,
                        subject,
                        &events::CampaignCancelled(change),
                    )
                    .await?
            }
        }
        transaction.commit().await?;
        tracing::info!(campaign_id = %id, action = action.name(), started, "newsletter campaign state changed");
        self.find_campaign(tenant, id).await
    }

    /// Gửi thử chiến dịch `id` tới một địa chỉ (không ghi lượt gửi; link huỷ trong mail thử không
    /// dùng được). Cần `newsletter.send`.
    pub(crate) async fn send_test(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        email: &str,
    ) -> Result<(), Error> {
        principal.require(&NEWSLETTER_SEND)?;
        let mut problems = Vec::new();
        let email = autoxide_kernel::email::checked(email, "email", &mut problems)
            .ok_or_else(|| Error::validation(problems))?;
        let campaign = self.find_campaign(tenant, id).await?;
        let list = self.find_list(tenant, campaign.row.list_id).await?;
        let token = autoxide_kernel::secret::random_token(32)?;
        let token = autoxide_kernel::tenancy::link_token(tenant, &token);
        let mail = self.campaign_mail(&campaign.row, &list, &email, &token, None)?;
        self.mailer.send(tenant, mail).await?;
        tracing::info!(campaign_id = %id, "newsletter test mail queued");
        Ok(())
    }

    /// Kiểm được gửi chưa: URL công khai, trang lựa chọn, địa chỉ người gửi (thư quảng cáo).
    pub(crate) fn check_ready(&self, row: &campaign::Model) -> Result<(), Error> {
        self.public_url.base()?;
        self.preferences_url()?;
        if row.advertising && self.config.postal_address.is_none() {
            return Err(postal_address_missing());
        }
        Ok(())
    }

    /// Đưa job chụp người nhận vào hàng đợi (trong `connection`).
    pub(crate) async fn enqueue_prepare<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        id: Uuid,
        run: i32,
        after: Option<Uuid>,
        run_at: Option<DateTime<Utc>>,
    ) -> Result<(), Error> {
        let key = match after {
            Some(after) => format!("newsletter.prepare:{id}:{run}:{after}"),
            None => format!("newsletter.prepare:{id}:{run}"),
        };
        let mut options = EnqueueOptions::default().unique_key(key);
        if let Some(run_at) = run_at {
            options = options.run_at(run_at);
        }
        self.jobs
            .enqueue(connection, tenant, &PrepareJob { campaign_id: id, run, after }, options)
            .await?;
        Ok(())
    }

    /// Đưa job gửi lô thứ `seq` vào hàng đợi (trong `connection`).
    pub(crate) async fn enqueue_send<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        id: Uuid,
        run: i32,
        seq: u32,
        run_at: Option<DateTime<Utc>>,
    ) -> Result<(), Error> {
        let mut options =
            EnqueueOptions::default().unique_key(format!("newsletter.send:{id}:{run}:{seq}"));
        if let Some(run_at) = run_at {
            options = options.run_at(run_at);
        }
        self.jobs
            .enqueue(connection, tenant, &SendJob { campaign_id: id, run, seq }, options)
            .await?;
        Ok(())
    }

    /// Áp `input` lên `row` rồi kiểm toàn bộ (mọi lỗi vào `problems`).
    async fn apply(
        &self,
        tenant: &TenantContext,
        row: &mut campaign::Model,
        input: CampaignInput,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        if let Some(list_id) = input.list_id {
            if self.lists_by_id(tenant.database().read(), tenant, [list_id]).await?.is_empty() {
                problems.push(FieldError::new("list_id", "invalid", "no list has this id"));
            }
            row.list_id = list_id;
        }
        if let Some(subject) = input.subject {
            match text::line(Some(&subject), "subject", SUBJECT_MAX, problems) {
                Some(subject) => row.subject = subject,
                None => problems.push(FieldError::new("subject", "required", "must not be empty")),
            }
        }
        if let Some(preheader) = input.preheader {
            row.preheader = text::line(preheader.as_deref(), "preheader", PREHEADER_MAX, problems);
        }
        if let Some(body) = input.body_text {
            match text::text(Some(&body), "body_text", TEXT_MAX, problems) {
                Some(body) => row.body_text = body,
                None => {
                    problems.push(FieldError::new("body_text", "required", "must not be empty"))
                }
            }
        }
        if let Some(html) = input.body_html {
            let html = html.map(|html| html.trim().to_owned()).filter(|html| !html.is_empty());
            if html.as_ref().is_some_and(|html| html.len() > HTML_MAX) {
                problems.push(
                    FieldError::new("body_html", "length", "must be at most {html_max} bytes")
                        .with_param("html_max", HTML_MAX),
                );
            }
            row.body_html = html;
        }
        if let Some(locale) = input.locale {
            row.locale = locale.as_str().to_owned();
        }
        if let Some(advertising) = input.advertising {
            row.advertising = advertising;
        }
        Ok(())
    }
}

/// Nội dung hay người nhận của chiến dịch có khác không (những gì `update_campaign` sửa được).
fn content_changed(before: &campaign::Model, after: &campaign::Model) -> bool {
    before.list_id != after.list_id
        || before.subject != after.subject
        || before.preheader != after.preheader
        || before.body_text != after.body_text
        || before.body_html != after.body_html
        || before.locale != after.locale
        || before.advertising != after.advertising
}

/// Chiến dịch nháp rỗng của danh sách `list_id`.
pub(crate) fn blank(tenant: &TenantContext, list_id: Uuid, now: DateTime<Utc>) -> campaign::Model {
    campaign::Model {
        id: Uuid::now_v7(),
        tenant_id: tenant.id().as_uuid(),
        list_id,
        subject: String::new(),
        preheader: None,
        body_text: String::new(),
        body_html: None,
        locale: Locale::En.as_str().to_owned(),
        advertising: false,
        status: CampaignStatus::Draft.as_str().to_owned(),
        scheduled_at: None,
        started_at: None,
        finished_at: None,
        prepared_at: None,
        run: 0,
        source: None,
        source_ref: None,
        created_by: None,
        created_at: now,
        updated_at: now,
    }
}

fn not_editable(status: CampaignStatus) -> Error {
    Error::conflict(
        "newsletter.campaign_not_editable",
        "The campaign is `{status}`; only a draft or scheduled campaign can be changed.",
    )
    .with_param("status", status.as_str())
}

pub(crate) fn postal_address_missing() -> Error {
    Error::unavailable(
        "newsletter.postal_address_missing",
        "Advertising campaigns must show the sender's postal address: set \
         AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS and restart.",
    )
}
