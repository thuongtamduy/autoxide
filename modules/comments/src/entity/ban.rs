//! Bảng `comments_bans`: cấm bình luận theo người dùng, email hay dấu vết IP.

use autoxide_kernel::db::{Record, TenantKey};
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

#[derive(Record, Clone, Debug, PartialEq, Eq)]
#[record(table = "comments_bans", scope = tenant)]
pub(crate) struct Model {
    #[record(primary_key)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: TenantKey,
    /// `user`, `email` hoặc `ip`.
    pub kind: String,
    /// ID người dùng, email (chữ thường) hay `ip_fingerprint`.
    pub value: String,
    pub reason: String,
    /// Hết cấm lúc này; `None`: vĩnh viễn.
    pub until: Option<Timestamp>,
    pub created_by: Option<Uuid>,
    pub created_at: Timestamp,
}
