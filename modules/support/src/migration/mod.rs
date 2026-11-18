//! Migration của module `support`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file mới
//! và một dòng vào cuối [`all`].

mod m0001_create_support;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_support::Migration),
        // Bảng phụ trợ của kernel: outbox event, job, throttle, nonce chống spam (D-253).
        autoxide_kernel::event::outbox_migration("m0002_create_events", "support"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "support"),
        autoxide_kernel::job::jobs_migration("m0004_create_jobs", "support"),
        autoxide_kernel::throttle::throttle_migration("m0005_create_throttle", "support"),
        autoxide_kernel::antispam::antispam_migration("m0006_create_antispam_nonces", "support"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0007_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("support"),
                autoxide_kernel::job::tenant_table("support"),
                autoxide_kernel::throttle::tenant_table("support"),
                autoxide_kernel::antispam::tenant_table("support"),
            ],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "support"),
    ]
}
