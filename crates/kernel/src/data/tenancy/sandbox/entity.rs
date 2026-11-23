//! `kernel_tenants` seen from the sandbox side: the shared columns plus the four sandbox columns
//! (`m0037_tenant_sandbox`). The table's other columns (`read_only`, `data_residency`) have defaults / NULL so
//! they need no declaration here.

use crate::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "kernel_tenants")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub status: String,
    pub parent_id: Option<Uuid>,
    pub placement_id: Uuid,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub read_only: bool,
    /// `production` / `sandbox`.
    pub kind: String,
    /// The sandbox's source tenant; no foreign key.
    pub sandbox_of: Option<Uuid>,
    pub sandbox_expires_at: Option<Timestamp>,
    /// JSON `SandboxOptions` (`products`, `role_id`).
    pub sandbox_options: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
