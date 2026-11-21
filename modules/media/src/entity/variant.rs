//! Bảng `media_variant`: ảnh thu nhỏ của một phiên bản nội dung (D-093).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "media_variant")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub item_id: Uuid,
    pub version_id: Uuid,
    pub width: i32,
    pub height: i32,
    pub content_type: String,
    pub size: i64,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}

impl Model {
    pub(crate) fn storage_key(&self) -> String {
        storage_key(self.item_id, self.version_id, self.width, &self.content_type)
    }

    /// Bản WebP (D-196), làm thêm bên cạnh bản JPEG / PNG cùng cỡ.
    pub(crate) fn is_webp(&self) -> bool {
        self.content_type == WEBP
    }

    /// Bản AVIF (bước 39 B8, D-20261119-step39-avif), làm bằng job nền khi bật `AUTOXIDE_MEDIA_AVIF`.
    pub(crate) fn is_avif(&self) -> bool {
        self.content_type == AVIF
    }
}

/// Loại nội dung của bản WebP.
pub(crate) const WEBP: &str = "image/webp";

/// Loại nội dung của bản AVIF.
pub(crate) const AVIF: &str = "image/avif";

/// Khoá ở nơi lưu: `media/<item_id>/<version_id>_w<width>` (JPEG / PNG, cạnh khoá của bản gốc)
/// hay `…_w<width>.webp` (bản WebP cùng cỡ), `…_w<width>.avif` (bản AVIF cùng cỡ).
pub(crate) fn storage_key(
    item_id: Uuid,
    version_id: Uuid,
    width: i32,
    content_type: &str,
) -> String {
    let suffix = match content_type {
        WEBP => ".webp",
        AVIF => ".avif",
        _ => "",
    };
    format!("{}_w{width}{suffix}", crate::entity::item::storage_key(item_id, version_id))
}
