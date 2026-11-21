//! Bảng `orders_status_history`: một lần đổi trạng thái của đơn (chỉ thêm).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_status_history")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub order_id: Uuid,
    pub from_status: Option<String>,
    pub to_status: String,
    pub cause: String,
    pub actor_id: Option<Uuid>,
    pub reason: Option<String>,
    pub at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
