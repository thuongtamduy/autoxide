//! Migration của module `webhooks`, theo thứ tự áp dụng. Đã commit thì không sửa —
//! thêm file mới và một dòng vào cuối [`all`].

mod m0001_create_webhooks;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_webhooks::Migration),
        // Bảng `webhooks_events` (transactional outbox), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "webhooks"),
        // Bảng `webhooks_jobs` (job queue): mỗi lần gửi là một job, có retry.
        autoxide_kernel::job::jobs_migration("m0003_create_jobs", "webhooks"),
        // Cột `actor_kind` của bảng `webhooks_events` (người lạ khác hệ thống, D-116).
        autoxide_kernel::event::outbox_actor_migration("m0004_add_event_actor", "webhooks"),
    ]
}
