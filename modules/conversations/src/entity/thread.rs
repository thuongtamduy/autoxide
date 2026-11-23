//! Bảng `conversations_threads`: một cuộc trò chuyện giữa người mua và một nút (cửa hàng, gian hàng).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "conversations_threads")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub customer_user_id: Option<Uuid>,
    pub subject: Option<String>,
    pub status: String,
    pub assignee_user_id: Option<Uuid>,
    pub open_key: Option<String>,
    pub unread_customer: i32,
    pub unread_staff: i32,
    pub auto_reply_paused: bool,
    pub auto_replies_in_row: i32,
    pub needs_agent: bool,
    pub ticket_id: Option<Uuid>,
    pub draft_reply: Option<String>,
    pub draft_for: Option<Uuid>,
    pub locale: Option<String>,
    pub last_message_at: Timestamp,
    pub closed_at: Option<Timestamp>,
    /// Đầu khung báo tin chưa đọc của người mua / nhân viên (tin chưa đọc đầu tiên); trống khi chưa có khung.
    pub notify_customer_from: Option<Timestamp>,
    pub notify_staff_from: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
