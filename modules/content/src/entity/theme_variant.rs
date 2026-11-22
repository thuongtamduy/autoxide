//! Bảng `content_theme_variants`: bản chủ đề theo lịch của một website (bước 67, D-234).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_theme_variants")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    /// Website (nút của cây tổ chức).
    pub org_id: Uuid,
    pub name: String,
    /// Object JSON: token ghi đè.
    pub overrides: String,
    pub effect: Option<String>,
    pub effect_intensity: Option<String>,
    /// Object JSON: lịch (`theme::schedule`).
    pub schedule: String,
    pub starts_at: Option<Timestamp>,
    pub ends_at: Option<Timestamp>,
    pub priority: i32,
    pub status: String,
    pub preset_key: Option<String>,
    pub created_by: Option<Uuid>,
    pub updated_by: Option<Uuid>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
