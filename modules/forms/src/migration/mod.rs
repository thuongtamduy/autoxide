//! Migration của module `forms`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file mới
//! và một dòng vào cuối [`all`].

mod m0001_create_forms;
mod m0007_create_exports;

use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::tenancy::migration::TenantTable;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_forms::Migration),
        // Bảng `forms_events` (transactional outbox) và cột người gây ra event (người lạ khác
        // hệ thống, D-122), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "forms"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "forms"),
        // Bảng `forms_jobs` (job queue): job dọn dẹp hằng ngày.
        autoxide_kernel::job::jobs_migration("m0004_create_jobs", "forms"),
        // Bảng `forms_throttle`: giới hạn bài gửi theo IP băm, email, người dùng (D-116).
        autoxide_kernel::throttle::throttle_migration("m0005_create_throttle", "forms"),
        // Bảng `forms_antispam_nonces`: nonce của token form — gửi lại là trả kết quả cũ (D-119).
        autoxide_kernel::antispam::antispam_migration("m0006_create_antispam_nonces", "forms"),
        // Bảng `forms_exports`: bản xuất CSV lớn chạy bằng job (đặc tả 32 B3).
        Box::new(m0007_create_exports::Migration),
        // Bảng phụ trợ của kernel (event, job, throttle, token, nonce) mang `tenant_id` (D-253).
        autoxide_kernel::tenancy::migration::adopt(
            "m0008_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("forms"),
                autoxide_kernel::job::tenant_table("forms"),
                autoxide_kernel::throttle::tenant_table("forms"),
                autoxide_kernel::antispam::tenant_table("forms"),
            ],
        ),
        // Form, phiên bản, bài gửi và bản xuất thuộc một khách (bước 33, D-279). Chỉ việc xoá
        // dấu vết IP quét cả database (index `_sweep_idx`); việc dọn khác chạy theo từng khách.
        autoxide_kernel::tenancy::migration::adopt(
            "m0009_tenancy",
            vec![
                TenantTable::new("forms_forms")
                    .unique("forms_forms_key_unique", ["tenant_id", "key"])
                    .index("forms_forms_status_idx", ["tenant_id", "status", "id"])
                    .index("forms_forms_created_by_idx", ["tenant_id", "created_by"]),
                TenantTable::new("forms_form_versions")
                    .unique("forms_form_versions_unique", ["tenant_id", "form_id", "version"]),
                TenantTable::new("forms_submissions")
                    .index("forms_submissions_inbox_idx", ["tenant_id", "form_id", "status", "id"])
                    .index("forms_submissions_form_idx", ["tenant_id", "form_id", "id"])
                    .index("forms_submissions_email_idx", ["tenant_id", "email"])
                    .index(
                        "forms_submissions_hash_idx",
                        ["tenant_id", "form_id", "content_hash", "created_at"],
                    )
                    .index("forms_submissions_retention_idx", ["tenant_id", "status", "created_at"])
                    .drop_index("forms_submissions_created_idx")
                    .sweep_index("forms_submissions_created_sweep_idx", ["created_at"]),
                TenantTable::new("forms_exports")
                    .index("forms_exports_form_idx", ["tenant_id", "form_id", "id"])
                    .index("forms_exports_expires_idx", ["tenant_id", "expires_at"]),
            ],
        ),
        // Danh sách theo khách sắp theo `id` cần index `(tenant_id, id)` (đo ở bước 33 C, D-302).
        autoxide_kernel::tenancy::migration::list_index(
            "m0010_tenant_list_index",
            vec!["forms_forms"],
        ),
    ]
}
