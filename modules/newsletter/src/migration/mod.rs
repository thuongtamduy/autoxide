//! Migration của module `newsletter`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file
//! mới và một dòng vào cuối [`all`].

mod m0001_create_newsletter;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_newsletter::Migration),
        // Bảng `newsletter_events` (transactional outbox) và cột người gây ra event (người lạ
        // khác hệ thống, D-122), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "newsletter"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "newsletter"),
        // Bảng `newsletter_jobs` (job queue): chụp người nhận, gửi theo lô, dọn dẹp hằng ngày.
        autoxide_kernel::job::jobs_migration("m0004_create_jobs", "newsletter"),
        // Bảng `newsletter_throttle`: giới hạn đăng ký theo IP băm, email, người dùng; số mail
        // xác nhận gửi tới một địa chỉ (D-116).
        autoxide_kernel::throttle::throttle_migration("m0005_create_throttle", "newsletter"),
        // Bảng `newsletter_antispam_nonces`: nonce của token form — gửi lại là trả kết quả cũ
        // (D-119).
        autoxide_kernel::antispam::antispam_migration("m0006_create_antispam_nonces", "newsletter"),
        // Bảng `newsletter_tokens`: token xác nhận đăng ký (dùng một lần, có hạn — D-117).
        autoxide_kernel::tokens::tokens_migration("m0007_create_tokens", "newsletter"),
    ]
}
