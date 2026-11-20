//! Bảng `customers_points_lots`.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "customers_points_lots")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub profile_id: Uuid,
    pub earned: i64,
    pub remaining: i64,
    pub held: i64,
    pub value_minor_per_point: i64,
    pub currency: String,
    pub expires_at: DateTimeUtc,
    pub source_ref: String,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
