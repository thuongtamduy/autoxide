//! Bảng `content_revisions`: ảnh chụp đầy đủ các trường sửa được của một mục.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_revisions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub entry_id: Uuid,
    /// JSON của `Snapshot` (xem `revisions.rs`).
    pub snapshot: String,
    /// `initial`, `explicit_save`, `background_save`, `published`, `unpublished`, `restored`,
    /// `scheduled`.
    pub reason: String,
    pub created_by: Option<Uuid>,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
