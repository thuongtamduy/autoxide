//! Migration của module `forms`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file mới
//! và một dòng vào cuối [`all`].

mod m0001_create_forms;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_forms::Migration),
        // Bảng `forms_events` (transactional outbox) và cột người gây ra event (người lạ khác
        // hệ thống, D-116), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "forms"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "forms"),
        // Bảng `forms_jobs` (job queue): job dọn dẹp hằng ngày.
        autoxide_kernel::job::jobs_migration("m0004_create_jobs", "forms"),
        // Bảng `forms_throttle`: giới hạn bài gửi theo IP băm, email, người dùng (D-110).
        autoxide_kernel::throttle::throttle_migration("m0005_create_throttle", "forms"),
        // Bảng `forms_antispam_nonces`: nonce của token form — gửi lại là trả kết quả cũ (D-113).
        autoxide_kernel::antispam::antispam_migration("m0006_create_antispam_nonces", "forms"),
    ]
}
