//! Bảng `comments_targets`: trạng thái và số đếm của một đích. Khoá chính (`tenant_id`,
//! `target_type`, `target_id`); dòng tạo khi có bình luận đầu tiên (hoặc khi quản trị viên đặt
//! trạng thái).

use autoxide_kernel::db::{Record, TenantKey};
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

#[derive(Record, Clone, Debug, PartialEq, Eq)]
#[record(table = "comments_targets", scope = tenant)]
pub(crate) struct Model {
    /// Khách sở hữu dòng (bước 33), cột đầu của khoá chính.
    pub tenant_id: TenantKey,
    pub target_type: String,
    pub target_id: String,
    /// UUIDv7 cấp khi tạo (dòng cũ: cấp ở migration `m0012`), cho cursor của danh sách (D-270);
    /// unique trong khách (`comments_targets_uid_key`).
    pub uid: Option<Uuid>,
    /// `open` hoặc `closed`.
    pub state: String,
    /// Ẩn mọi bình luận khỏi phần công khai (vd đích đã bị xoá).
    pub hidden: bool,
    /// Tự đóng từ lúc này.
    pub closes_at: Option<Timestamp>,
    /// Số bình luận đang hiện.
    pub published_count: i32,
    /// Số bình luận chờ duyệt.
    pub pending_count: i32,
    /// Số bình luận gốc (câu hỏi) đang hiện (D-202).
    pub published_thread_count: i32,
    /// Số bình luận gốc chờ duyệt.
    pub pending_thread_count: i32,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
