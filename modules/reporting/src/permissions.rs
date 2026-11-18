//! Permission của module `reporting`.

use autoxide_kernel::access::Permission;

// Theo nút (bước 34): độ sâu `tenant` ở khách X thấy số liệu của X và mọi khách cấp dưới của X;
// `subtree` / `unit` ở nút N chỉ thấy số liệu của X thuộc nhánh / nút đó.
pub(crate) const REPORTING_READ: Permission = Permission::new(
    "reporting.read",
    "See report figures: with scope `tenant`, of this tenant and every tenant below it; with a \
     narrower scope, only of your units.",
)
.scoped();

pub(crate) const REPORTING_MANAGE: Permission = Permission::new(
    "reporting.manage",
    "See how far behind the report store is and push the facts of a tenant again.",
);

pub(crate) fn all() -> Vec<Permission> {
    vec![REPORTING_READ, REPORTING_MANAGE]
}
