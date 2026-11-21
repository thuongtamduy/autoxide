//! Bảng `customers_stored_value_holds`.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "customers_stored_value_holds")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub reference: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub instrument: String,
    pub instrument_id: Uuid,
    pub amount_minor: i64,
    pub currency: String,
    pub status: String,
    pub expires_at: Timestamp,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
