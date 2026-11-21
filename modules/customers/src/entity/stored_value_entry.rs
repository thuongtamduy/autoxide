//! Bảng `customers_stored_value_entries`.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "customers_stored_value_entries")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub instrument: String,
    pub instrument_id: Uuid,
    pub kind: String,
    pub amount_minor: i64,
    pub currency: String,
    pub reference: String,
    pub reason: Option<String>,
    pub actor_id: Option<Uuid>,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
