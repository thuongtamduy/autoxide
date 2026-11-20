//! Migration của module `customers`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file mới và
//! một dòng vào cuối [`all`].

mod m0001_create_customers;
mod m0006_loyalty;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        // Hồ sơ, lịch sử mua, nhóm khách (bước 47 mục 1–2).
        Box::new(m0001_create_customers::Migration),
        // Bảng `customers_events` (transactional outbox) và `customers_jobs`, định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "customers"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "customers"),
        autoxide_kernel::job::jobs_migration("m0004_create_jobs", "customers"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0005_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("customers"),
                autoxide_kernel::job::tenant_table("customers"),
            ],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "customers"),
        // Điểm thưởng, hạng, voucher tặng (bước 47 mục 3, 6).
        Box::new(m0006_loyalty::Migration),
    ]
}
