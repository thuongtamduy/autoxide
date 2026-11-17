//! Gửi chiến dịch bằng hai loại job:
//!
//! 1. `newsletter.prepare` chụp người nhận: duyệt các đăng ký `confirmed` của danh sách theo
//!    `subscriber_id` (keyset), chèn một dòng `pending` vào `newsletter_deliveries` cho mỗi người,
//!    mỗi khúc một transaction ngắn (SQLite chỉ có một người ghi). Chèn bằng `ON CONFLICT DO
//!    NOTHING`: chạy lại không sinh dòng trùng. Chạy quá lâu thì đưa job tiếp theo (từ chỗ đã
//!    dừng) vào hàng đợi. Xong thì đưa job gửi lô đầu vào hàng đợi.
//! 2. `newsletter.send` gửi một lô: lấy tối đa `AUTOXIDE_NEWSLETTER_BATCH_SIZE` dòng `pending`,
//!    kiểm lại ngay trước khi gửi (bị chặn, đã huỷ, bị khoá, quá giới hạn thư quảng cáo →
//!    `skipped`), rồi với mỗi dòng: **nhận** dòng bằng một câu `UPDATE … WHERE status = 'pending'
//!    AND unsubscribe_token_hash IS NULL` (đúng một worker thắng, trên cả hai database), đưa mail
//!    vào hàng đợi của module `mail` với `dedupe_key` = ID lượt gửi, rồi đánh `sent`. Xong lô thì
//!    hẹn lô sau theo tốc độ cấu hình (`run_at`); hết dòng `pending` thì chiến dịch thành `sent`.
//!
//! Không gửi trùng: job chạy lại hay hai worker chạy song song cùng một lô thì chỉ một bên nhận
//! được mỗi dòng; mail module `mail` còn chặn thêm một lớp bằng `dedupe_key`. Khe hở còn lại
//! (worker chết sau khi đưa mail vào hàng đợi, trước khi đánh `sent`) được xử theo kiểu "nhiều
//! nhất một lần": dòng đã nhận quá thời hạn giữ job (300 giây) mà vẫn `pending` thì thành
//! `failed` với lỗi `interrupted` — không gửi lại vì không biết mail đã đi hay chưa.
//!
//! Một lô chỉ là vài trăm lần ghi vào database (đưa mail vào hàng đợi), nên xong rất xa dưới
//! 240 giây — job queue coi job giữ quá 300 giây là chết.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{Job, JobContext, JobHandler};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, Set};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::audience::lists::List;
use crate::audience::subscribers::{SubscriberStatus, SubscriptionStatus};
use crate::campaigns::{CampaignStatus, summary};
use crate::entity::{campaign, delivery, subscriber, subscription, suppression};
use crate::events::{self, Change};
use crate::service::{NewsletterService, now};

pub(crate) const PREPARE: &str = "newsletter.prepare";
pub(crate) const SEND: &str = "newsletter.send";
/// Số người chụp mỗi transaction.
const PREPARE_CHUNK: u64 = 500;
/// Job chụp chạy quá chừng này thì nhường cho job tiếp theo.
const PREPARE_BUDGET: Duration = Duration::from_secs(60);
/// Thời hạn job queue giữ một job: dòng đã nhận quá chừng này là của worker đã chết.
const LEASE_SECONDS: i64 = 300;

/// Where one recipient of a campaign is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DeliveryStatus {
    Pending,
    Sent,
    Failed,
    Skipped,
}

impl DeliveryStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            DeliveryStatus::Pending => "pending",
            DeliveryStatus::Sent => "sent",
            DeliveryStatus::Failed => "failed",
            DeliveryStatus::Skipped => "skipped",
        }
    }

    pub(crate) fn stored(raw: &str) -> DeliveryStatus {
        match raw {
            "sent" => DeliveryStatus::Sent,
            "failed" => DeliveryStatus::Failed,
            "skipped" => DeliveryStatus::Skipped,
            _ => DeliveryStatus::Pending,
        }
    }
}

