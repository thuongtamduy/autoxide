//! Bảng `conversations_voice_events`: webhook của nhà cung cấp thoại, nguyên văn đã kiểm chữ ký.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "conversations_voice_events")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub provider: String,
    pub provider_event_id: String,
    pub call_id: Option<Uuid>,
    pub notification: String,
    pub body: String,
    pub status: String,
    pub received_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
