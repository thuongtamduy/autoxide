//! Bảng `orders_checkouts`: một lượt đặt hàng (chống trùng theo `Idempotency-Key`, giữ chỗ).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_checkouts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub owner_key: String,
    pub reference: String,
    pub fingerprint: String,
    pub cart_id: Uuid,
    pub state: String,
    pub failure_code: Option<String>,
    pub holds: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
