//! Bảng `media_item`: một item của thư viện (ảnh, video, âm thanh, tài liệu).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "media_item")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    /// `entity::ROOT` khi ở thư mục gốc.
    pub folder_id: Uuid,
    pub name: String,
    /// `search::fold(name)` (viết thường, bỏ dấu): để tìm và sắp theo tên (D-104).
    pub name_key: String,
    pub content_type: String,
    pub size: i64,
    pub sha256: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub alt: String,
    pub title: String,
    pub caption: String,
    /// Ảnh chỉ để trang trí: `alt` để trống có chủ ý (D-267).
    pub decorative: bool,
    /// `private` hoặc `public`.
    pub visibility: String,
    pub version_id: Uuid,
    pub uploaded_by: Option<Uuid>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub trashed_at: Option<DateTimeUtc>,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}

impl Model {
    /// Khoá nội dung hiện tại ở nơi lưu.
    pub(crate) fn storage_key(&self) -> String {
        storage_key(self.id, self.version_id)
    }
}

/// Khoá ở nơi lưu của phiên bản `version_id` của item `id`. Mọi khoá của module nằm
/// dưới `media/`; module không bao giờ đụng khoá khác.
pub(crate) fn storage_key(id: Uuid, version_id: Uuid) -> String {
    format!("{KEY_PREFIX}{id}/{version_id}")
}

/// Tiền tố mọi khoá của module ở nơi lưu.
pub(crate) const KEY_PREFIX: &str = "media/";
