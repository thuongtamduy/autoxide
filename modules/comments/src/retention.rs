//! Job dọn dẹp hằng ngày `comments.cleanup`:
//!
//! - bình luận chờ duyệt / spam CHƯA TỪNG HIỆN (`published_at IS NULL`) quá
//!   `AUTOXIDE_COMMENTS_RETENTION_DAYS` (tính từ lúc tạo) bị xoá hẳn (bình luận gốc còn trả lời
//!   thì xoá mềm, để luồng giữ nguyên). Bình luận đã từng hiện rồi về chờ duyệt (bị báo cáo đủ
//!   ngưỡng, sửa lại, người kiểm duyệt giữ lại) hay bị đánh spam KHÔNG bao giờ tự bị xoá: vài báo
//!   cáo vô danh không được thành cách xoá bình luận của người khác; người kiểm duyệt quyết;
//! - dấu vết IP quá `AUTOXIDE_COMMENTS_IP_RETENTION_DAYS` bị xoá, cả trong khoá người báo của
//!   khách (`ip:…` thành `expired:<id báo cáo>`);
//! - nonce, bộ đếm, token đã hết hạn bị dọn.
//!
//! Chạy lại bao nhiêu lần cũng ra cùng kết quả.
//!
//! Job theo lịch chạy một lần cho mỗi database (bước 33): bình luận hết hạn (kèm event) dọn theo
//! từng khách có bình luận trên database đó; xoá dấu vết IP, khoá người báo, nonce, bộ đếm, token
//! quét cả database.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::database::Database;
use autoxide_kernel::db::{AllTenants, Queries as _, TenantKey};
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext, TenantId};
use autoxide_kernel::throttle::Throttle;
use serde_json::json;
use uuid::Uuid;

use crate::entity::{comment, report, subscription, target};
use crate::events::{self, Change};
use crate::posting::authoring::{Deletion, soft_delete};
use crate::service::{CommentsService, MODULE, Status, now, shift_counts};

pub(crate) const CLEANUP: &str = "comments.cleanup";
/// Số bình luận xét mỗi vòng.
const BATCH: u64 = 200;

/// Đã dọn những gì.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Cleaned {
    /// Xoá hẳn.
    pub deleted: u64,
    /// Xoá mềm (bình luận gốc còn trả lời).
    pub cleared: u64,
    pub fingerprints: u64,
    /// Khoá người báo của khách đã bỏ dấu vết IP.
    pub reporters: u64,
}

/// Trạng thái hết hạn được (khi chưa từng hiện).
const EXPIRING: [&str; 2] = ["pending", "spam"];

/// Tiền tố khoá người báo của khách (`moderation/reports.rs`).
pub(crate) const GUEST_REPORTER: &str = "ip:";

/// Một bình luận hết hạn đã được dọn thế nào.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Expired {
    Deleted,
    Cleared,
}

pub(crate) struct Cleanup {
    pub service: Arc<CommentsService>,
    /// Sổ database: tra ngữ cảnh của từng khách có dữ liệu trên database của job.
    pub databases: DatabaseRegistry,
}

#[autoxide_kernel::async_trait]
impl JobHandler for Cleanup {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let all = job.all_tenants("comments retention sweep");
        self.service.cleanup(&self.databases, &job.database, all, now()).await.map(|_| ())
    }
}

