//! Migration của module `media`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm
//! file mới và một dòng vào cuối [`all`].

mod m0001_create_media;
mod m0004_create_tags_and_variants;
mod m0005_fold_search_keys;
mod m0007_variant_formats;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_media::Migration),
        // Bảng `media_events` (transactional outbox), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "media"),
        // Bảng `media_jobs` (job queue): dọn thùng rác và nội dung mồ côi.
        autoxide_kernel::job::jobs_migration("m0003_create_jobs", "media"),
        Box::new(m0004_create_tags_and_variants::Migration),
        Box::new(m0005_fold_search_keys::Migration),
        // Cột `actor_kind` của bảng `media_events` (người lạ khác hệ thống, D-122).
        autoxide_kernel::event::outbox_actor_migration("m0006_add_event_actor", "media"),
        Box::new(m0007_variant_formats::Migration),
    ]
}
