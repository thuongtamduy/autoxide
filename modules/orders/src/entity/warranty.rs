//! Bảng `orders_warranties`: bảo hành của một dòng đơn (hay một serial), có hạn.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_warranties")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub order_id: Uuid,
    pub order_line_id: Uuid,
    pub product_id: Uuid,
    pub variant_id: Uuid,
    pub serial: Option<String>,
    pub serial_key: String,
    pub buyer_user_id: Option<Uuid>,
    pub buyer_key: Option<String>,
    pub starts_at: Timestamp,
    pub ends_at: Timestamp,
    pub status: String,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
