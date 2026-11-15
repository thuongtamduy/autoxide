//! Migration của module `notes`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm
//! file mới và một dòng vào cuối [`all`].

mod m0001_create_note;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_note::Migration),
        // Bảng `notes_events` (transactional outbox), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "notes"),
        // Cột `actor_kind` của bảng `notes_events` (người lạ khác hệ thống, D-122).
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "notes"),
        // Bảng phụ trợ của kernel (event, job, throttle, token, nonce) mang `tenant_id` (D-253).
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_helpers",
            vec![autoxide_kernel::event::tenant_table("notes")],
        ),
    ]
}
