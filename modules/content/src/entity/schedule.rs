//! Bảng `content_schedules`: lịch đăng / gỡ một mục.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_schedules")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub entry_id: Uuid,
    /// `publish` hoặc `unpublish`.
    pub action: String,
    /// Revision được ghim (lịch đăng).
    pub revision_id: Option<Uuid>,
    pub run_at: Timestamp,
    /// `scheduled`, `running`, `succeeded`, `failed`, `canceled`.
    pub status: String,
    pub error_code: Option<String>,
    pub created_by: Option<Uuid>,
    /// Người xác nhận đã duyệt nội dung do AI tạo lúc đặt lịch; dùng khi lịch chạy.
    pub ai_reviewed_by: Option<Uuid>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
