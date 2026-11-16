//! Migration của module `ai`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file mới và
//! một dòng vào cuối [`all`].

mod m0001_create_usage;

use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::tenancy::migration::TenantTable;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_usage::Migration),
        // Bảng `ai_jobs` (job queue): dọn số đếm cũ.
        autoxide_kernel::job::jobs_migration("m0002_create_jobs", "ai"),
        // Bảng phụ trợ của kernel (event, job, throttle, token, nonce) mang `tenant_id` (D-253).
        autoxide_kernel::tenancy::migration::adopt(
            "m0003_tenancy_helpers",
            vec![autoxide_kernel::job::tenant_table("ai")],
        ),
        // Số token đếm theo khách (bước 33); job dọn quét theo ngày trên cả database.
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy",
            vec![
                TenantTable::new("ai_usage")
                    .unique("ai_usage_day_purpose_unique", ["tenant_id", "day", "purpose"])
                    .sweep_index("ai_usage_day_sweep_idx", ["day"]),
            ],
        ),
    ]
}
