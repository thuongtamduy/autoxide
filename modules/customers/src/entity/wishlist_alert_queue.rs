//! Bảng `customers_wishlist_alert_queue`.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "customers_wishlist_alert_queue")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub profile_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub product_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub variant_key: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub kind: String,
    pub variant_id: Option<Uuid>,
    pub price_minor: Option<i64>,
    pub currency: Option<String>,
    pub queued_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
