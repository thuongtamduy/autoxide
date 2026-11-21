//! Bảng `customers_order_facts`.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "customers_order_facts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub order_id: Uuid,
    pub profile_id: Uuid,
    pub org_id: Uuid,
    pub number: String,
    pub channel: String,
    pub status: String,
    pub currency: String,
    pub net_minor: i64,
    pub refunded_minor: i64,
    pub placed_at: Timestamp,
    pub completed_at: Option<Timestamp>,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
