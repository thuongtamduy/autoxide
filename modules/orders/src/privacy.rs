//! Đơn hàng trong bản xuất dữ liệu cá nhân (Luật 91/2025/QH15 — cần đối chiếu văn bản gốc; rà soát bảo
//! mật 2026-11): người mua xem được các đơn đặt bằng tài khoản của mình, kèm thông tin liên hệ, địa chỉ
//! nhận hàng và ghi chú đã khai. Xoá tài khoản không xoá đơn: đơn là chứng từ kế toán phải giữ; tên và
//! số trong hồ sơ khách do `customers` xoá.

use autoxide_kernel::error::Error;
use autoxide_kernel::privacy::PersonalData;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::order;

/// Phần dữ liệu cá nhân của `orders`.
pub(crate) struct OrderPrivacy;

/// Cột JSON đọc lại thành giá trị (hỏng thì giữ chuỗi gốc: bản xuất không được thiếu dữ liệu).
fn json_of(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::from(raw))
}

#[async_trait::async_trait]
impl PersonalData for OrderPrivacy {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        let rows = order::Entity::find()
            .filter(order::Column::BuyerUserId.eq(user_id))
            .scoped(tenant)
            .order_by_asc(order::Column::Id)
            .all(tenant.database().read())
            .await?;
        let orders: Vec<Value> = rows
            .iter()
            .map(|row| {
                json!({
                    "id": row.id,
                    "number": row.number,
                    "placed_at": row.placed_at,
                    "status": row.status,
                    "currency": row.currency,
                    "total_minor": row.total_minor,
                    "contact": json_of(&row.contact),
                    "ship_to": row.ship_to.as_deref().map(json_of),
                    "note": row.note,
                })
            })
            .collect();
        Ok(json!({ "orders": orders }))
    }
}
