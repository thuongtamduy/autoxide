//! Migration của module `orders`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file mới và
//! một dòng vào cuối [`all`].

use autoxide_kernel::migration::prelude::*;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        // Bảng `orders_events` (transactional outbox) và `orders_jobs`, định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0001_create_events", "orders"),
        autoxide_kernel::event::outbox_actor_migration("m0002_add_event_actor", "orders"),
        autoxide_kernel::job::jobs_migration("m0003_create_jobs", "orders"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("orders"),
                autoxide_kernel::job::tenant_table("orders"),
            ],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "orders"),
    ]
}
