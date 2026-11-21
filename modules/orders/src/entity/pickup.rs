//! Bảng `orders_pickups`: đơn nhận tại cửa hàng đã sẵn sàng.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_pickups")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub order_id: Uuid,
    pub pickup_org_id: Uuid,
    pub status: String,
    pub code: String,
    pub ready_at: Timestamp,
    pub hold_until: Timestamp,
    pub reminded_at: Option<Timestamp>,
    pub handed_over_at: Option<Timestamp>,
    pub handed_over_to: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
