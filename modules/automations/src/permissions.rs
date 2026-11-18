//! Permission của module `automations`: `<module>.<việc>`.

use autoxide_kernel::access::Permission;

// Theo nút (bước 34): luật thuộc một nút; người có quyền ở nút / nhánh thấy luật của nút đó.
pub(crate) const AUTOMATIONS_READ: Permission = Permission::new(
    "automations.read",
    "See the automation rules and their runs in your scope, and the triggers and actions you may use.",
)
.scoped();

pub(crate) const AUTOMATIONS_MANAGE: Permission = Permission::new(
    "automations.manage",
    "Create, change, enable, disable and delete automation rules in your scope. A rule's actions run \
     with the current permissions of its author.",
)
.scoped();

pub(crate) const AUTOMATIONS_RUN: Permission = Permission::new(
    "automations.run",
    "Run automation rules of your scope by hand, retry failed runs and decide on steps that wait for \
     approval (deciding also needs the step's own permission).",
)
.scoped();

pub(crate) fn all() -> Vec<Permission> {
    vec![AUTOMATIONS_READ, AUTOMATIONS_MANAGE, AUTOMATIONS_RUN]
}
