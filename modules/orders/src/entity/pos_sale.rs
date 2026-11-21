//! Bảng `orders_pos_sales`: lượt bán tại quầy theo `reference` của máy quầy (chống trùng).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_pos_sales")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub shift_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub reference: String,
    pub fingerprint: String,
    pub order_id: Uuid,
    pub cash_minor: i64,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
