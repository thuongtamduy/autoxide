//! Migration của module `media`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm
//! file mới và một dòng vào cuối [`all`].

mod m0001_create_media;
mod m0004_create_tags_and_variants;
mod m0005_fold_search_keys;
mod m0007_variant_formats;
mod m0008_alt_translations;
mod m0010_add_decorative;

use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::tenancy::migration::TenantTable;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_media::Migration),
        // Bảng `media_events` (transactional outbox), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "media"),
        // Bảng `media_jobs` (job queue): dọn thùng rác và nội dung mồ côi.
        autoxide_kernel::job::jobs_migration("m0003_create_jobs", "media"),
        Box::new(m0004_create_tags_and_variants::Migration),
        Box::new(m0005_fold_search_keys::Migration),
        // Cột `actor_kind` của bảng `media_events` (người lạ khác hệ thống, D-122).
        autoxide_kernel::event::outbox_actor_migration("m0006_add_event_actor", "media"),
        Box::new(m0007_variant_formats::Migration),
        // `alt` theo ngôn ngữ (D-202).
        Box::new(m0008_alt_translations::Migration),
        // Bảng phụ trợ của kernel (event, job, throttle, token, nonce) mang `tenant_id` (D-253).
        autoxide_kernel::tenancy::migration::adopt(
            "m0009_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("media"),
                autoxide_kernel::job::tenant_table("media"),
            ],
        ),
        // Cờ `decorative`: ảnh trang trí, `alt` trống có chủ ý (D-267).
        Box::new(m0010_add_decorative::Migration),
        // Item, thư mục, nhãn, ảnh thu nhỏ, bản dịch `alt` và dòng khoá cây thư mục thuộc một khách
        // (bước 33). Chỉ việc tìm khách có item quá hạn trong thùng rác quét cả database (index
        // `_sweep_idx`); việc xoá hẳn chạy theo từng khách.
        autoxide_kernel::tenancy::migration::adopt(
            "m0011_tenancy",
            vec![
                TenantTable::new("media_folder")
                    .unique(
                        "media_folder_parent_name_unique",
                        ["tenant_id", "parent_id", "name_key"],
                    )
                    .index("media_folder_name_idx", ["tenant_id", "name_key", "id"]),
                TenantTable::new("media_item")
                    .index("media_item_folder_idx", ["tenant_id", "folder_id", "id"])
                    .index("media_item_name_idx", ["tenant_id", "name_key", "id"])
                    .index("media_item_size_idx", ["tenant_id", "size", "id"])
                    .index("media_item_sha256_idx", ["tenant_id", "sha256"])
                    .index("media_item_trashed_idx", ["tenant_id", "trashed_at"])
                    .index("media_item_uploaded_by_idx", ["tenant_id", "uploaded_by"])
                    .sweep_index("media_item_trashed_sweep_idx", ["trashed_at"]),
                TenantTable::new("media_lock").primary_key(["tenant_id", "name"]),
                TenantTable::new("media_tag")
                    .unique("media_tag_name_unique", ["tenant_id", "name_key"]),
                TenantTable::new("media_item_tag")
                    .primary_key(["tenant_id", "item_id", "tag_id"])
                    .index("media_item_tag_tag_idx", ["tenant_id", "tag_id", "item_id"]),
                TenantTable::new("media_variant").unique(
                    "media_variant_format_unique",
                    ["tenant_id", "item_id", "version_id", "width", "content_type"],
                ),
                TenantTable::new("media_alt_translations").primary_key([
                    "tenant_id",
                    "media_id",
                    "locale",
                ]),
            ],
        ),
        // Danh sách theo khách sắp theo `id` cần index `(tenant_id, id)` (đo ở bước 33 C, D-302).
        autoxide_kernel::tenancy::migration::list_index(
            "m0012_tenant_list_index",
            vec!["media_item", "media_folder", "media_tag"],
        ),
    ]
}
