//! Bảng `orders_cart_lines`: một biến thể trong giỏ. Giá không lưu: luôn tính lại ở máy chủ.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_cart_lines")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub cart_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub variant_id: Uuid,
    pub quantity: i64,
    pub added_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
