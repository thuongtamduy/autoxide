//! Bảng `content_edit_locks`: ai đang sửa một mục, tới khi nào (bước 67).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_edit_locks")]
pub struct Model {
    /// Khách sở hữu dòng (bước 33), cột đầu của khoá chính.
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub entry_id: Uuid,
    pub user_id: Uuid,
    pub api_key_id: Option<Uuid>,
    pub acquired_at: Timestamp,
    pub expires_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
