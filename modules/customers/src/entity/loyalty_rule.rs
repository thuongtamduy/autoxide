//! Bảng `customers_loyalty_rules`.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "customers_loyalty_rules")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub earn_per_minor: i64,
    pub points_per_unit: i64,
    pub point_value_minor: i64,
    pub currency: String,
    pub points_valid_months: i32,
    pub min_redeem_points: i64,
    pub max_redeem_ratio_bp: i32,
    pub effective_from: DateTimeUtc,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
