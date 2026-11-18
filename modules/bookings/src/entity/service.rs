//! Bảng `bookings_services`.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "bookings_services")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    /// Tên theo ngôn ngữ khác, JSON `{"en": "…"}`.
    pub names: String,
    pub description: String,
    pub duration_minutes: i32,
    pub buffer_before_minutes: i32,
    pub buffer_after_minutes: i32,
    pub price_minor: i64,
    pub currency: String,
    pub deposit_kind: String,
    pub deposit_value: i64,
    pub capacity: i32,
    pub active: bool,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
