//! Bảng `content_schedules`: lịch đăng / gỡ một mục.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_schedules")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub entry_id: Uuid,
    /// `publish` hoặc `unpublish`.
    pub action: String,
    /// Revision được ghim (lịch đăng).
    pub revision_id: Option<Uuid>,
    pub run_at: DateTimeUtc,
    /// `scheduled`, `running`, `succeeded`, `failed`, `canceled`.
    pub status: String,
    pub error_code: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
