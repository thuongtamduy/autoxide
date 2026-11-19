//! Bảng `orders_orders`: một đơn hàng.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_orders")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub checkout_id: Option<Uuid>,
    pub number: String,
    pub channel: String,
    pub external_ref: Option<String>,
    pub buyer_user_id: Option<Uuid>,
    pub buyer_key: Option<String>,
    pub contact: String,
    pub ship_to: Option<String>,
    pub shipping_option: Option<String>,
    pub pickup_org_id: Option<Uuid>,
    pub status: String,
    pub payment_status: String,
    pub fulfillment_status: String,
    pub payment_method: String,
    pub funds_flow: String,
    pub currency: String,
    pub subtotal_minor: i64,
    pub discount_minor: i64,
    pub shipping_fee_minor: i64,
    pub shipping_discount_minor: i64,
    pub tax_minor: i64,
    pub total_minor: i64,
    pub paid_minor: i64,
    pub refunded_minor: i64,
    pub payment_due_at: Option<DateTimeUtc>,
    pub attribution: Option<String>,
    pub promotions: Option<String>,
    pub stock_shortage: bool,
    pub invoice_request: Option<String>,
    pub terms_version: Option<String>,
    pub note: Option<String>,
    pub locale: String,
    pub placed_at: DateTimeUtc,
    pub confirmed_at: Option<DateTimeUtc>,
    pub delivered_at: Option<DateTimeUtc>,
    pub completed_at: Option<DateTimeUtc>,
    pub cancelled_at: Option<DateTimeUtc>,
    pub cancel_reason: Option<String>,
    pub version: i32,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
