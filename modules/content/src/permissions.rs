//! Permission của module `content` (D-102, D-109).
//!
//! `content.write_own` / `content.publish_own` cho AI agent: chỉ làm trên mục do chính bên gọi
//! tạo — cùng người dùng **và** cùng API key (hai agent dùng hai key của một người không đụng
//! bài của nhau).

use autoxide_kernel::access::{OrgTarget, Permission, Principal, Risk};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};

use crate::entries::Entry;

pub(crate) const CONTENT_READ: Permission =
    Permission::new("content.read", "Read drafts, revisions and schedules of posts and pages.")
        .scoped();
pub(crate) const CONTENT_WRITE: Permission = Permission::new(
    "content.write",
    "Create posts and pages, edit drafts, add translations, restore revisions, make preview links.",
)
.scoped();
pub(crate) const CONTENT_WRITE_OWN: Permission = Permission::new(
    "content.write_own",
    "Like content.write, but only on entries the caller created (same user and same API key). \
     Meant for AI agents.",
)
.scoped();
pub(crate) const CONTENT_PUBLISH: Permission = Permission::new(
    "content.publish",
    "Publish, unpublish, archive and schedule posts and pages; move published pages; change the \
     terms and authors of published entries.",
)
.scoped();
pub(crate) const CONTENT_PUBLISH_OWN: Permission = Permission::new(
    "content.publish_own",
    "Publish and unpublish only entries the caller created (same user and same API key); no \
     archive, delete, move or schedule. Meant for AI agents.",
)
.scoped();
pub(crate) const CONTENT_DELETE: Permission =
    Permission::new("content.delete", "Delete posts and pages for good.")
        .with_risk(Risk::Destructive)
        .scoped();
pub(crate) const CONTENT_MANAGE: Permission =
    Permission::new("content.manage", "Manage categories, tags, authors, menus and redirects.");

pub(crate) const CONTENT_BANNERS: Permission = Permission::new(
    "content.banners",
    "Create, edit, schedule, order and archive banners of display placements, for the \
     organization units in reach.",
)
.scoped();

pub(crate) const CONTENT_THEMES: Permission = Permission::new(
    "content.themes.manage",
    "Change the look of websites (colors, fonts, spacing, logo) for the organization units in reach.",
)
.scoped();

pub(crate) const CONTENT_TYPES: Permission = Permission::new(
    "content.types.manage",
    "Create, change and delete the tenant's own content types (team, projects, branches…).",
)
.scoped();

// Theo nút (bước 34, D-208): đọc, viết, đăng, xoá áp theo độ sâu — người của một cửa hàng chỉ làm
// trên bài của cửa hàng mình. `content.manage` (danh mục, thẻ, tác giả, chuyển hướng) giữ cấp khách.

pub(crate) fn all() -> Vec<Permission> {
    vec![
        CONTENT_READ,
        CONTENT_WRITE,
        CONTENT_WRITE_OWN,
        CONTENT_PUBLISH,
        CONTENT_PUBLISH_OWN,
        CONTENT_DELETE,
        CONTENT_MANAGE,
        CONTENT_BANNERS,
        CONTENT_THEMES,
        CONTENT_TYPES,
    ]
}

/// Mục `entry` do chính `principal` tạo: cùng người dùng và cùng credential uỷ quyền — API key hay grant OAuth
/// (hay cùng không có). Cột `created_by_api_key_id` giữ ID credential đó (D-20261126-mcp-oauth).
pub(crate) fn owns(principal: &Principal, entry: &Entry) -> bool {
    entry.created_by == Some(principal.user_id())
        && entry.created_by_api_key_id == principal.credential_id()
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

/// Việc cần làm trên một mục, để chọn permission khi kiểm phạm vi theo nút.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Need {
    Read,
    Write,
    Publish,
    Delete,
}

/// Đích kiểm quyền của mục: nút sở hữu và người tạo.
pub(crate) fn target(tenant: &TenantContext, entry: &Entry) -> OrgTarget {
    let org = entry.org_id.map_or(root_of(tenant.id()), OrgId::from_uuid);
    OrgTarget::new(org, entry.created_by)
}

/// Permission dùng cho việc `need` của `principal` (bản `_own` khi chỉ có bản đó).
fn permission_for(principal: &Principal, need: Need) -> &'static Permission {
    match need {
        Need::Read => &CONTENT_READ,
        Need::Write if principal.has(&CONTENT_WRITE) => &CONTENT_WRITE,
        Need::Write => &CONTENT_WRITE_OWN,
        Need::Publish if principal.has(&CONTENT_PUBLISH) => &CONTENT_PUBLISH,
        Need::Publish => &CONTENT_PUBLISH_OWN,
        Need::Delete => &CONTENT_DELETE,
    }
}

/// Phạm vi theo nút của `principal` phủ mục `entry` cho việc `need` (bước 34). Gọi SAU kiểm
/// permission thường (`require_write`, `require_publish`…). Không đọc được mục → 404
/// `content.not_found` (không lộ mục tồn tại); đọc được mà không ghi được → 403
/// `org_out_of_scope`. Việc ghi còn từ chối nút đang khoá (`org_frozen`).
pub(crate) async fn require_org(
    principal: &Principal,
    tenant: &TenantContext,
    entry: &Entry,
    need: Need,
) -> Result<(), Error> {
    let at = target(tenant, entry);
    let read = tenant.database().read();
    let hidden = |error: Error| {
        if error.code() == "org_out_of_scope" {
            crate::service::entry_not_found(entry.id)
        } else {
            error
        }
    };
    if need == Need::Read || !principal.has(&CONTENT_READ) {
        // Không có `content.read`: chỉ phạm vi của chính việc này quyết thấy hay không.
        let permission = permission_for(principal, need);
        principal.require_at(read, permission, at).await.map_err(hidden)?;
    } else {
        principal.require_at(read, &CONTENT_READ, at).await.map_err(hidden)?;
    }
    if need == Need::Read {
        return Ok(());
    }
    // Kiểm trên kết nối đọc, không khoá: chỗ gọi có thể đang giữ transaction ghi (SQLite chỉ một
    // kết nối ghi). Tạo mục mới kiểm lại trong transaction (`require_write_at`).
    let permission = permission_for(principal, need);
    principal.require_at(read, permission, at).await?;
    autoxide_kernel::tenancy::OrgTree::ensure_writable_unlocked(read, tenant.id(), at.org).await
}

/// Như [`require_org`] cho việc viết, TRONG transaction ghi `transaction` (tạo mục: nút do người
/// tạo nêu, ngoài phạm vi → 403).
pub(crate) async fn require_write_at<C: sea_orm::ConnectionTrait>(
    principal: &Principal,
    transaction: &C,
    tenant: &TenantContext,
    entry: &Entry,
) -> Result<(), Error> {
    let permission = permission_for(principal, Need::Write);
    principal.require_write_at(transaction, permission, target(tenant, entry)).await
}
