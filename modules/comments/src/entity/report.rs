//! Bảng `comments_reports`: báo cáo vi phạm; mỗi người báo một bình luận một lần.

use autoxide_kernel::db::{Record, TenantKey};
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

#[derive(Record, Clone, Debug, PartialEq, Eq)]
#[record(table = "comments_reports", scope = tenant)]
pub(crate) struct Model {
    #[record(primary_key)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: TenantKey,
    pub comment_id: Uuid,
    /// `user:<id>`, `ip:<dấu vết IP của khối /24 hay /48>`, `expired:<id báo cáo>` (dấu vết IP quá
    /// hạn) hoặc `deleted:<id báo cáo>` (tài khoản bị xoá); unique cùng `comment_id`.
    pub reporter_key: String,
    pub reporter_user_id: Option<Uuid>,
    /// `spam`, `abuse`, `illegal` hoặc `other`.
    pub reason: String,
    /// Ghi chú của người báo (chỉ người kiểm duyệt xem).
    pub note: Option<String>,
    pub created_at: Timestamp,
}