impl CommentsService {
    /// Dọn `database` theo thời hạn, coi `now` là bây giờ (test truyền thời điểm tương lai). `all`: handle
    /// mọi khách của chính `database` (job của cả database, `JobContext::all_tenants`).
    pub(crate) async fn cleanup(
        &self,
        databases: &DatabaseRegistry,
        database: &Database,
        all: AllTenants<'_>,
        now: Timestamp,
    ) -> Result<Cleaned, Error> {
        let mut cleaned = Cleaned::default();
        let before = now
            - autoxide_kernel::time::Duration::from_hours(
                24 * (i64::from(self.config.retention_days)),
            );
        // Khách có bình luận trên database này: mọi bình luận có dòng đích (`ensure_target`), nên
        // đọc bảng đích — nhỏ, khoá chính có `tenant_id` đứng đầu.
        let tenants: Vec<(TenantKey,)> = all
            .read()
            .find::<target::Model>()
            .select((target::Model::TENANT_ID,))
            .group_by(target::Model::TENANT_ID)
            .all()
            .await?;
        for (id,) in tenants {
            let id = id.as_uuid();
            match databases.tenant(TenantId::from_uuid(id)).await {
                Ok(tenant) => self.expire_tenant(&tenant, before, &mut cleaned).await?,
                // Khách đã bị xoá khỏi sổ: dữ liệu còn lại do việc xoá khách lo, không đoán ở đây.
                Err(error) => {
                    tracing::warn!(tenant_id = %id, error = %error, "comments cleanup skipped a tenant");
                }
            }
        }
        let ip_before = now
            - autoxide_kernel::time::Duration::from_hours(
                24 * (i64::from(self.config.ip_retention_days)),
            );
        let write = all.begin_write().await?;
        // Job hạn lưu IP, chạy cho mọi khách trên database (handle `all_tenants`).
        cleaned.fingerprints = write
            .update::<comment::Model>()
            .set(comment::Model::IP_FINGERPRINT, None)
            .filter(comment::Model::IP_FINGERPRINT.is_not_null())
            .filter(comment::Model::CREATED_AT.lt(ip_before))
            .exec()
            .await?;
        write.commit().await?;
        // Khoá người báo của khách (`ip:<dấu vết>`) cũng là dấu vết IP: quá hạn thì thành
        // `expired:<id báo cáo>` — vẫn duy nhất trong `(tenant_id, comment_id, reporter_key)`.
        loop {
            // Job hạn lưu người báo cáo, mọi khách trên database; mỗi lô một transaction ngắn (SQLite chỉ
            // một bên ghi một lúc: không giữ khoá suốt cả đợt quét).
            let batch = all.begin_write().await?;
            let expired: Vec<Uuid> = batch
                .find::<report::Model>()
                .filter(report::Model::REPORTER_KEY.starts_with(GUEST_REPORTER))
                .filter(report::Model::CREATED_AT.lt(ip_before))
                .order_asc(report::Model::ID)
                .limit(BATCH)
                .all()
                .await?
                .into_iter()
                .map(|row| row.id)
                .collect();
            if expired.is_empty() {
                batch.rollback().await?;
                break;
            }
            for id in expired {
                cleaned.reporters += batch
                    .update::<report::Model>()
                    .set(report::Model::REPORTER_KEY, format!("expired:{id}"))
                    .filter(report::Model::ID.eq(id))
                    .filter(report::Model::REPORTER_KEY.starts_with(GUEST_REPORTER))
                    .exec()
                    .await?;
            }
            batch.commit().await?;
        }
        let write = all.begin_write().await?;
        self.guard.purge_nonces(&write, MODULE).await?;
        self.tokens.purge(&write, now).await?;
        write.commit().await?;
        Throttle::new(MODULE, 1, Duration::from_secs(3600))?
            .purge(database, now - autoxide_kernel::time::Duration::from_hours(24))
            .await?;
        if cleaned != Cleaned::default() {
            tracing::info!(
                deleted = cleaned.deleted,
                cleared = cleaned.cleared,
                fingerprints = cleaned.fingerprints,
                reporters = cleaned.reporters,
                "comments cleanup"
            );
        }
        Ok(cleaned)
    }

