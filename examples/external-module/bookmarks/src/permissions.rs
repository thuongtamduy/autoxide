//! Permissions of the module: `<module>.<action>`. REST handlers call `principal.require(&…)`,
//! MCP tools declare `.requires(…)`. Access is checked by permission, never by role name.

use autoxide_kernel::access::Permission;

pub(crate) const BOOKMARKS_READ: Permission =
    Permission::new("bookmarks.read", "See your own bookmarks.");

pub(crate) const BOOKMARKS_WRITE: Permission =
    Permission::new("bookmarks.write", "Create and delete your own bookmarks.");

pub(crate) fn all() -> Vec<Permission> {
    vec![BOOKMARKS_READ, BOOKMARKS_WRITE]
}
