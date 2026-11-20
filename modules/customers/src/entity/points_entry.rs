//! Bảng `customers_points_entries`.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "customers_points_entries")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub profile_id: Uuid,
    pub lot_id: Option<Uuid>,
    pub lot_key: String,
    pub kind: String,
    pub points: i64,
    pub value_minor: i64,
    pub currency: String,
    pub reference: String,
    pub reason: Option<String>,
    pub actor_id: Option<Uuid>,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
