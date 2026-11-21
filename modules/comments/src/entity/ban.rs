//! Bảng `comments_bans`: cấm bình luận theo người dùng, email hay dấu vết IP.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "comments_bans")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    /// `user`, `email` hoặc `ip`.
    pub kind: String,
    /// ID người dùng, email (chữ thường) hay `ip_fingerprint`.
    pub value: String,
    pub reason: String,
    /// Hết cấm lúc này; `None`: vĩnh viễn.
    pub until: Option<Timestamp>,
    pub created_by: Option<Uuid>,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
