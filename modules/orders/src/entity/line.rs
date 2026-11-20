//! Bảng `orders_lines`: một dòng của đơn (bản chụp lúc đặt).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_lines")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub order_id: Uuid,
    pub line_key: String,
    pub parent_line_id: Option<Uuid>,
    pub variant_id: Uuid,
    pub product_id: Uuid,
    pub category_id: Option<Uuid>,
    pub seller_org_id: Uuid,
    pub kind: String,
    pub sku: String,
    pub name: String,
    pub snapshot: String,
    pub sale_mode: String,
    pub preorder_available_on: Option<Date>,
    pub requires_shipping: bool,
    pub tax_inclusive: bool,
    pub quantity: i64,
    pub unit_price_minor: i64,
    pub subtotal_minor: i64,
    pub discount_minor: i64,
    pub net_minor: i64,
    pub tax_minor: i64,
    pub total_minor: i64,
    pub compare_at_minor: Option<i64>,
    pub discounts: String,
    pub gift_of: Option<Uuid>,
    pub warehouse_org_id: Option<Uuid>,
    pub returned_quantity: i64,
    pub allocated_quantity: i64,
    pub cancelled_quantity: i64,
    pub preorder_delayed_to: Option<Date>,
    pub advance_minor: Option<i64>,
    pub preorder_commit_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