    /// Phần dọn của khách `tenant`: bình luận chưa từng hiện tạo trước `before`, kèm một event.
    async fn expire_tenant(
        &self,
        tenant: &TenantContext,
        before: Timestamp,
        cleaned: &mut Cleaned,
    ) -> Result<(), Error> {
        let mut after = Uuid::nil();
        let mut ids = Vec::new();
        loop {
            let rows = tenant
                .db()
                .read()
                .find::<comment::Model>()
                .filter(comment::Model::STATUS.is_in(EXPIRING.map(str::to_owned)))
                .filter(comment::Model::PUBLISHED_AT.is_null())
                .filter(comment::Model::CREATED_AT.lt(before))
                .filter(comment::Model::ID.gt(after))
                .order_asc(comment::Model::ID)
                .limit(BATCH)
                .all()
                .await?;
            let Some(last) = rows.last() else { break };
            after = last.id;
            for row in rows {
                match self.expire(tenant, &row).await? {
                    Some(Expired::Deleted) => cleaned.deleted += 1,
                    Some(Expired::Cleared) => cleaned.cleared += 1,
                    None => continue,
                }
                ids.push(row.id);
            }
        }
        if !ids.is_empty() {
            let change = Change {
                before: Some(
                    json!({"comment_ids": ids, "count": ids.len(), "reason": "retention"}),
                ),
                after: None,
            };
            let transaction = tenant.db().begin_write().await?;
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::system(),
                    None,
                    &events::CommentsExpired(change),
                )
                .await?;
            transaction.commit().await?;
        }
        Ok(())
    }

    /// Dọn MỘT bình luận hết hạn `row` (đọc ở lô, có thể đã cũ). Chỉ dọn khi nó vẫn đúng như lúc
    /// đọc (cùng trạng thái, cùng `updated_at`, chưa từng hiện): ai đó vừa duyệt / sửa nó thì
    /// thôi (`None`), không xoá nhầm và không đổi số đếm theo trạng thái cũ.
    pub(crate) async fn expire(
        &self,
        tenant: &TenantContext,
        row: &comment::Model,
    ) -> Result<Option<Expired>, Error> {
        let status = Status::of(row);
        if !EXPIRING.contains(&status.as_str()) || row.published_at.is_some() {
            return Ok(None);
        }
        let transaction = tenant.db().begin_write().await?;
        let others = if row.depth == 0 {
            transaction
                .find::<comment::Model>()
                .filter(comment::Model::ROOT_ID.eq(row.id))
                .filter(comment::Model::ID.ne(row.id))
                .count()
                .await?
        } else {
            0
        };
        let done = if others > 0 {
            // `soft_delete` ghi có điều kiện trạng thái + `updated_at` (đổi rồi thì 412).
            match soft_delete(
                &self.tokens,
                Some(&self.outbox),
                &transaction,
                tenant,
                row,
                Deletion::Erased,
            )
            .await
            {
                Ok(_) => Expired::Cleared,
                Err(error) if error.kind() == ErrorKind::PreconditionFailed => {
                    transaction.rollback().await?;
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
        } else {
            // Xoá trước (khoá dòng, như mọi lệnh ghi: bình luận → gốc → đích), có điều kiện.
            let removed = transaction
                .delete::<comment::Model>()
                .filter(comment::Model::ID.eq(row.id))
                .filter(comment::Model::STATUS.eq(status.as_str().to_owned()))
                .filter(comment::Model::UPDATED_AT.eq(row.updated_at))
                .filter(comment::Model::PUBLISHED_AT.is_null())
                .exec()
                .await?;
            if removed == 0 {
                transaction.rollback().await?;
                return Ok(None);
            }
            shift_counts(&transaction, row, Some(status), None).await?;
            transaction
                .delete::<report::Model>()
                .filter(report::Model::COMMENT_ID.eq(row.id))
                .exec()
                .await?;
            transaction
                .delete::<subscription::Model>()
                .filter(subscription::Model::COMMENT_ID.eq(row.id))
                .exec()
                .await?;
            self.tokens.forget(&transaction, tenant, &row.id.to_string()).await?;
            Expired::Deleted
        };
        transaction.commit().await?;
        Ok(Some(done))
    }
}
