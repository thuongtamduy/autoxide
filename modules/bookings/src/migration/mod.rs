//! Migration của module `bookings`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file mới và
//! một dòng vào cuối [`all`].

mod m0001_create_bookings;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_bookings::Migration),
        // Bảng `bookings_events` (transactional outbox), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "bookings"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "bookings"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_events",
            vec![autoxide_kernel::event::tenant_table("bookings")],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "bookings"),
        // Job: giữ tạm hết hạn, hoàn cọc, nhắc lịch.
        autoxide_kernel::job::jobs_migration("m0005_create_jobs", "bookings"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0006_tenancy_jobs",
            vec![autoxide_kernel::job::tenant_table("bookings")],
        ),
        // Bảng `bookings_throttle`: giới hạn xin mã SMS theo địa chỉ client (pentest 2026-11).
        autoxide_kernel::throttle::throttle_migration("m0007_create_throttle", "bookings"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0008_tenancy_throttle",
            vec![autoxide_kernel::throttle::tenant_table("bookings")],
        ),
    ]
}
