//! Migration của module `menus`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm
//! file mới và một dòng vào cuối [`all`].

mod m0001_create_menus;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_menus::Migration),
        // Bảng `menus_events` (transactional outbox), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "menus"),
        // Cột `actor_kind` của bảng `menus_events` (người lạ khác hệ thống, D-122).
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "menus"),
    ]
}
