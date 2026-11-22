//! SeaORM entity of the table `bookmarks_bookmark`. Private to the module.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "bookmarks_bookmark")]
pub struct Model {
    // UUIDv7, made in the application: never an auto-increment id.
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// The tenant (customer) the bookmark belongs to; every query filters on it (`.scoped`).
    pub tenant_id: Uuid,
    /// The user. Only the id: no foreign key into the tables of the `auth` module.
    pub owner_id: Uuid,
    pub url: String,
    pub title: String,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

// One line that lets `.scoped(&tenant)` filter this entity by tenant.
impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
