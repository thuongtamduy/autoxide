//! Bảng `conversations_callback_requests`: người mua xin cửa hàng gọi lại.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "conversations_callback_requests")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub thread_id: Uuid,
    pub requester_user_id: Option<Uuid>,
    pub client_id: String,
    pub phone_sealed: Option<String>,
    pub phone_masked: String,
    pub phone_source: String,
    pub window_start: Timestamp,
    pub window_end: Timestamp,
    pub recording_ack_at: Option<Timestamp>,
    pub status: String,
    pub open_key: Option<String>,
    pub assigned_user_id: Option<Uuid>,
    pub attempts: i32,
    pub last_call_id: Option<Uuid>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
