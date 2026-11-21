//! Bảng `orders_advance_plans`: kế hoạch trả trước một phần của một đơn có hàng đặt trước (D-232).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_advance_plans")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub order_id: Uuid,
    pub currency: String,
    pub advance_minor: i64,
    pub upfront_minor: i64,
    pub balance_minor: i64,
    pub balance_method: String,
    pub terms: String,
    pub terms_sha256: String,
    pub status: String,
    pub balance_attempt: i32,
    pub balance_requested_at: Option<Timestamp>,
    pub balance_due_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
