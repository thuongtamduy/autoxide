//! Migration của module `content`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm
//! file mới và một dòng vào cuối [`all`].

mod m0001_create_content;
mod m0004_agent_fields;
mod m0005_translation_requests;
mod m0006_embeddings;
mod m0009_schedule_ai_review;
mod m0012_org;
mod m0013_site_paths;
mod m0014_redirect_search;
mod m0015_review_index;
mod m0016_ai_assisted;
mod m0017_restriction;
mod m0018_custom_fields;
mod m0020_published_custom_fields;
mod m0021_banners;
mod m0022_edit_locks;
mod m0023_themes;
mod m0024_shared_sections;
mod m0025_page_templates;
mod m0026_theme_variants;
mod m0027_site_regions;
mod m0028_content_types;
mod m0029_ai_turns;
mod m0030_site_health;
mod m0031_type_detail_layout;
mod m0033_campaign_drafts;
mod m0034_not_found_paths;
mod m0035_list_indexes;
pub(crate) mod m0036_word_index;

use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::tenancy::migration::TenantTable;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_content::Migration),
        // Bảng `content_events` (transactional outbox), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "content"),
        // Bảng `content_jobs` (job queue): hẹn giờ, dọn revision.
        autoxide_kernel::job::jobs_migration("m0003_create_jobs", "content"),
        // Cột cho AI agent: ai tạo bằng API key nào, xin duyệt, nội dung do AI tạo (D-109).
        Box::new(m0004_agent_fields::Migration),
        // Dịch bằng AI: bảng yêu cầu dịch, cột `translated_from_revision_id` (D-109).
        Box::new(m0005_translation_requests::Migration),
        // Tìm theo ý nghĩa: vector của đoạn, số phiên bản để nạp lại (D-109).
        Box::new(m0006_embeddings::Migration),
        // Cột `actor_kind` của bảng `content_events` (người lạ khác hệ thống, D-122).
        autoxide_kernel::event::outbox_actor_migration("m0007_add_event_actor", "content"),
        // Bảng phụ trợ của kernel (event, job, throttle, token, nonce) mang `tenant_id` (D-253).
        autoxide_kernel::tenancy::migration::adopt(
            "m0008_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("content"),
                autoxide_kernel::job::tenant_table("content"),
            ],
        ),
        // Lịch đăng mang xác nhận đã duyệt nội dung AI (D-266).
        Box::new(m0009_schedule_ai_review::Migration),
        // Mọi bảng riêng của `content` thuộc một khách (bước 33): slug, đường dẫn công khai, tác
        // giả, menu, chuyển hướng, term, khoá cây, yêu cầu dịch, embedding chỉ không trùng trong
        // cùng khách. Không index nào quét cả database: job định kỳ (dọn revision, lập index bù)
        // tìm khách qua index có `tenant_id` đứng đầu rồi làm theo từng khách; lịch đăng chạy
        // bằng job của khách.
        autoxide_kernel::tenancy::migration::adopt(
            "m0010_tenancy",
            vec![
                TenantTable::new("content_entries")
                    .unique(
                        "content_entries_translation_unique",
                        ["tenant_id", "translation_group_id", "locale"],
                    )
                    .unique(
                        "content_entries_slug_unique",
                        ["tenant_id", "locale", "kind", "parent_key", "slug"],
                    )
                    .index("content_entries_list_idx", ["tenant_id", "kind", "locale", "id"])
                    .index("content_entries_parent_idx", ["tenant_id", "parent_key", "id"])
                    .index("content_entries_status_idx", ["tenant_id", "status", "id"])
                    .index("content_entries_updated_idx", ["tenant_id", "updated_at", "id"])
                    .index("content_entries_created_by_idx", ["tenant_id", "created_by"]),
                // Khoá chính `entry_id` thành `(tenant_id, entry_id)`: helper chỉ giữ khoá `id`.
                TenantTable::new("content_published")
                    .primary_key(["tenant_id", "entry_id"])
                    .unique("content_published_path_unique", ["tenant_id", "locale", "path"])
                    .index(
                        "content_published_list_idx",
                        ["tenant_id", "locale", "kind", "published_at", "entry_id"],
                    )
                    .index(
                        "content_published_modified_idx",
                        ["tenant_id", "content_modified_at", "entry_id"],
                    )
                    .index("content_published_group_idx", ["tenant_id", "translation_group_id"])
                    .index("content_published_parent_idx", ["tenant_id", "parent_id"]),
                TenantTable::new("content_revisions").index(
                    "content_revisions_entry_idx",
                    ["tenant_id", "entry_id", "created_at", "id"],
                ),
                TenantTable::new("content_schedules")
                    .index("content_schedules_due_idx", ["tenant_id", "status", "run_at"])
                    .index("content_schedules_entry_idx", ["tenant_id", "entry_id", "status"]),
                TenantTable::new("content_redirects")
                    .unique("content_redirects_path_unique", ["tenant_id", "locale", "from_path"])
                    .index("content_redirects_target_idx", ["tenant_id", "to_entry_id"]),
                TenantTable::new("content_tree_locks").primary_key(["tenant_id", "locale"]),
                TenantTable::new("content_terms"),
                TenantTable::new("content_term_translations")
                    .primary_key(["tenant_id", "term_id", "locale"])
                    .unique(
                        "content_term_slug_unique",
                        ["tenant_id", "taxonomy", "locale", "slug"],
                    ),
                TenantTable::new("content_entry_terms")
                    .primary_key(["tenant_id", "entry_id", "term_id"])
                    .index("content_entry_terms_term_idx", ["tenant_id", "term_id", "entry_id"]),
                TenantTable::new("content_authors")
                    .unique("content_authors_slug_unique", ["tenant_id", "slug"])
                    .index("content_authors_user_idx", ["tenant_id", "user_id"]),
                TenantTable::new("content_entry_authors")
                    .primary_key(["tenant_id", "entry_id", "author_id"])
                    .index(
                        "content_entry_authors_author_idx",
                        ["tenant_id", "author_id", "entry_id"],
                    ),
                TenantTable::new("content_menus")
                    .unique("content_menus_unique", ["tenant_id", "key", "locale"]),
                TenantTable::new("content_menu_items")
                    .index("content_menu_items_menu_idx", ["tenant_id", "menu_id", "sort_order"]),
                TenantTable::new("content_translation_requests")
                    .unique(
                        "content_translation_requests_active_unique",
                        ["tenant_id", "active_key"],
                    )
                    .index(
                        "content_translation_requests_source_idx",
                        ["tenant_id", "source_entry_id", "id"],
                    ),
                TenantTable::new("content_embeddings")
                    .unique(
                        "content_embeddings_chunk_unique",
                        ["tenant_id", "entry_id", "scope", "chunk_index"],
                    )
                    .index("content_embeddings_scope_idx", ["tenant_id", "scope", "locale"]),
                TenantTable::new("content_embedding_state")
                    .unique("content_embedding_state_unique", ["tenant_id", "scope", "locale"]),
            ],
        ),
        // Danh sách theo khách sắp theo `id` cần index `(tenant_id, id)` (đo ở bước 33 C, D-302).
        autoxide_kernel::tenancy::migration::list_index(
            "m0011_tenant_list_index",
            vec!["content_entries", "content_redirects", "content_terms", "content_authors"],
        ),
        // Bài, trang thuộc một nút của cây tổ chức (bước 34, D-208).
        Box::new(m0012_org::Migration),
        // Đường dẫn công khai riêng theo website (bước 34, D-307).
        Box::new(m0013_site_paths::Migration),
        // Đường người làm vào khách (bước 34, D-310).
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "content"),
        // Khoá tìm của chuyển hướng: `q` của `GET /redirects` (W-059 của giao diện).
        Box::new(m0014_redirect_search::Migration),
        // Index của nguồn việc "chờ duyệt" (bước 51 P2).
        Box::new(m0015_review_index::Migration),
        // Nhãn "viết từ gợi ý của AI" (Luật 134/2025/QH15, bước 37, D-362).
        Box::new(m0016_ai_assisted::Migration),
        // Mục bị ẩn tạm vì vụ việc của `compliance` (bước 37, D-363).
        Box::new(m0017_restriction::Migration),
        // Trường tự định nghĩa của khách (bước 51 P19): giá trị trên bài, index để lọc `cf.<key>`.
        Box::new(m0018_custom_fields::Migration),
        autoxide_kernel::custom_fields::index_migration("m0019_custom_index", "content"),
        Box::new(m0020_published_custom_fields::Migration),
        // Vị trí hiển thị và banner (bước 55, D-236).
        Box::new(m0021_banners::Migration),
        // Khoá sửa trang: ai đang sửa, tới khi nào (bước 67).
        Box::new(m0022_edit_locks::Migration),
        // Bộ giao diện của website: token ghi đè theo nút (bước 67).
        Box::new(m0023_themes::Migration),
        // Khu dùng chung nhiều trang (bước 67).
        Box::new(m0024_shared_sections::Migration),
        // Trang mẫu theo ngành và trang khách lưu thành mẫu (bước 67).
        Box::new(m0025_page_templates::Migration),
        // Bản chủ đề theo lịch: mùa, ngày lễ, khung giờ (bước 67, D-234).
        Box::new(m0026_theme_variants::Migration),
        // Đầu trang, chân trang của website (bước 67, D-234).
        Box::new(m0027_site_regions::Migration),
        // Loại nội dung tự định nghĩa và cột `type_id` của mục (bước 67, D-235).
        Box::new(m0028_content_types::Migration),
        // AI tạo và sửa trang: lịch sử trò chuyện của trang (bước 67).
        Box::new(m0029_ai_turns::Migration),
        // Kiểm tra sức khoẻ website: lần kiểm gần nhất (bước 67).
        Box::new(m0030_site_health::Migration),
        // Trang chi tiết dựng sẵn của loại nội dung (bước 67, D-235).
        Box::new(m0031_type_detail_layout::Migration),
        // Index lọc `cf.<key>` theo giá trị của BẢN ĐĂNG (bảng index của kernel, tên riêng): danh sách
        // công khai không được lọc theo giá trị nháp (D-20261123-content-types-later).
        autoxide_kernel::custom_fields::index_migration(
            "m0032_published_custom_index",
            "content_published",
        ),
        // Bộ nháp chiến dịch từ một chương trình khuyến mại (bước 67).
        Box::new(m0033_campaign_drafts::Migration),
        // Đếm đường dẫn 404 của website, gợi ý chuyển hướng (bước 67).
        Box::new(m0034_not_found_paths::Migration),
        // Index cho danh sách có phân trang (bước 70 P9).
        Box::new(m0035_list_indexes::Migration),
        // Chỉ mục từ của bản công khai: bảng `content_published_words`, giống nhau trên hai database (PT-15,
        // D-20261126-content-word-index).
        Box::new(m0036_word_index::Migration),
    ]
}
