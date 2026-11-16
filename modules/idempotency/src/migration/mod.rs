//! Migration của module `idempotency`, theo thứ tự áp dụng. Đã commit thì không sửa —
//! thêm file mới và một dòng vào cuối [`all`].

mod m0001_create_record;

use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::tenancy::migration::TenantTable;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_record::Migration),
        // Bảng `idempotency_jobs` (job dọn key hết hạn), định nghĩa ở kernel.
        autoxide_kernel::job::jobs_migration("m0002_create_jobs", "idempotency"),
        // Bảng phụ trợ của kernel (event, job, throttle, token, nonce) mang `tenant_id` (D-253).
        autoxide_kernel::tenancy::migration::adopt(
            "m0003_tenancy_helpers",
            vec![autoxide_kernel::job::tenant_table("idempotency")],
        ),
        // Key thuộc một khách (bước 33): cùng key ở hai khách là hai lần chạy khác nhau. Index hết hạn
        // phục vụ job dọn cả database nên không bắt đầu bằng `tenant_id`.
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy",
            vec![
                TenantTable::new("idempotency_record")
                    .unique("idempotency_record_key_unique", ["tenant_id", "scope", "key"])
                    .drop_index("idempotency_record_expires_idx")
                    .sweep_index("idempotency_record_expires_sweep_idx", ["expires_at"]),
            ],
        ),
    ]
}
