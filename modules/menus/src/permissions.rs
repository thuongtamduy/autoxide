//! Permission của module `menus`: `<module>.<việc>`. Service gọi `principal.require(&…)`; tool MCP
//! khai `.requires(…)`. Đọc menu công khai không cần permission.

use autoxide_kernel::access::Permission;

pub(crate) const MENUS_READ: Permission =
    Permission::new("menus.read", "See the website menus in your scope as they are saved.")
        .scoped();

pub(crate) const MENUS_MANAGE: Permission =
    Permission::new("menus.manage", "Create, change and delete the website menus in your scope.")
        .scoped();

pub(crate) fn all() -> Vec<Permission> {
    vec![MENUS_READ, MENUS_MANAGE]
}
