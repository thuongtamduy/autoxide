//! Permission của module `notes`: `<module>.<việc>`. Handler REST gọi
//! `principal.require(&…)`; tool MCP khai `.requires(…)`.

use autoxide_kernel::access::Permission;

pub(crate) const NOTES_READ: Permission = Permission::new("notes.read", "See your own notes.");

pub(crate) const NOTES_WRITE: Permission =
    Permission::new("notes.write", "Create, change and delete your own notes.");

pub(crate) fn all() -> Vec<Permission> {
    vec![NOTES_READ, NOTES_WRITE]
}
