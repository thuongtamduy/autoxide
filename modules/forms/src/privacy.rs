//! Dữ liệu cá nhân trong module `forms`.
//!
//! - Theo tài khoản: form chỉ giữ `created_by` của người tạo. Xuất: các form người đó tạo. Tài
//!   khoản bị xoá (`privacy::USER_DELETED`): bỏ trống `created_by`; form vẫn giữ (là của site).
//!   Nhận lại cùng event thì kết quả không đổi (D-054).
//! - Theo email (người gửi form không có tài khoản): `GET` / `DELETE /api/v1/forms/data?email=`
//!   (`submissions/inbox.rs`), permission `forms.export`.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::TenantContext;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::service::FormsService;
use crate::submissions::inbox::{forget_creator, forms_created_by};

#[async_trait::async_trait]
impl PersonalData for FormsService {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        export(tenant, user_id).await
    }
}

#[async_trait::async_trait]
impl EventHandler for FormsService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        forget(event).await
    }
}

/// Phần dữ liệu cá nhân của `forms` khi module đang tắt (chế độ nghỉ, bước 70 P9): cùng hàm với
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

/// Các form người `user_id` tạo.
async fn export(tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
    let forms = forms_created_by(tenant, user_id).await?;
    Ok(json!({
        "forms": forms.iter().map(|form| json!({
            "id": form.row.id,
            "key": form.row.key,
            "name": form.row.name,
            "created_at": form.row.created_at,
        })).collect::<Vec<_>>(),
    }))
}

/// Tài khoản bị xoá: bỏ `created_by` (xem đầu file).
async fn forget(event: &Delivered) -> Result<(), Error> {
    let Some(user_id) = deleted_user(event) else {
        return Ok(());
    };
    let forms = forget_creator(&event.tenant, user_id).await?;
    tracing::info!(user_id = %user_id, forms, "forms of a deleted user unlinked");
    Ok(())
}
