//! Migration của module `content`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm
//! file mới và một dòng vào cuối [`all`].

mod m0001_create_content;
mod m0004_agent_fields;
mod m0005_translation_requests;
mod m0006_embeddings;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_content::Migration),
        // Bảng `content_events` (transactional outbox), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "content"),
        // Bảng `content_jobs` (job queue): hẹn giờ, dọn revision.
        autoxide_kernel::job::jobs_migration("m0003_create_jobs", "content"),
        // Cột cho AI agent: ai tạo bằng API key nào, xin duyệt, nội dung do AI tạo (D-109).
        Box::new(m0004_agent_fields::Migration),
        // Dịch bằng AI: bảng yêu cầu dịch, cột `translated_from_revision_id` (D-109).
        Box::new(m0005_translation_requests::Migration),
        // Tìm theo ý nghĩa: vector của đoạn, số phiên bản để nạp lại (D-109).
        Box::new(m0006_embeddings::Migration),
        // Cột `actor_kind` của bảng `content_events` (người lạ khác hệ thống, D-122).
        autoxide_kernel::event::outbox_actor_migration("m0007_add_event_actor", "content"),
    ]
}
