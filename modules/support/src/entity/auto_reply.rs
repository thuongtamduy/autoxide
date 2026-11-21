//! Bảng `support_auto_replies`: nhật ký trả lời tự động.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "support_auto_replies")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub thread: String,
    pub message_ref: String,
    pub source_kind: String,
    pub macro_id: Option<Uuid>,
    pub intent: Option<String>,
    pub confidence_bp: i32,
    pub feedback: Option<String>,
    pub reply_message_id: Option<Uuid>,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
