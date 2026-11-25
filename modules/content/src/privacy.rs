//! Dữ liệu cá nhân trong module `content`.
//!
//! Tài khoản bị xoá (`privacy::USER_DELETED`): xoá hồ sơ tác giả gắn với người đó (cùng liên
//! kết của hồ sơ với các mục), bỏ trống `created_by` ở mọi bảng, cùng `review_requested_by` (ai
//! xin duyệt) và `ai_reviewed_by` (ai xác nhận đã duyệt nội dung AI — thời điểm
//! `ai_reviewed_at` vẫn giữ, nên người đọc vẫn thấy `ai_reviewed`), huỷ lịch người đó đặt. Bài và
//! trang vẫn giữ: nội dung là của site, không của người viết. Nhận lại cùng event thì kết quả
//! không đổi (D-054).
//!
//! Module đang tắt (chế độ nghỉ, bước 70 P9) vẫn xuất và gỡ như trên qua [`DormantPrivacy`]: cùng
//! hàm, chỉ không ghi event (không ai phát event của module tắt — audit / webhook của lần huỷ lịch,
//! xoá hồ sơ tác giả lúc đó bị bỏ qua, dữ liệu vẫn được gỡ đủ).

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Outbox};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{author, entry, published, redirect, revision, schedule};
use crate::service::ContentService;

#[async_trait::async_trait]
impl PersonalData for ContentService {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        export(tenant, user_id).await
    }
}

#[async_trait::async_trait]
impl EventHandler for ContentService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        forget(event, Some(&self.outbox)).await
    }
}

/// Phần dữ liệu cá nhân của `content` khi module đang tắt (chế độ nghỉ, bước 70 P9): không cần
/// service, chỉ database của khách; không ghi event.
pub(crate) struct DormantPrivacy;

#[async_trait::async_trait]
impl PersonalData for DormantPrivacy {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        export(tenant, user_id).await
    }
}

#[async_trait::async_trait]
impl EventHandler for DormantPrivacy {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        forget(event, None).await
    }
}

/// Hồ sơ tác giả và các mục người `user_id` tạo.
async fn export(tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
    let read = tenant.database().read();
    let authors = author::Entity::find()
        .scoped(tenant)
        .filter(author::Column::UserId.eq(user_id))
        .order_by_asc(author::Column::Id)
        .all(read)
        .await?;
    let entries = entry::Entity::find()
        .scoped(tenant)
        .filter(entry::Column::CreatedBy.eq(user_id))
        .order_by_asc(entry::Column::Id)
        .all(read)
        .await?;
    Ok(json!({
        "authors": authors.iter().map(|author| json!({
            "id": author.id,
            "display_name": author.display_name,
            "slug": author.slug,
            "bio": author.bio,
            "website_url": author.website_url,
            "created_at": author.created_at,
        })).collect::<Vec<_>>(),
        "entries": entries.iter().map(|entry| json!({
            "id": entry.id,
            "kind": entry.kind,
            "locale": entry.locale,
            "title": entry.title,
            "status": entry.status,
            "created_at": entry.created_at,
        })).collect::<Vec<_>>(),
    }))
}

/// Tài khoản bị xoá (xem đầu file); event ghi qua `outbox` nếu có.
async fn forget(event: &Delivered, outbox: Option<&Outbox>) -> Result<(), Error> {
    let Some(user_id) = deleted_user(event) else {
        return Ok(());
    };
    // Tài khoản thuộc khách của event (bước 33): chỉ gỡ dữ liệu của khách đó.
    let tenant = &event.tenant;
    // Huỷ lịch trước khi bỏ `created_by`: sau đó không còn biết lịch nào của người này.
    let canceled =
        crate::publishing::schedules::cancel_schedules_by(tenant, outbox, user_id).await?;
    // Yêu cầu dịch đang chờ của người này: huỷ (job chạy tới sẽ thấy đã xong và thôi).
    let translations = crate::ai::translate::cancel_translations_by(tenant, user_id).await?;
    let profiles = author::Entity::find()
        .scoped(tenant)
        .filter(author::Column::UserId.eq(user_id))
        .all(tenant.database().read())
        .await?;
    for profile in &profiles {
        crate::taxonomy::authors::remove_author_with(tenant, outbox, profile.id, Actor::system())
            .await?;
    }
    // Mỗi lệnh tự đủ (không cần chung transaction): chạy lại giữa chừng vẫn ra cùng kết quả.
    let write = tenant.database().write();
    let none = || Expr::value(Option::<Uuid>::None);
    entry::Entity::update_many()
        .scoped(tenant)
        .col_expr(entry::Column::CreatedBy, none())
        .filter(entry::Column::CreatedBy.eq(user_id))
        .exec(write)
        .await?;
    entry::Entity::update_many()
        .scoped(tenant)
        .col_expr(entry::Column::ReviewRequestedBy, none())
        .filter(entry::Column::ReviewRequestedBy.eq(user_id))
        .exec(write)
        .await?;
    published::Entity::update_many()
        .scoped(tenant)
        .col_expr(published::Column::AiReviewedBy, none())
        .filter(published::Column::AiReviewedBy.eq(user_id))
        .exec(write)
        .await?;
    revision::Entity::update_many()
        .scoped(tenant)
        .col_expr(revision::Column::CreatedBy, none())
        .filter(revision::Column::CreatedBy.eq(user_id))
        .exec(write)
        .await?;
    schedule::Entity::update_many()
        .scoped(tenant)
        .col_expr(schedule::Column::CreatedBy, none())
        .filter(schedule::Column::CreatedBy.eq(user_id))
        .exec(write)
        .await?;
    redirect::Entity::update_many()
        .scoped(tenant)
        .col_expr(redirect::Column::CreatedBy, none())
        .filter(redirect::Column::CreatedBy.eq(user_id))
        .exec(write)
        .await?;
    crate::entity::translation_request::Entity::update_many()
        .scoped(tenant)
        .col_expr(crate::entity::translation_request::Column::CreatedBy, none())
        .filter(crate::entity::translation_request::Column::CreatedBy.eq(user_id))
        .exec(write)
        .await?;
    tracing::info!(user_id = %user_id, authors = profiles.len(), schedules = canceled, translations, "content of a deleted user unlinked");
    Ok(())
}
