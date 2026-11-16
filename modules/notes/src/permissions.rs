//! Permission của module `notes`: `<module>.<việc>`. Handler REST gọi
//! `principal.require(&…)`; tool MCP khai `.requires(…)`.

use autoxide_kernel::access::Permission;

// Theo nút (bước 34): độ sâu `own` = chỉ ghi chú của mình; `unit` / `subtree` = ghi chú của nút /
// nhánh; `tenant` = mọi ghi chú của khách.
pub(crate) const NOTES_READ: Permission =
    Permission::new("notes.read", "See the notes in your scope (with scope `own`: only your own).")
        .scoped();

pub(crate) const NOTES_WRITE: Permission = Permission::new(
    "notes.write",
    "Create, change and delete the notes in your scope (with scope `own`: only your own).",
)
.scoped();

pub(crate) fn all() -> Vec<Permission> {
    vec![NOTES_READ, NOTES_WRITE]
}
