//! Chính sách tách / gộp khách của các bảng của `content` (bước 38, D-20261118-step38-relocation):
//! bài theo nút của bài, mọi thứ của bài đi theo bài; phân loại, tác giả, menu cũ là cấu hình chép
//! sang khách mới.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .by_org(&[
            "content_entries",
            "content_banners",
            "content_themes",
            "content_shared_sections",
            "content_page_templates",
            "content_theme_variants",
            "content_site_regions",
            "content_types",
        ])
        .follow("content_published", "entry_id", "content_entries")
        .follow("content_revisions", "entry_id", "content_entries")
        .follow("content_schedules", "entry_id", "content_entries")
        .follow("content_entry_terms", "entry_id", "content_entries")
        .follow("content_entry_authors", "entry_id", "content_entries")
        .follow("content_translation_requests", "source_entry_id", "content_entries")
        .follow("content_embeddings", "entry_id", "content_entries")
        .follow("content_custom_index", "record_id", "content_entries")
        .copy_on_split(&[
            "content_terms",
            "content_term_translations",
            "content_authors",
            "content_menus",
            "content_placements",
        ])
        .follow("content_menu_items", "menu_id", "content_menus")
        .stay(&["content_redirects"])
        .ephemeral(&["content_tree_locks", "content_embedding_state", "content_edit_locks"])
}
