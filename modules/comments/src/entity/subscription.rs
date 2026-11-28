//! Bảng `comments_reply_subscriptions`: email đã xác minh muốn nhận mail khi bình luận có trả
//! lời. Token huỷ của từng mail nằm ở `comments_tokens` (mục đích `unsubscribe`, D-130 của
//! `newsletter`: token riêng cho từng mail, chỉ lưu bản băm).

use autoxide_kernel::db::{Record, TenantKey};
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

#[derive(Record, Clone, Debug, PartialEq, Eq)]
#[record(table = "comments_reply_subscriptions", scope = tenant)]
pub(crate) struct Model {
    #[record(primary_key)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: TenantKey,
    /// Bình luận được theo dõi.
    pub comment_id: Uuid,
    /// Email đã xác minh (chữ thường).
    pub email: String,
    pub created_at: Timestamp,
}
