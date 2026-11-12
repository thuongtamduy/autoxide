//! Migration của module `content`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm
//! file mới và một dòng vào cuối [`all`].

mod m0001_create_content;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_content::Migration),
        // Bảng `content_events` (transactional outbox), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "content"),
        // Bảng `content_jobs` (job queue): hẹn giờ, dọn revision.
        autoxide_kernel::job::jobs_migration("m0003_create_jobs", "content"),
    ]
}
