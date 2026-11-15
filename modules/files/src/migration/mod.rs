//! Migration của module `files`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm
//! file mới và một dòng vào cuối [`all`].

mod m0001_create_file;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_file::Migration),
        // Bảng `files_events` (transactional outbox), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "files"),
        // Cột `actor_kind` của bảng `files_events` (người lạ khác hệ thống, D-122).
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "files"),
        // Bảng `files_jobs` (job queue): job dọn nội dung mồ côi hằng ngày (D-160).
        autoxide_kernel::job::jobs_migration("m0004_create_jobs", "files"),
        // Bảng phụ trợ của kernel (event, job, throttle, token, nonce) mang `tenant_id` (D-253).
        autoxide_kernel::tenancy::migration::adopt(
            "m0005_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("files"),
                autoxide_kernel::job::tenant_table("files"),
            ],
        ),
    ]
}
