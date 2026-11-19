//! Bảng `orders_line_taxes`: một loại thuế của một dòng đơn.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_line_taxes")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub line_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub tax_code: String,
    pub rate: String,
    pub exempt: bool,
    pub base_minor: i64,
    pub amount_minor: i64,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
