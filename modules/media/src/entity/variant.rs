//! Bảng `media_variant`: ảnh thu nhỏ của một phiên bản nội dung (D-093).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "media_variant")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub item_id: Uuid,
    pub version_id: Uuid,
    pub width: i32,
    pub height: i32,
    pub content_type: String,
    pub size: i64,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    pub(crate) fn storage_key(&self) -> String {
        storage_key(self.item_id, self.version_id, self.width)
    }
}

/// Khoá ở nơi lưu: `media/<item_id>/<version_id>_w<width>`, cạnh khoá của bản gốc.
pub(crate) fn storage_key(item_id: Uuid, version_id: Uuid, width: i32) -> String {
    format!("{}_w{width}", crate::entity::item::storage_key(item_id, version_id))
}
