//! Bảng `orders_cod_blocks`: người mua không được trả khi nhận hàng, có lý do và hạn.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_cod_blocks")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub buyer_key: String,
    pub reason_code: String,
    pub note: Option<String>,
    pub expires_at: DateTimeUtc,
    pub created_by: Uuid,
    pub created_at: DateTimeUtc,
    pub lifted_by: Option<Uuid>,
    pub lifted_at: Option<DateTimeUtc>,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
