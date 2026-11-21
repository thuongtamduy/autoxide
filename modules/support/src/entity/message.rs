//! Bảng `support_messages`: tin nhắn của phiếu — chỉ thêm, không sửa (ẩn kèm lý do).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "support_messages")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub ticket_id: Uuid,
    pub kind: String,
    pub author_kind: String,
    pub author_user_id: Option<Uuid>,
    pub body: String,
    pub media_ids: String,
    pub mentions: String,
    pub hidden_at: Option<Timestamp>,
    pub hidden_by: Option<Uuid>,
    pub hidden_reason: Option<String>,
    pub ai_assisted: bool,
    pub auto_reply: bool,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
