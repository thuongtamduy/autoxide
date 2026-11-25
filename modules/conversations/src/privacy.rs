//! Dữ liệu cá nhân trong tin nhắn (Luật 91/2025/QH15; NĐ 356/2025 — cần đối chiếu văn bản gốc):
//!
//! - bản xuất cho người dùng: các cuộc trò chuyện của người đó và tin người đó viết;
//! - xoá tài khoản (`privacy::USER_DELETED`): ẩn danh hoá phía người mua — bỏ nội dung và ảnh của tin
//!   người đó viết, bỏ liên kết cuộc trò chuyện với tài khoản, đóng cuộc đang mở; xoá chặn, con trỏ
//!   đọc; báo vi phạm giữ lại nhưng bỏ người báo. Nhận lại event cũng không sao: lần sau không còn gì
//!   để đổi;
//! - hạn lưu: job `conversations.purge` mỗi ngày xoá cuộc trò chuyện có tin cuối quá
//!   `AUTOXIDE_CONVERSATIONS_RETENTION_DAYS` ngày, cùng tin, con trỏ đọc và báo vi phạm của nó.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::throttle::Throttle;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{block, message, read, report, thread};
use crate::service::{AuthorKind, CLOSED, ConversationsService, MODULE, now};
use crate::voice::recordings::RecordingEraser;

pub(crate) const PURGE: &str = "conversations.purge";

/// Số cuộc trò chuyện xoá mỗi lượt (giữ transaction ngắn).
const PURGE_BATCH: u64 = 200;

#[async_trait::async_trait]
impl PersonalData for ConversationsService {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        export(tenant, user_id).await
    }
}

#[async_trait::async_trait]
impl EventHandler for ConversationsService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        forget(&RecordingEraser::Service(self), &event.tenant, user_id).await
    }
}

/// Phần dữ liệu cá nhân của `conversations` khi module đang tắt (chế độ nghỉ, bước 70 P9): cùng hàm
/// với lúc bật, không cần service. Bản ghi âm xoá bằng kho dùng chung và bộ nối thoại đang bật; thiếu thì
/// báo lỗi để event được thử lại (không báo xong khi bản ghi còn nằm lại).
pub(crate) struct DormantPrivacy {
    /// Kho dùng chung (của `files` đang bật, hay `files` "nghỉ" tự mở) — đọc lúc xử lý.
    pub tenancy: autoxide_kernel::tenancy::DatabaseRegistry,
    /// Bộ nối thoại của module đang bật, để xoá bản ghi ở nhà cung cấp.
    pub voice: Vec<Arc<dyn autoxide_kernel::voice::VoiceProvider>>,
}

#[async_trait::async_trait]
impl PersonalData for DormantPrivacy {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        export(tenant, user_id).await
    }
}

#[async_trait::async_trait]
impl EventHandler for DormantPrivacy {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        let eraser =
            RecordingEraser::Dormant { storage: self.tenancy.shared_storage(), voice: &self.voice };
        forget(&eraser, &event.tenant, user_id).await
    }
}

/// Các cuộc trò chuyện của người `user_id` và tin người đó viết.
async fn export(tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
    let read = tenant.database().read();
    let threads = thread::Entity::find()
        .filter(thread::Column::CustomerUserId.eq(user_id))
        .scoped(tenant)
        .order_by_asc(thread::Column::Id)
        .all(read)
        .await?;
    let mut out = Vec::new();
    for thread in threads {
        let messages = message::Entity::find()
            .filter(message::Column::ThreadId.eq(thread.id))
            .filter(message::Column::AuthorUserId.eq(user_id))
            .scoped(tenant)
            .order_by_asc(message::Column::Id)
            .all(read)
            .await?;
        out.push(json!({
            "id": thread.id,
            "org_id": thread.org_id,
            "subject": thread.subject,
            "status": thread.status,
            "created_at": thread.created_at,
            "messages": messages.iter().map(|message| json!({
                "body": message.body,
                "media_ids": crate::service::media_of(message),
                "recalled": message.recalled_at.is_some(),
                "created_at": message.created_at,
            })).collect::<Vec<_>>(),
        }));
    }
    Ok(Value::Array(out))
}

