//! Entity (SeaORM) của các bảng `content_…`. Chỉ dùng trong module này.

pub(crate) mod author;
pub(crate) mod entry;
pub(crate) mod entry_author;
pub(crate) mod entry_term;
pub(crate) mod menu;
pub(crate) mod menu_item;
pub(crate) mod published;
pub(crate) mod redirect;
pub(crate) mod revision;
pub(crate) mod schedule;
pub(crate) mod term;
pub(crate) mod term_translation;
pub(crate) mod tree_lock;

use uuid::Uuid;

/// "Không có cha" trong cột `parent_key` (xem migration m0001).
pub(crate) const ROOT: Uuid = Uuid::nil();
