//! Bảng `orders_cod_rules`: quy tắc buộc trả trước khi rủi ro cao, có ngày hiệu lực.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_cod_rules")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub min_orders: i32,
    pub max_fail_ratio_bp: i32,
    pub max_order_value_minor: Option<i64>,
    pub currency: Option<String>,
    pub action: String,
    pub effective_from: DateTimeUtc,
    pub effective_to: Option<DateTimeUtc>,
    pub created_by: Uuid,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
