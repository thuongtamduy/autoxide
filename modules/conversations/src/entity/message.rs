//! Bảng `conversations_messages`: tin nhắn, chỉ thêm (không sửa; thu hồi thì ẩn nội dung, giữ dấu).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "conversations_messages")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub thread_id: Uuid,
    pub author_kind: String,
    pub author_user_id: Option<Uuid>,
    pub body: String,
    pub media_ids: String,
    pub client_id: String,
    pub call_id: Option<Uuid>,
    pub auto_reply: bool,
    pub recalled_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
