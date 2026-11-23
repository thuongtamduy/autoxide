//! Entity (SeaORM) của các bảng `content_…`. Chỉ dùng trong module này.

pub(crate) mod ai_turn;
pub(crate) mod author;
pub(crate) mod banner;
pub(crate) mod campaign_draft;
pub(crate) mod content_type;
pub(crate) mod edit_lock;
pub(crate) mod embedding;
pub(crate) mod embedding_state;
pub(crate) mod entry;
pub(crate) mod entry_author;
pub(crate) mod entry_term;
pub(crate) mod menu;
pub(crate) mod menu_item;
pub(crate) mod not_found_path;
pub(crate) mod page_template;
pub(crate) mod placement;
pub(crate) mod published;
pub(crate) mod redirect;
pub(crate) mod revision;
pub(crate) mod schedule;
pub(crate) mod shared_section;
pub(crate) mod site_health;
pub(crate) mod site_region;
pub(crate) mod term;
pub(crate) mod term_translation;
pub(crate) mod theme;
pub(crate) mod theme_variant;
pub(crate) mod translation_request;
pub(crate) mod tree_lock;

use uuid::Uuid;

/// "Không có cha" trong cột `parent_key` (xem migration m0001).
pub(crate) const ROOT: Uuid = Uuid::nil();
