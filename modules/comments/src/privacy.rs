//! Dữ liệu cá nhân trong module `comments`.
//!
//! - Theo tài khoản: bình luận có `user_id`, tên hiển thị (`comments_profiles`), báo cáo đã gửi.
//!   Xuất qua `PersonalData`. Tài khoản bị xoá (`privacy::USER_DELETED`): mọi bình luận của người
//!   đó bị xoá mềm (giữ chỗ trong luồng), `user_id` bỏ trống, hồ sơ bị xoá, báo cáo bỏ
//!   `reporter_user_id`. Nhận lại cùng event thì không còn gì để làm (D-054).
//! - Theo email (khách, hay email nhận mail báo): `GET` / `DELETE /api/v1/comments/data?email=`
//!   (`moderation.rs`), permission `comments.export`.
//!
//! Module đang tắt (chế độ nghỉ, bước 70 P9) vẫn xuất và xoá theo tài khoản như trên qua
//! [`DormantPrivacy`]: cùng hàm, chỉ không ghi event `comments.comments_erased` (không ai phát event
//! của module tắt).

use autoxide_kernel::db::Queries as _;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Outbox};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::tokens::OneTimeTokens;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{comment, profile, report};
use crate::events::{self, Change};
use crate::posting::authoring::{Deletion, soft_delete};
use crate::service::{CommentsService, Status};
use crate::text;

#[autoxide_kernel::async_trait]
impl PersonalData for CommentsService {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        export(tenant, user_id).await
    }
}

#[autoxide_kernel::async_trait]
impl EventHandler for CommentsService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        handle_deleted(event, &self.tokens, Some(&self.outbox)).await
    }
}

/// Phần dữ liệu cá nhân của `comments` khi module đang tắt (chế độ nghỉ, bước 70 P9): cùng hàm với
/// lúc bật, không cần service; không ghi event.
pub(crate) struct DormantPrivacy {
    pub(crate) tokens: OneTimeTokens,
}

impl DormantPrivacy {
    pub(crate) fn new() -> Self {
        DormantPrivacy { tokens: OneTimeTokens::new(crate::service::MODULE) }
    }
}

#[autoxide_kernel::async_trait]
impl PersonalData for DormantPrivacy {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        export(tenant, user_id).await
    }
}

#[autoxide_kernel::async_trait]
impl EventHandler for DormantPrivacy {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        handle_deleted(event, &self.tokens, None).await
    }
}

async fn handle_deleted(
    event: &Delivered,
    tokens: &OneTimeTokens,
    outbox: Option<&Outbox>,
) -> Result<(), Error> {
    let Some(user_id) = deleted_user(event) else {
        return Ok(());
    };
    let count = forget_user(&event.tenant, tokens, outbox, user_id).await?;
    tracing::info!(user_id = %user_id, count, "comments of a deleted user cleared");
    Ok(())
}

/// Bình luận, hồ sơ và báo cáo của tài khoản `user_id`.
async fn export(tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
    let mut rows = tenant
        .db()
        .read()
        .find::<comment::Model>()
        .filter(comment::Model::USER_ID.eq(Some(user_id)))
        .all()
        .await?;
    rows.sort_by_key(|row| row.id);
    let profile = tenant
        .db()
        .read()
        .find::<profile::Model>()
        .filter(profile::Model::USER_ID.eq(user_id))
        .one()
        .await?;
    let reports = tenant
        .db()
        .read()
        .find::<report::Model>()
        .filter(report::Model::REPORTER_USER_ID.eq(Some(user_id)))
        .all()
        .await?;
    Ok(json!({
        "profile": profile.map(|row| json!({"display_name": row.display_name, "updated_at": row.updated_at})),
        "comments": rows.iter().map(|row| json!({
            "id": row.id,
            "target_type": row.target_type,
            "target_id": row.target_id,
            "reply_to_id": row.reply_to_id,
            "status": row.status,
            "body": row.body,
            "body_html": text::to_html(&row.body, text::Format::from_column(&row.format)),
            "format": row.format,
            "email": row.author_email,
            "locale": row.locale,
            "edited_at": row.edited_at,
            "created_at": row.created_at,
        })).collect::<Vec<_>>(),
        "reports": reports.iter().map(|row| json!({
            "comment_id": row.comment_id,
            "reason": row.reason,
            "note": row.note,
            "created_at": row.created_at,
        })).collect::<Vec<_>>(),
    }))
}

/// Xoá dữ liệu của tài khoản `user_id` đã bị xoá. Trả số bình luận vừa xoá mềm. Event ghi qua
/// `outbox` nếu có (không có khi module đang tắt).
async fn forget_user(
    tenant: &TenantContext,
    tokens: &OneTimeTokens,
    outbox: Option<&Outbox>,
    user_id: Uuid,
) -> Result<usize, Error> {
    let transaction = tenant.db().begin_write().await?;
    let rows = transaction
        .find::<comment::Model>()
        .filter(comment::Model::USER_ID.eq(Some(user_id)))
        .all()
        .await?;
    let mut ids = Vec::new();
    for row in &rows {
        if Status::of(row) == Status::Deleted {
            continue;
        }
        soft_delete(tokens, None, &transaction, tenant, row, Deletion::Erased).await?;
        ids.push(row.id);
    }
    // Bình luận đã xoá từ trước vẫn còn `user_id`.
    transaction
        .update::<comment::Model>()
        .set(comment::Model::USER_ID, None)
        .filter(comment::Model::USER_ID.eq(Some(user_id)))
        .exec()
        .await?;
    transaction
        .delete::<profile::Model>()
        .filter(profile::Model::USER_ID.eq(user_id))
        .exec()
        .await?;
    // Báo cáo đã gửi giữ lại (người kiểm duyệt cần), nhưng không còn chỉ tới người đó.
    let reports = transaction
        .find::<report::Model>()
        .filter(report::Model::REPORTER_USER_ID.eq(Some(user_id)))
        .all()
        .await?;
    for found in reports {
        transaction
            .update::<report::Model>()
            .set(report::Model::REPORTER_USER_ID, None)
            .set(report::Model::REPORTER_KEY, format!("deleted:{}", found.id))
            .filter(report::Model::ID.eq(found.id))
            .exec()
            .await?;
    }
    if !ids.is_empty()
        && let Some(outbox) = outbox
    {
        let change = Change {
            before: Some(json!({"comment_ids": ids, "count": ids.len(), "reason": "user_deleted"})),
            after: None,
        };
        outbox
            .record(&transaction, tenant, Actor::system(), None, &events::CommentsErased(change))
            .await?;
    }
    transaction.commit().await?;
    Ok(ids.len())
}
