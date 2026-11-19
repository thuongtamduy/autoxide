//! Bảng `orders_payments`: một khoản thanh toán của đơn.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_payments")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub order_id: Uuid,
    pub purpose: String,
    pub method: String,
    pub gateway: Option<String>,
    pub ledger_payment_id: Option<Uuid>,
    pub external_id: Option<String>,
    pub reference: String,
    pub amount_minor: i64,
    pub currency: String,
    pub tendered_minor: Option<i64>,
    pub status: String,
    pub action: Option<String>,
    pub expires_at: Option<DateTimeUtc>,
    pub succeeded_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
