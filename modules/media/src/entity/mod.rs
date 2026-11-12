//! Entity (SeaORM) của các bảng `media_…`. Chỉ dùng trong module này.

pub(crate) mod folder;
pub(crate) mod item;
pub(crate) mod item_tag;
pub(crate) mod lock;
pub(crate) mod tag;
pub(crate) mod variant;

use uuid::Uuid;

/// ID của thư mục gốc trong cột `folder_id` / `parent_id` (D-086). API trả `null`.
pub(crate) const ROOT: Uuid = Uuid::nil();

/// Đổi ID thư mục của API (`None` = gốc) sang giá trị lưu trong cột.
pub(crate) fn stored_folder(folder: Option<Uuid>) -> Uuid {
    folder.unwrap_or(ROOT)
}

/// Đổi giá trị lưu trong cột sang ID thư mục của API.
pub(crate) fn api_folder(stored: Uuid) -> Option<Uuid> {
    (stored != ROOT).then_some(stored)
}
