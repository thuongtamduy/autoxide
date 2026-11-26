//! Bảng `customers_wishlist_items`.

use autoxide_kernel::time::Timestamp;
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
    /// UUIDv7 cấp khi thêm (dòng cũ: cấp ở migration `m0013`), cho cursor của danh sách; unique trong
    /// khách (`customers_wishlist_items_id_key`).
    pub id: Uuid,
    pub variant_id: Option<Uuid>,
    pub added_at: Timestamp,
    pub baseline_price_minor: Option<i64>,
    pub baseline_currency: Option<String>,
    pub baseline_at: Option<Timestamp>,
    pub back_in_stock_alerted_at: Option<Timestamp>,
    pub price_drop_alerted_at: Option<Timestamp>,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
