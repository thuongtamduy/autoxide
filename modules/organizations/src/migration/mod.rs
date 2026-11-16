//! Migration của module `organizations`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm
//! file mới và một dòng vào cuối [`all`].

mod m0001_create_units;
mod m0005_settings;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_units::Migration),
        // Bảng `organizations_events` (transactional outbox) theo khách từ đầu.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "organizations"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "organizations"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_events",
            vec![autoxide_kernel::event::tenant_table("organizations")],
        ),
        // Cài đặt kế thừa theo cây (bước 34 mục 4).
        Box::new(m0005_settings::Migration),
        // Đường người làm vào khách (bước 34, D-310).
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "organizations"),
    ]
}
