//! Bảng `orders_returns`: một yêu cầu trả hàng.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_returns")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub order_id: Uuid,
    pub org_id: Uuid,
    pub buyer_user_id: Option<Uuid>,
    pub number: String,
    pub status: String,
    pub reason_code: String,
    pub buyer_note: Option<String>,
    pub resolution: String,
    pub return_method: String,
    pub media_ids: String,
    pub lines: String,
    pub refund_minor: Option<i64>,
    pub breakdown: Option<String>,
    pub currency: String,
    pub reject_reason: Option<String>,
    pub warehouse_org_id: Option<Uuid>,
    pub requested_at: Timestamp,
    pub deadline_at: Timestamp,
    pub decided_by: Option<Uuid>,
    pub decided_at: Option<Timestamp>,
    pub received_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
