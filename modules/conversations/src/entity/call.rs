//! Bảng `conversations_calls`: một cuộc gọi (người mua gọi tới hay cửa hàng gọi ra).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "conversations_calls")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub thread_id: Uuid,
    pub callback_id: Option<Uuid>,
    pub direction: String,
    pub status: String,
    pub provider: String,
    pub provider_call_id: Option<String>,
    pub agent_user_id: Option<Uuid>,
    pub caller_user_id: Option<Uuid>,
    pub started_at: Option<Timestamp>,
    pub answered_at: Option<Timestamp>,
    pub ended_at: Option<Timestamp>,
    pub duration_seconds: Option<i32>,
    pub recording_enabled: bool,
    pub recording_ack_at: Option<Timestamp>,
    pub provider_recording_id: Option<String>,
    pub recording_ref: Option<String>,
    pub recording_expires_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
