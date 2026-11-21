//! Bảng `customers_wishlist_alert_batches`.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "customers_wishlist_alert_batches")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub profile_id: Uuid,
    pub user_id: Uuid,
    pub local_day: String,
    pub items: String,
    pub status: String,
    pub created_at: Timestamp,
    pub sent_at: Option<Timestamp>,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
