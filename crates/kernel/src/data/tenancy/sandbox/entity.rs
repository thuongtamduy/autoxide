//! `kernel_tenants` nhìn từ phía sandbox: cột chung cùng bốn cột của sandbox (`m0037_tenant_sandbox`).
//! Cột khác của bảng (`read_only`, `data_residency`) có mặc định / NULL nên không cần khai ở đây.

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
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub read_only: bool,
    /// `production` / `sandbox`.
    pub kind: String,
    /// Khách gốc của sandbox; không foreign key.
    pub sandbox_of: Option<Uuid>,
    pub sandbox_expires_at: Option<DateTimeUtc>,
    /// JSON `SandboxOptions` (`products`, `role_id`).
    pub sandbox_options: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
