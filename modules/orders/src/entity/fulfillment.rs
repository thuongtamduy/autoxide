//! Bảng `orders_fulfillments`: một lần giao của đơn.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_fulfillments")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub order_id: Uuid,
    pub request_key: String,
    pub lines: String,
    pub status: String,
    pub cod_collected_minor: Option<i64>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
