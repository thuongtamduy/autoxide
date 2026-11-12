//! Permission của module `content` (D-102, D-109).
//!
//! `content.write_own` / `content.publish_own` cho AI agent: chỉ làm trên mục do chính bên gọi
//! tạo — cùng người dùng **và** cùng API key (hai agent dùng hai key của một người không đụng
//! bài của nhau).

use autoxide_kernel::access::{Permission, Principal};
use autoxide_kernel::error::Error;

use crate::entries::Entry;

pub(crate) const CONTENT_READ: Permission =
    Permission::new("content.read", "Read drafts, revisions and schedules of posts and pages.");
pub(crate) const CONTENT_WRITE: Permission = Permission::new(
    "content.write",
    "Create posts and pages, edit drafts, add translations, restore revisions, make preview links.",
);
pub(crate) const CONTENT_WRITE_OWN: Permission = Permission::new(
    "content.write_own",
    "Like content.write, but only on entries the caller created (same user and same API key). \
     Meant for AI agents.",
);
pub(crate) const CONTENT_PUBLISH: Permission = Permission::new(
    "content.publish",
    "Publish, unpublish, archive and schedule posts and pages; move published pages; change the \
     terms and authors of published entries.",
);
pub(crate) const CONTENT_PUBLISH_OWN: Permission = Permission::new(
    "content.publish_own",
    "Publish and unpublish only entries the caller created (same user and same API key); no \
     archive, delete, move or schedule. Meant for AI agents.",
);
pub(crate) const CONTENT_DELETE: Permission =
    Permission::new("content.delete", "Delete posts and pages for good.");
pub(crate) const CONTENT_MANAGE: Permission =
    Permission::new("content.manage", "Manage categories, tags, authors, menus and redirects.");

pub(crate) fn all() -> Vec<Permission> {
    vec![
        CONTENT_READ,
        CONTENT_WRITE,
        CONTENT_WRITE_OWN,
        CONTENT_PUBLISH,
        CONTENT_PUBLISH_OWN,
        CONTENT_DELETE,
        CONTENT_MANAGE,
    ]
}

/// Mục `entry` do chính `principal` tạo: cùng người dùng và cùng API key (hay cùng không có).
pub(crate) fn owns(principal: &Principal, entry: &Entry) -> bool {
    entry.created_by == Some(principal.user_id())
        && entry.created_by_api_key_id == principal.api_key_id()
}

/// Có `content.write` hoặc `content.write_own` không (được tạo mục mới; kiểm trước khi đọc mục
/// để người không có quyền nhận 403 chứ không phải 404). Thiếu cả hai: lỗi nêu `content.write`.
pub(crate) fn require_create(principal: &Principal) -> Result<(), Error> {
    if principal.has(&CONTENT_WRITE_OWN) { Ok(()) } else { principal.require(&CONTENT_WRITE) }
}

/// Có `content.publish` hoặc `content.publish_own` không (kiểm trước khi đọc mục).
pub(crate) fn require_some_publish(principal: &Principal) -> Result<(), Error> {
    if principal.has(&CONTENT_PUBLISH_OWN) { Ok(()) } else { principal.require(&CONTENT_PUBLISH) }
}

/// Được sửa `entry` không: `content.write`, hoặc `content.write_own` trên mục của mình.
pub(crate) fn require_write(principal: &Principal, entry: &Entry) -> Result<(), Error> {
    scoped(principal, entry, &CONTENT_WRITE, &CONTENT_WRITE_OWN)
}

/// Được đăng / gỡ `entry` không: `content.publish`, hoặc `content.publish_own` trên mục của
/// mình.
pub(crate) fn require_publish(principal: &Principal, entry: &Entry) -> Result<(), Error> {
    scoped(principal, entry, &CONTENT_PUBLISH, &CONTENT_PUBLISH_OWN)
}

fn scoped(
    principal: &Principal,
    entry: &Entry,
    any: &Permission,
    own: &Permission,
) -> Result<(), Error> {
    if principal.has(any) {
        return Ok(());
    }
    if principal.has(own) {
        return if owns(principal, entry) { Ok(()) } else { Err(not_owner()) };
    }
    principal.require(any)
}

/// Quyền viết của `principal` chỉ là `content.write_own` (không có `content.write`). Gọi sau
/// `require_create`/`require_write`. Người như vậy (thường là AI agent) bị giới hạn thêm: mục
/// luôn mang `ai_generated`, trang cha phải là của mình, chỉ gắn hồ sơ tác giả của chính mình.
pub(crate) fn write_own_only(principal: &Principal) -> bool {
    !principal.has(&CONTENT_WRITE)
}

/// Quyền đăng của `principal` chỉ là `content.publish_own` (không có `content.publish`). Người
/// như vậy không đăng lại mục đã lưu trữ, không tự đặt `published_at`, không đăng đè lên chuyển
/// hướng làm tay.
pub(crate) fn publish_own_only(principal: &Principal) -> bool {
    !principal.has(&CONTENT_PUBLISH)
}

/// Được đặt mục dưới trang cha `parent` không: có `content.write`, hoặc trang cha là của mình.
/// Không thì trang con của người chỉ có `content.write_own` sẽ chặn chủ trang cha gỡ hay xoá nó.
pub(crate) fn require_parent(principal: &Principal, parent: &Entry) -> Result<(), Error> {
    if !write_own_only(principal) || owns(principal, parent) { Ok(()) } else { Err(not_owner()) }
}

pub(crate) fn not_owner() -> Error {
    Error::forbidden(
        "content.not_owner",
        "This entry was created by someone else (or with another API key); your permission only \
         covers entries you created.",
    )
}