/// Ẩn danh hoá người mua `user_id` (xem đầu file); `eraser` xoá bản ghi âm.
/// Như [`forget`], cho test của phần "nghỉ" (truyền kho thẳng vào).
#[cfg(test)]
pub(crate) async fn forget_for_test(
    eraser: &RecordingEraser<'_>,
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<(), Error> {
    forget(eraser, tenant, user_id).await
}

async fn forget(
    eraser: &RecordingEraser<'_>,
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<(), Error> {
    let threads: Vec<Uuid> = thread::Entity::find()
        .select_only()
        .column(thread::Column::Id)
        .filter(thread::Column::CustomerUserId.eq(user_id))
        .scoped(tenant)
        .into_tuple()
        .all(tenant.database().read())
        .await?;
    // Bản ghi âm và số trong yêu cầu gọi lại (gọi nơi lưu / nhà cung cấp: ngoài transaction).
    crate::voice::recordings::forget_calls(eraser, tenant, user_id, &threads).await?;
    let transaction = tenant.database().begin_write().await?;
    let ids: Vec<Uuid> = thread::Entity::find()
        .select_only()
        .column(thread::Column::Id)
        .filter(thread::Column::CustomerUserId.eq(user_id))
        .scoped(tenant)
        .into_tuple()
        .all(&transaction)
        .await?;
    if !ids.is_empty() {
        message::Entity::update_many()
            .col_expr(message::Column::Body, Expr::value(""))
            .col_expr(message::Column::MediaIds, Expr::value("[]"))
            .col_expr(message::Column::AuthorUserId, Expr::value(Option::<Uuid>::None))
            .filter(message::Column::ThreadId.is_in(ids.clone()))
            .filter(message::Column::AuthorKind.eq(AuthorKind::Customer.as_str()))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let at = now();
        thread::Entity::update_many()
            .col_expr(thread::Column::CustomerUserId, Expr::value(Option::<Uuid>::None))
            .col_expr(thread::Column::OpenKey, Expr::value(Option::<String>::None))
            .col_expr(thread::Column::Status, Expr::value(CLOSED))
            .col_expr(thread::Column::ClosedAt, Expr::value(Some(at)))
            .col_expr(thread::Column::DraftReply, Expr::value(Option::<String>::None))
            .col_expr(thread::Column::UpdatedAt, Expr::value(at))
            .filter(thread::Column::Id.is_in(ids.clone()))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
    }
    block::Entity::delete_many()
        .filter(block::Column::UserId.eq(user_id))
        .scoped(tenant)
        .exec(&transaction)
        .await?;
    read::Entity::delete_many()
        .filter(read::Column::UserId.eq(user_id))
        .scoped(tenant)
        .exec(&transaction)
        .await?;
    report::Entity::update_many()
        .col_expr(report::Column::ReporterUserId, Expr::value(Option::<Uuid>::None))
        .filter(report::Column::ReporterUserId.eq(user_id))
        .scoped(tenant)
        .exec(&transaction)
        .await?;
    transaction.commit().await?;
    tracing::info!(user_id = %user_id, threads = ids.len(), "conversations of a deleted user anonymized");
    Ok(())
}

impl ConversationsService {
    /// Xoá cuộc trò chuyện có tin cuối trước hạn lưu (tính từ `now`) trên `database`, theo lô; trả
    /// số cuộc đã xoá. Hạn lưu là của cả bản cài, nên quét cả database.
    pub(crate) async fn purge(&self, database: &Database, now: Timestamp) -> Result<u64, Error> {
        let before = now - SignedDuration::from_hours(24 * (i64::from(self.config.retention_days)));
        let mut total = 0;
        loop {
            let expired: Vec<(Uuid, Uuid)> = thread::Entity::find()
                .select_only()
                .column(thread::Column::TenantId)
                .column(thread::Column::Id)
                .filter(thread::Column::LastMessageAt.lt(before))
                .limit(PURGE_BATCH)
                .into_tuple()
                .all(database.read())
                .await?;
            if expired.is_empty() {
                break;
            }
            let ids: Vec<Uuid> = expired.iter().map(|(_, id)| *id).collect();
            let transaction = database.begin_write().await?;
            // Xoá cuộc trò chuyện trước, nhắc lại điều kiện hạn lưu: tin mới tới giữa lúc quét thì cuộc
            // đó còn lại — và chỉ xoá dòng con của cuộc đã thật sự bị xoá.
            // tenant-scope: job hạn lưu, xoá cuộc trò chuyện quá hạn của mọi khách.
            let removed = thread::Entity::delete_many()
                .filter(thread::Column::Id.is_in(ids.clone()))
                .filter(thread::Column::LastMessageAt.lt(before))
                .exec(&transaction)
                .await?
                .rows_affected;
            let survivors = sea_orm::sea_query::Query::select()
                .column(thread::Column::Id)
                .from(thread::Entity)
                .and_where(thread::Column::Id.is_in(ids.clone()))
                .to_owned();
            // tenant-scope: dòng con của cuộc vừa xoá (job hạn lưu, mọi khách).
            message::Entity::delete_many()
                .filter(message::Column::ThreadId.is_in(ids.clone()))
                .filter(message::Column::ThreadId.not_in_subquery(survivors.clone()))
                .exec(&transaction)
                .await?;
            // tenant-scope: dòng con của cuộc vừa xoá (job hạn lưu, mọi khách).
            read::Entity::delete_many()
                .filter(read::Column::ThreadId.is_in(ids.clone()))
                .filter(read::Column::ThreadId.not_in_subquery(survivors.clone()))
                .exec(&transaction)
                .await?;
            // tenant-scope: dòng con của cuộc vừa xoá (job hạn lưu, mọi khách).
            report::Entity::delete_many()
                .filter(report::Column::ThreadId.is_in(ids))
                .filter(report::Column::ThreadId.not_in_subquery(survivors))
                .exec(&transaction)
                .await?;
            transaction.commit().await?;
            total += removed;
            if (expired.len() as u64) < PURGE_BATCH {
                break;
            }
        }
        Throttle::new(MODULE, 1, std::time::Duration::from_secs(60))?
            .purge(database, now - SignedDuration::from_hours(1))
            .await?;
        if total > 0 {
            tracing::info!(threads = total, "expired conversations deleted");
        }
        Ok(total)
    }
}

pub(crate) struct Purge(pub Arc<ConversationsService>);

#[async_trait::async_trait]
impl JobHandler for Purge {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.purge(&job.database, now()).await.map(|_| ())
    }
}
