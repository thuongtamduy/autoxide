//! Permission của module `content` (D-102).

use autoxide_kernel::access::Permission;

pub(crate) const CONTENT_READ: Permission =
    Permission::new("content.read", "Read drafts, revisions and schedules of posts and pages.");
pub(crate) const CONTENT_WRITE: Permission = Permission::new(
    "content.write",
    "Create posts and pages, edit drafts, add translations, restore revisions, make preview links.",
);
pub(crate) const CONTENT_PUBLISH: Permission = Permission::new(
    "content.publish",
    "Publish, unpublish, archive, delete and schedule posts and pages; move published pages; \
     change the terms and authors of published entries.",
);
pub(crate) const CONTENT_MANAGE: Permission =
    Permission::new("content.manage", "Manage categories, tags, authors, menus and redirects.");

pub(crate) fn all() -> Vec<Permission> {
    vec![CONTENT_READ, CONTENT_WRITE, CONTENT_PUBLISH, CONTENT_MANAGE]
}
