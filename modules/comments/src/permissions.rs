//! Permission của module `comments` (đề bài F), cộng `comments.export` của hợp đồng chung §3
//! (xuất / xoá theo email — quyền của chủ dữ liệu, tách khỏi kiểm duyệt).

use autoxide_kernel::access::{Permission, Risk};

pub(crate) const COMMENTS_POST: Permission = Permission::new(
    "comments.post",
    "Comment as a signed-in user, change and delete one's own comments, set one's display name.",
);
pub(crate) const COMMENTS_READ: Permission = Permission::new(
    "comments.read",
    "Read every comment in any status, with author emails, network fingerprints and reports.",
)
.with_risk(Risk::SensitiveData);
pub(crate) const COMMENTS_MODERATE: Permission = Permission::new(
    "comments.moderate",
    "Approve, reject or mark comments as spam, and delete them for good.",
);
pub(crate) const COMMENTS_MANAGE: Permission =
    Permission::new("comments.manage", "Ban and unban people, open, close and hide targets.");
pub(crate) const COMMENTS_EXPORT: Permission = Permission::new(
    "comments.export",
    "Export or erase every comment written with an email address.",
)
.with_risk(Risk::SensitiveData);

pub(crate) fn all() -> Vec<Permission> {
    vec![COMMENTS_POST, COMMENTS_READ, COMMENTS_MODERATE, COMMENTS_MANAGE, COMMENTS_EXPORT]
}
