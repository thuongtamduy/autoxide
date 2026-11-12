//! Migration của module `ai`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file mới và
//! một dòng vào cuối [`all`].

mod m0001_create_usage;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_usage::Migration),
        // Bảng `ai_jobs` (job queue): dọn số đếm cũ.
        autoxide_kernel::job::jobs_migration("m0002_create_jobs", "ai"),
    ]
}
