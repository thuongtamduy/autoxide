//! Dữ liệu cá nhân trong sổ địa chỉ (Luật 91/2025/QH15, NĐ 356/2025): bản xuất cho người dùng, và
//! xoá khi tài khoản bị xoá. Mục đích duy nhất là giao hàng. Vận đơn đã tạo giữ bản chụp người nhận
//! theo luật kế toán / bưu chính, không thuộc sổ địa chỉ. Nhận lại event cũng không sao: lần sau
//! không còn gì để xoá.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::TenantContext;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::service::ShippingService;

#[async_trait::async_trait]
impl PersonalData for Erasure {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        let addresses = ShippingService::all_addresses(tenant, user_id).await?;
        Ok(json!({
            "addresses": addresses.iter().map(|entry| json!({
                "label": entry.label,
                "recipient_name": entry.recipient_name,
                "phone": entry.phone,
                "address": entry.address,
                "postal_code": entry.postal_code,
                "is_default": entry.is_default,
                "updated_at": entry.updated_at,
            })).collect::<Vec<_>>(),
        }))
    }
}

/// Bản xuất sổ địa chỉ, và bên nghe `privacy.user_deleted`: xoá sổ địa chỉ của tài khoản. Chỉ cần database
/// của khách, nên dùng được cả khi module đang tắt (chế độ "nghỉ", bước 70 P9).
pub(crate) struct Erasure;

#[async_trait::async_trait]
impl EventHandler for Erasure {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        let removed = ShippingService::erase_addresses(&event.tenant, user_id).await?;
        tracing::info!(user_id = %user_id, removed, "shipping addresses of a deleted user erased");
        Ok(())
    }
}
