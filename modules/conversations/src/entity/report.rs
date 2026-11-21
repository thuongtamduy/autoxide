//! Bảng `conversations_reports`: báo vi phạm về một cuộc trò chuyện hay một tin.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "conversations_reports")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub thread_id: Uuid,
    pub message_id: Option<Uuid>,
    pub reporter_kind: String,
    pub reporter_user_id: Option<Uuid>,
    pub reason: String,
    pub status: String,
    pub resolved_by: Option<Uuid>,
    pub resolved_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
