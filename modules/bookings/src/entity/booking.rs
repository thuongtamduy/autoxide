//! Bảng `bookings_bookings`.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "bookings_bookings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub service_id: Uuid,
    pub resource_id: Uuid,
    pub session_id: Option<Uuid>,
    pub seats: i32,
    pub starts_at: DateTimeUtc,
    pub ends_at: DateTimeUtc,
    pub status: String,
    pub customer_user_id: Option<Uuid>,
    pub customer_name: String,
    pub customer_phone: Option<String>,
    pub customer_email: Option<String>,
    pub note: String,
    pub locale: Option<String>,
    pub price_minor: i64,
    pub currency: String,
    pub deposit_minor: i64,
    pub deposit_status: String,
    pub refund_minor: i64,
    pub payment_id: Option<Uuid>,
    pub hold_expires_at: Option<DateTimeUtc>,
    pub reminded_24h_at: Option<DateTimeUtc>,
    pub reminded_2h_at: Option<DateTimeUtc>,
    pub cancelled_at: Option<DateTimeUtc>,
    pub cancel_reason: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
