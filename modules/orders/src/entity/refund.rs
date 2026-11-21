//! Bảng `orders_refunds`: một lượt hoàn tiền của đơn.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_refunds")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub order_id: Uuid,
    pub return_id: Option<Uuid>,
    pub payment_id: Uuid,
    pub reference: String,
    pub reference_fingerprint: String,
    pub amount_minor: i64,
    pub currency: String,
    pub status: String,
    pub ledger_refund_id: Option<Uuid>,
    pub reason: String,
    pub created_by: Option<Uuid>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
