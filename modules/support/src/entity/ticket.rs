//! Bảng `support_tickets`: một phiếu hỗ trợ.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "support_tickets")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub number: String,
    pub requester_user_id: Option<Uuid>,
    pub requester_email: Option<String>,
    pub requester_phone: Option<String>,
    pub requester_name: String,
    pub locale: Option<String>,
    pub verified_at: Option<DateTimeUtc>,
    pub token_hash: Option<String>,
    pub otp_hash: Option<String>,
    pub otp_expires_at: Option<DateTimeUtc>,
    pub otp_attempts: i32,
    pub category: String,
    pub priority: String,
    pub status: String,
    pub resolution: Option<String>,
    pub subject: String,
    pub channel: String,
    pub assignee_user_id: Option<Uuid>,
    pub team_id: Option<Uuid>,
    pub first_response_due_at: Option<DateTimeUtc>,
    pub resolve_due_at: Option<DateTimeUtc>,
    pub first_responded_at: Option<DateTimeUtc>,
    pub sla_paused_at: Option<DateTimeUtc>,
    pub sla_paused_minutes: i64,
    pub solved_at: Option<DateTimeUtc>,
    pub closed_at: Option<DateTimeUtc>,
    pub breached: bool,
    pub case_id: Option<Uuid>,
    pub dispute_id: Option<Uuid>,
    pub follow_up_of: Option<Uuid>,
    pub escalated_to_platform: bool,
    pub satisfaction: Option<i16>,
    pub satisfaction_note: Option<String>,
    pub auto_reply_paused: bool,
    pub auto_replies_in_row: i32,
    pub last_message_at: DateTimeUtc,
    pub reference: Option<String>,
    pub search_key: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
