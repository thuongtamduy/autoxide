//! The table `kernel_saved_views`, for the paged list `GET /api/v1/me/views` (keyset on the ID). The writes
//! of `views.rs` build their statements by hand; the columns here are the same.

use crate::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "kernel_saved_views")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub resource: String,
    pub name: String,
    pub scope: String,
    pub org_id: Option<Uuid>,
    pub owner_id: Uuid,
    pub params: String,
    pub columns: String,
    pub group_by: Option<String>,
    pub density: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl crate::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
