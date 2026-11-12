//! Entity của bảng `files_file` (SeaORM).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "files_file")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub content_type: String,
    pub size: i64,
    pub sha256: String,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    /// Khoá của nội dung ở nơi lưu.
    pub(crate) fn storage_key(&self) -> String {
        storage_key(self.id)
    }

    /// `ETag` của nội dung: SHA-256 trong ngoặc kép.
    pub(crate) fn etag(&self) -> String {
        format!("\"{}\"", self.sha256)
    }
}

pub(crate) fn storage_key(id: Uuid) -> String {
    format!("files/{id}")
}
