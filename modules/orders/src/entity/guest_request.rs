//! Bảng `orders_guest_requests`: thao tác vãng lai đã làm theo `Idempotency-Key` (gọi lại không làm
//! hai lần; lớp idempotency HTTP của kernel bỏ qua request chưa đăng nhập).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_guest_requests")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub order_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub action: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub reference: String,
    pub fingerprint: String,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
