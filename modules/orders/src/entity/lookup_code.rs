//! Bảng `orders_lookup_codes`: mã một lần gửi tới số điện thoại của đơn vãng lai (chỉ lưu bản băm).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_lookup_codes")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub order_id: Uuid,
    pub purpose: String,
    pub token_id: Option<Uuid>,
    pub code_hash: String,
    pub attempts: i32,
    pub expires_at: Timestamp,
    pub consumed_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