/// Job chụp người nhận của chiến dịch, lượt `run`, từ sau người đăng ký `after`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PrepareJob {
    pub campaign_id: Uuid,
    pub run: i32,
    pub after: Option<Uuid>,
}

impl Job for PrepareJob {
    const KIND: &'static str = PREPARE;
}

/// Job gửi lô thứ `seq` của chiến dịch, lượt `run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SendJob {
    pub campaign_id: Uuid,
    pub run: i32,
    pub seq: u32,
}

impl Job for SendJob {
    const KIND: &'static str = SEND;
}

pub(crate) struct Preparer(pub Arc<NewsletterService>);

#[async_trait::async_trait]
impl JobHandler for Preparer {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("newsletter prepare job without a tenant"))?;
        let job: PrepareJob =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.prepare(tenant, &job).await
    }
}

pub(crate) struct Sender(pub Arc<NewsletterService>);

#[async_trait::async_trait]
impl JobHandler for Sender {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("newsletter send job without a tenant"))?;
        let job: SendJob = serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.send_batch(tenant, &job).await.map(|_| ())
    }
}

/// Lô vừa gửi được gì.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Batch {
    pub sent: u64,
    pub skipped: u64,
    pub failed: u64,
    /// Chiến dịch vừa xong.
    pub finished: bool,
}

impl NewsletterService {
    /// Chiến dịch còn ở lượt `run` và đang gửi không (đọc lại mỗi khúc: tạm dừng có hiệu lực ngay).
    async fn still_sending(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        run: i32,
    ) -> Result<Option<campaign::Model>, Error> {
        let row =
            campaign::Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?;
        Ok(row.filter(|row| {
            row.run == run && CampaignStatus::stored(&row.status) == CampaignStatus::Sending
        }))
    }

    /// Chạy job chụp người nhận.
    pub(crate) async fn prepare(
        &self,
        tenant: &TenantContext,
        job: &PrepareJob,
    ) -> Result<(), Error> {
        let Some(row) = campaign::Entity::find_by_id(job.campaign_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
        else {
            return Ok(());
        };
        if row.run != job.run {
            return Ok(());
        }
        match CampaignStatus::stored(&row.status) {
            CampaignStatus::Scheduled => self.start_scheduled(tenant, &row).await?,
            CampaignStatus::Sending => {}
            _ => return Ok(()),
        }
        let started = Instant::now();
        let mut after = job.after;
        loop {
            let Some(row) = self.still_sending(tenant, job.campaign_id, job.run).await? else {
                return Ok(());
            };
            let mut select = subscription::Entity::find()
                .scoped(tenant)
                .select_only()
                .column(subscription::Column::SubscriberId)
                .filter(subscription::Column::ListId.eq(row.list_id))
                .filter(subscription::Column::Status.eq(SubscriptionStatus::Confirmed.as_str()))
                .order_by_asc(subscription::Column::SubscriberId)
                .limit(PREPARE_CHUNK);
            if let Some(after) = after {
                select = select.filter(subscription::Column::SubscriberId.gt(after));
            }
            let chunk: Vec<Uuid> = select.into_tuple().all(tenant.database().read()).await?;
            let transaction = tenant.database().begin_write().await?;
            let Some(last) = chunk.last().copied() else {
                let marked = campaign::Entity::update_many()
                    .scoped(tenant)
                    .col_expr(campaign::Column::PreparedAt, Expr::value(Some(now())))
                    .filter(campaign::Column::Id.eq(row.id))
                    .filter(campaign::Column::Run.eq(job.run))
                    .exec(&transaction)
                    .await?;
                if marked.rows_affected == 1 {
                    self.enqueue_send(&transaction, tenant, row.id, job.run, 0, None).await?;
                }
                transaction.commit().await?;
                return Ok(());
            };
            let now = now();
            let rows = chunk.iter().map(|subscriber_id| delivery::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                campaign_id: Set(row.id),
                subscriber_id: Set(*subscriber_id),
                status: Set(DeliveryStatus::Pending.as_str().to_owned()),
                attempts: Set(0),
                unsubscribe_token_hash: Set(None),
                claimed_at: Set(None),
                sent_at: Set(None),
                error: Set(None),
                unsubscribed_at: Set(None),
                created_at: Set(now),
            });
            delivery::Entity::insert_many(rows)
                .on_conflict(
                    OnConflict::columns([
                        delivery::Column::TenantId,
                        delivery::Column::CampaignId,
                        delivery::Column::SubscriberId,
                    ])
                    .do_nothing()
                    .to_owned(),
                )
                .exec_without_returning(&transaction)
                .await?;
            if started.elapsed() > PREPARE_BUDGET {
                self.enqueue_prepare(&transaction, tenant, row.id, job.run, Some(last), None)
                    .await?;
                transaction.commit().await?;
                return Ok(());
            }
            transaction.commit().await?;
            after = Some(last);
        }
    }

