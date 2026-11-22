//! Bảng `content_ai_turns`: lịch sử trò chuyện AI của trang (bước 67).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_ai_turns")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub entry_id: Uuid,
    pub kind: String,
    pub instruction: String,
    pub selection: Option<String>,
    pub status: String,
    pub tries: i32,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    pub base_version: Timestamp,
    pub previous_revision_id: Option<Uuid>,
    pub revision_id: Option<Uuid>,
    /// JSON của `PrincipalSnapshot`.
    pub principal: String,
    pub requested_by: Uuid,
    pub created_at: Timestamp,
    pub finished_at: Option<Timestamp>,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
