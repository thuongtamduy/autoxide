//! Bảng `orders_line_serials`: số serial của dòng đơn ghi khi chuẩn bị hàng (mở bảo hành theo serial).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_line_serials")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    pub order_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub line_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub serial: String,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
