//! Dữ liệu cá nhân trong sổ địa chỉ (Luật 91/2025/QH15, NĐ 356/2025): bản xuất cho người dùng, và
//! xoá khi tài khoản bị xoá. Mục đích duy nhất là giao hàng. Vận đơn đã tạo giữ bản chụp người nhận
//! theo luật kế toán / bưu chính, không thuộc sổ địa chỉ. Nhận lại event cũng không sao: lần sau
//! không còn gì để xoá.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::TenantContext;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::service::ShippingService;

#[async_trait::async_trait]
impl PersonalData for ShippingService {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        let addresses = self.all_addresses(tenant, user_id).await?;
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

/// Nghe `privacy.user_deleted`: xoá sổ địa chỉ của tài khoản.
pub(crate) struct Erasure(pub Arc<ShippingService>);

#[async_trait::async_trait]
impl EventHandler for Erasure {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        let removed = self.0.erase_addresses(&event.tenant, user_id).await?;
        tracing::info!(user_id = %user_id, removed, "shipping addresses of a deleted user erased");
        Ok(())
    }
}
