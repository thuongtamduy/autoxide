//! Bảng `customers_wishlist_items`.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "customers_wishlist_items")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub profile_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub product_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub variant_key: String,
    pub variant_id: Option<Uuid>,
    pub added_at: DateTimeUtc,
    pub baseline_price_minor: Option<i64>,
    pub baseline_currency: Option<String>,
    pub baseline_at: Option<DateTimeUtc>,
    pub back_in_stock_alerted_at: Option<DateTimeUtc>,
    pub price_drop_alerted_at: Option<DateTimeUtc>,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
