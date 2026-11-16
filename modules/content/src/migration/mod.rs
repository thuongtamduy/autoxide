//! Migration của module `content`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm
//! file mới và một dòng vào cuối [`all`].

mod m0001_create_content;
mod m0004_agent_fields;
mod m0005_translation_requests;
mod m0006_embeddings;
mod m0009_schedule_ai_review;
mod m0012_org;
mod m0013_site_paths;

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
    ]
}
