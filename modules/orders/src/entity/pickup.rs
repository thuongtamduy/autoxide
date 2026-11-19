//! Bảng `orders_pickups`: đơn nhận tại cửa hàng đã sẵn sàng.

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
    pub ready_at: DateTimeUtc,
    pub hold_until: DateTimeUtc,
    pub reminded_at: Option<DateTimeUtc>,
    pub handed_over_at: Option<DateTimeUtc>,
    pub handed_over_to: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
