//! Dữ liệu cá nhân trong module `newsletter`.
//!
//! - Theo email (người đăng ký không có tài khoản): `GET` / `DELETE
//!   /api/v1/newsletter/data?email=` (`audience/subscribers.rs`), permission `newsletter.export`.
//!   Người đăng ký KHÔNG gắn với tài khoản: đăng ký công khai chỉ có email, nên mọi yêu cầu của
//!   chủ dữ liệu đi theo email.
//! - Theo tài khoản: chỉ `created_by` của danh sách và chiến dịch. Xuất: những thứ người đó tạo.
//!   Tài khoản bị xoá (`privacy::USER_DELETED`): bỏ trống `created_by`; danh sách và chiến dịch
//!   vẫn giữ (là của site). Nhận lại cùng event thì kết quả không đổi (D-054).

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{campaign, list};
use crate::service::NewsletterService;

#[async_trait::async_trait]
impl PersonalData for NewsletterService {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        export(tenant, user_id).await
    }
}

#[async_trait::async_trait]
impl EventHandler for NewsletterService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        forget(event).await
    }
}

/// Phần dữ liệu cá nhân của `newsletter` khi module đang tắt (chế độ nghỉ, bước 70 P9): cùng hàm với
/// lúc bật; chỉ cần database của khách, không cần service.
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
        forget(event).await
    }
}

/// Dữ liệu của người `user_id` cho bản xuất dữ liệu cá nhân.
async fn export(tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
    let read = tenant.database().read();
    let lists = list::Entity::find()
        .scoped(tenant)
        .filter(list::Column::CreatedBy.eq(user_id))
        .all(read)
        .await?;
    let campaigns = campaign::Entity::find()
        .scoped(tenant)
        .filter(campaign::Column::CreatedBy.eq(user_id))
        .all(read)
        .await?;
    Ok(json!({
        "lists": lists.iter().map(|row| json!({
            "id": row.id,
            "key": row.key,
            "created_at": row.created_at,
        })).collect::<Vec<_>>(),
        "campaigns": campaigns.iter().map(|row| json!({
            "id": row.id,
            "subject": row.subject,
            "status": row.status,
            "created_at": row.created_at,
        })).collect::<Vec<_>>(),
    }))
}

/// Tài khoản bị xoá (xem đầu file).
async fn forget(event: &Delivered) -> Result<(), Error> {
    let Some(user_id) = deleted_user(event) else {
        return Ok(());
    };
    // Tài khoản bị xoá ở khách của event: chỉ bỏ `created_by` ở dữ liệu của khách đó.
    let tenant = &event.tenant;
    let write = tenant.database().write();
    let lists = list::Entity::update_many()
        .scoped(tenant)
        .col_expr(list::Column::CreatedBy, Expr::value(Option::<Uuid>::None))
        .filter(list::Column::CreatedBy.eq(user_id))
        .exec(write)
        .await?
        .rows_affected;
    let campaigns = campaign::Entity::update_many()
        .scoped(tenant)
        .col_expr(campaign::Column::CreatedBy, Expr::value(Option::<Uuid>::None))
        .filter(campaign::Column::CreatedBy.eq(user_id))
        .exec(write)
        .await?
        .rows_affected;
    tracing::info!(user_id = %user_id, lists, campaigns, "newsletter items of a deleted user unlinked");
    Ok(())
}