    /// Chiến dịch hẹn giờ tới giờ: `scheduled` → `sending`.
    async fn start_scheduled(
        &self,
        tenant: &TenantContext,
        row: &campaign::Model,
    ) -> Result<(), Error> {
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        let started = campaign::Entity::update_many()
            .scoped(tenant)
            .col_expr(campaign::Column::Status, Expr::value(CampaignStatus::Sending.as_str()))
            .col_expr(campaign::Column::StartedAt, Expr::value(Some(now)))
            .col_expr(campaign::Column::UpdatedAt, Expr::value(now))
            .filter(campaign::Column::Id.eq(row.id))
            .filter(campaign::Column::Status.eq(CampaignStatus::Scheduled.as_str()))
            .filter(campaign::Column::Run.eq(row.run))
            .exec(&transaction)
            .await?;
        if started.rows_affected == 1 {
            let mut after = row.clone();
            after.status = CampaignStatus::Sending.as_str().to_owned();
            let change = Change { before: Some(summary(row)), after: Some(summary(&after)) };
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::system(),
                    events::campaign_subject(row.id),
                    &events::CampaignStarted(change),
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Chạy job gửi một lô.
    pub(crate) async fn send_batch(
        &self,
        tenant: &TenantContext,
        job: &SendJob,
    ) -> Result<Batch, Error> {
        let mut batch = Batch::default();
        let Some(row) = self.still_sending(tenant, job.campaign_id, job.run).await? else {
            return Ok(batch);
        };
        let list = self.find_list(tenant, row.list_id).await?;
        let pending = delivery::Entity::find()
            .scoped(tenant)
            .filter(delivery::Column::CampaignId.eq(row.id))
            .filter(delivery::Column::Status.eq(DeliveryStatus::Pending.as_str()))
            .order_by_asc(delivery::Column::Id)
            .limit(u64::from(self.config.batch_size))
            .all(tenant.database().read())
            .await?;
        if pending.is_empty() {
            batch.finished = self.finish(tenant, &row, job.run).await?;
            return Ok(batch);
        }
        let now = now();
        let lease = now - chrono::Duration::seconds(LEASE_SECONDS);
        let mut fresh = Vec::new();
        for item in pending {
            match (item.unsubscribe_token_hash.is_some(), item.claimed_at) {
                (false, _) => fresh.push(item),
                // Đã có worker nhận mà quá thời hạn: không biết mail đã đi chưa — không gửi lại.
                (true, Some(claimed)) if claimed < lease => {
                    if self
                        .mark(tenant, item.id, DeliveryStatus::Failed, "interrupted", true)
                        .await?
                    {
                        batch.failed += 1;
                    }
                }
                // Worker khác đang gửi dòng này.
                (true, _) => {}
            }
        }
        let checks = self.recipient_checks(tenant, &row, &list, &fresh, now).await?;
        let mut ads = checks.ads;
        for item in fresh {
            let Some(person) = checks.subscribers.get(&item.subscriber_id) else {
                self.mark(tenant, item.id, DeliveryStatus::Skipped, "erased", false).await?;
                batch.skipped += 1;
                continue;
            };
            let reason = if checks.suppressed.contains(&person.email) {
                Some("suppressed")
            } else if person.status == SubscriberStatus::Blocked.as_str() {
                Some("blocked")
            } else if !checks.confirmed.contains(&item.subscriber_id) {
                Some("unsubscribed")
            } else if checks
                .consented
                .as_ref()
                .is_some_and(|ids| !ids.contains(&item.subscriber_id))
            {
                // Sổ đồng ý chung không còn đồng ý (rút ở `compliance`, D-366).
                Some("consent_withdrawn")
            } else if row.advertising
                && self.config.advertising_max_per_day > 0
                && ads.get(&item.subscriber_id).copied().unwrap_or(0)
                    >= u64::from(self.config.advertising_max_per_day)
            {
                Some("advertising_limit")
            } else {
                None
            };
            if let Some(reason) = reason {
                if self.mark(tenant, item.id, DeliveryStatus::Skipped, reason, false).await? {
                    batch.skipped += 1;
                }
                continue;
            }
            let to = (person.email.as_str(), person.name.as_deref());
            match self.deliver(tenant, &row, &list, &item, to).await? {
                Some(DeliveryStatus::Sent) => {
                    batch.sent += 1;
                    if row.advertising {
                        *ads.entry(item.subscriber_id).or_default() += 1;
                    }
                }
                Some(DeliveryStatus::Failed) => batch.failed += 1,
                _ => {}
            }
        }
        // Lô sau, giãn theo tốc độ cấu hình.
        let millis = i64::from(self.config.batch_size) * 60_000
            / i64::from(Ord::max(self.config.send_rate_per_minute, 1));
        let next = Some(now + chrono::Duration::milliseconds(millis));
        let transaction = tenant.database().begin_write().await?;
        self.enqueue_send(&transaction, tenant, row.id, job.run, job.seq + 1, next).await?;
        transaction.commit().await?;
        tracing::info!(
            campaign_id = %row.id,
            sent = batch.sent,
            skipped = batch.skipped,
            failed = batch.failed,
            "newsletter batch handled"
        );
        Ok(batch)
    }

    /// Nhận dòng `item`, đưa mail vào hàng đợi, đánh `sent`. `None`: worker khác đã nhận.
    async fn deliver(
        &self,
        tenant: &TenantContext,
        row: &campaign::Model,
        list: &List,
        item: &delivery::Model,
        to: (&str, Option<&str>),
    ) -> Result<Option<DeliveryStatus>, Error> {
        // Database giữ bản băm của phần thô; link mang khách (`autoxide_kernel::tenancy::link_token`).
        let token = autoxide_kernel::secret::random_token(32)?;
        let link = autoxide_kernel::tenancy::link_token(tenant, &token);
        let mail = match self.campaign_mail(tenant, row, list, to, &link, Some(item.id)).await {
            Ok(mail) => mail,
            // Mẫu không dựng được với người nhận này (chiến dịch `template`): không thử lại.
            Err(error) if error.kind() == ErrorKind::Validation => {
                tracing::warn!(delivery_id = %item.id, code = error.code(), "newsletter campaign template failed for a recipient");
                self.mark(tenant, item.id, DeliveryStatus::Failed, "template_error", false).await?;
                return Ok(Some(DeliveryStatus::Failed));
            }
            Err(error) => return Err(error),
        };
        let claimed = delivery::Entity::update_many()
            .scoped(tenant)
            .col_expr(
                delivery::Column::UnsubscribeTokenHash,
                Expr::value(Some(autoxide_kernel::secret::sha256(&token))),
            )
            .col_expr(delivery::Column::ClaimedAt, Expr::value(Some(now())))
            .col_expr(delivery::Column::Attempts, Expr::col(delivery::Column::Attempts).add(1))
            .filter(delivery::Column::Id.eq(item.id))
            .filter(delivery::Column::Status.eq(DeliveryStatus::Pending.as_str()))
            .filter(delivery::Column::UnsubscribeTokenHash.is_null())
            .exec(tenant.database().write())
            .await?;
        if claimed.rows_affected != 1 {
            return Ok(None);
        }
        match self.mailer.send(tenant, mail).await {
            Ok(()) => {
                delivery::Entity::update_many()
                    .scoped(tenant)
                    .col_expr(delivery::Column::Status, Expr::value(DeliveryStatus::Sent.as_str()))
                    .col_expr(delivery::Column::SentAt, Expr::value(Some(now())))
                    .filter(delivery::Column::Id.eq(item.id))
                    .exec(tenant.database().write())
                    .await?;
                Ok(Some(DeliveryStatus::Sent))
            }
            // Địa chỉ không gửi được (sai dạng): không thử lại.
            Err(error) if error.kind() == ErrorKind::Validation => {
                self.mark(tenant, item.id, DeliveryStatus::Failed, "invalid_address", true).await?;
                Ok(Some(DeliveryStatus::Failed))
            }
            Err(error) => {
                // Không xếp được mail: trả dòng lại để lần chạy sau gửi.
                delivery::Entity::update_many()
                    .scoped(tenant)
                    .col_expr(
                        delivery::Column::UnsubscribeTokenHash,
                        Expr::value(Option::<String>::None),
                    )
                    .col_expr(
                        delivery::Column::ClaimedAt,
                        Expr::value(Option::<DateTime<Utc>>::None),
                    )
                    .filter(delivery::Column::Id.eq(item.id))
                    .filter(delivery::Column::Status.eq(DeliveryStatus::Pending.as_str()))
                    .exec(tenant.database().write())
                    .await?;
                Err(error)
            }
        }
    }

    /// Đánh dòng `id` (còn `pending`) sang `status` với lý do. `claimed`: dòng đã có worker
    /// nhận (`false`: chỉ đánh khi chưa ai nhận). Trả `true` nếu có đổi.
    async fn mark(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        status: DeliveryStatus,
        reason: &str,
        claimed: bool,
    ) -> Result<bool, Error> {
        let mut update = delivery::Entity::update_many()
            .scoped(tenant)
            .col_expr(delivery::Column::Status, Expr::value(status.as_str()))
            .col_expr(delivery::Column::Error, Expr::value(Some(reason.to_owned())))
            .filter(delivery::Column::Id.eq(id))
            .filter(delivery::Column::Status.eq(DeliveryStatus::Pending.as_str()));
        if !claimed {
            update = update.filter(delivery::Column::UnsubscribeTokenHash.is_null());
        }
        Ok(update.exec(tenant.database().write()).await?.rows_affected == 1)
    }

    /// Hết dòng `pending`: chiến dịch thành `sent`, kèm event có số đếm. Trả `true` nếu vừa xong.
    async fn finish(
        &self,
        tenant: &TenantContext,
        row: &campaign::Model,
        run: i32,
    ) -> Result<bool, Error> {
        let counts =
            self.delivery_counts(tenant, &[row.id]).await?.remove(&row.id).unwrap_or_default();
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        let finished = campaign::Entity::update_many()
            .scoped(tenant)
            .col_expr(campaign::Column::Status, Expr::value(CampaignStatus::Sent.as_str()))
            .col_expr(campaign::Column::FinishedAt, Expr::value(Some(now)))
            .col_expr(campaign::Column::UpdatedAt, Expr::value(now))
            .filter(campaign::Column::Id.eq(row.id))
            .filter(campaign::Column::Status.eq(CampaignStatus::Sending.as_str()))
            .filter(campaign::Column::Run.eq(run))
            .exec(&transaction)
            .await?;
        if finished.rows_affected == 1 {
            let mut after = summary(row);
            if let Some(object) = after.as_object_mut() {
                object.insert("status".to_owned(), json!(CampaignStatus::Sent.as_str()));
                object.insert(
                    "counts".to_owned(),
                    json!({
                        "total": counts.total,
                        "sent": counts.sent,
                        "failed": counts.failed,
                        "skipped": counts.skipped,
                    }),
                );
            }
            let change = Change { before: Some(summary(row)), after: Some(after) };
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::system(),
                    events::campaign_subject(row.id),
                    &events::CampaignSent(change),
                )
                .await?;
            tracing::info!(campaign_id = %row.id, sent = counts.sent, "newsletter campaign sent");
        }
        transaction.commit().await?;
        Ok(finished.rows_affected == 1)
    }

    /// Đọc một lần cho cả lô: người đăng ký, ai còn `confirmed`, địa chỉ bị chặn, số thư quảng
    /// cáo mỗi người đã nhận trong 24 giờ.
    async fn recipient_checks(
        &self,
        tenant: &TenantContext,
        row: &campaign::Model,
        list: &List,
        items: &[delivery::Model],
        now: DateTime<Utc>,
    ) -> Result<Checks, Error> {
        let read = tenant.database().read();
        let ids: Vec<Uuid> = items.iter().map(|item| item.subscriber_id).collect();
        if ids.is_empty() {
            return Ok(Checks::default());
        }
        let subscribers: HashMap<Uuid, subscriber::Model> = subscriber::Entity::find()
            .scoped(tenant)
            .filter(subscriber::Column::Id.is_in(ids.clone()))
            .all(read)
            .await?
            .into_iter()
            .map(|row| (row.id, row))
            .collect();
        let confirmed: HashSet<Uuid> = subscription::Entity::find()
            .scoped(tenant)
            .select_only()
            .column(subscription::Column::SubscriberId)
            .filter(subscription::Column::ListId.eq(list.row.id))
            .filter(subscription::Column::SubscriberId.is_in(ids.clone()))
            .filter(subscription::Column::Status.eq(SubscriptionStatus::Confirmed.as_str()))
            .into_tuple()
            .all(read)
            .await?
            .into_iter()
            .collect();
        let emails: Vec<String> = subscribers.values().map(|row| row.email.clone()).collect();
        let suppressed: HashSet<String> = suppression::Entity::find()
            .scoped(tenant)
            .select_only()
            .column(suppression::Column::Email)
            .filter(suppression::Column::Email.is_in(emails))
            .into_tuple()
            .all(read)
            .await?
            .into_iter()
            .collect();
        let mut ads = HashMap::new();
        if row.advertising && self.config.advertising_max_per_day > 0 {
            let since = now - chrono::Duration::hours(24);
            let advertising = campaign::Entity::find()
                .scoped(tenant)
                .select_only()
                .column(campaign::Column::Id)
                .filter(campaign::Column::Advertising.eq(true))
                .into_query();
            let sent: Vec<Uuid> = delivery::Entity::find()
                .scoped(tenant)
                .select_only()
                .column(delivery::Column::SubscriberId)
                .filter(delivery::Column::SubscriberId.is_in(ids))
                .filter(delivery::Column::Status.eq(DeliveryStatus::Sent.as_str()))
                .filter(delivery::Column::SentAt.gt(since))
                .filter(delivery::Column::CampaignId.in_subquery(advertising))
                .into_tuple()
                .all(read)
                .await?;
            for id in sent {
                *ads.entry(id).or_insert(0u64) += 1;
            }
        }
        // Sổ đồng ý chung: hỏi lại ngay trước khi gửi; sổ lỗi thì cả lô lỗi (job thử lại) — không
        // gửi khi không chứng minh được đồng ý.
        let people: Vec<(Uuid, String)> = subscribers
            .values()
            .filter(|row| confirmed.contains(&row.id))
            .map(|row| (row.id, row.email.clone()))
            .collect();
        let consented = self.consented(tenant, list.row.id, &people).await?.map(|answers| {
            people.iter().zip(answers).filter(|(_, yes)| *yes).map(|((id, _), _)| *id).collect()
        });
        Ok(Checks { subscribers, confirmed, suppressed, ads, consented })
    }
}

/// Dữ liệu kiểm lại trước khi gửi một lô.
#[derive(Debug, Default)]
struct Checks {
    subscribers: HashMap<Uuid, subscriber::Model>,
    confirmed: HashSet<Uuid>,
    suppressed: HashSet<String>,
    ads: HashMap<Uuid, u64>,
    /// Người còn đồng ý ở sổ chung; `None`: chương trình không có sổ.
    consented: Option<HashSet<Uuid>>,
}
