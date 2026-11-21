//! Bảng `orders_lookup_tokens`: token xem đơn vãng lai (chỉ lưu bản băm), mở đúng một đơn.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_lookup_tokens")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub order_id: Uuid,
    pub phone_hash: String,
    pub token_hash: String,
    pub expires_at: Timestamp,
    pub otp_confirmed_at: Timestamp,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
