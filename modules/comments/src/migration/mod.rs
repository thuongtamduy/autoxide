//! Migration của module `comments`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file mới
//! và một dòng vào cuối [`all`].

mod m0001_create_comments;
mod m0008_name_key_lookalikes;
mod m0009_questions;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_comments::Migration),
        // Bảng `comments_events` (transactional outbox) và cột người gây ra event (người lạ khác
        // hệ thống, D-122), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "comments"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "comments"),
        // Bảng `comments_jobs` (job queue): job dọn dẹp hằng ngày.
        autoxide_kernel::job::jobs_migration("m0004_create_jobs", "comments"),
        // Bảng `comments_throttle`: giới hạn theo IP băm, email, người dùng; báo cáo; mail xác
        // minh (D-116).
        autoxide_kernel::throttle::throttle_migration("m0005_create_throttle", "comments"),
        // Bảng `comments_antispam_nonces`: nonce của token form — gửi lại là trả kết quả cũ (D-119).
        autoxide_kernel::antispam::antispam_migration("m0006_create_antispam_nonces", "comments"),
        // Bảng `comments_tokens`: xác minh email của khách, huỷ theo dõi trả lời (D-117).
        autoxide_kernel::tokens::tokens_migration("m0007_create_tokens", "comments"),
        // Khoá tên so theo dạng "khung": tên giả chữ Latin bằng chữ Kirin / Hy Lạp (D-151).
        Box::new(m0008_name_key_lookalikes::Migration),
        // Hỏi đáp: nhãn tác giả, khoá tìm đúng dấu, số câu hỏi theo đích (D-202).
        Box::new(m0009_questions::Migration),
    ]
}
