//! Permission của module `analytics`: `<module>.<việc>`.

use autoxide_kernel::access::Permission;

// Theo nút (bước 34): xem số liệu của website của nút trong phạm vi.
pub(crate) const ANALYTICS_READ: Permission = Permission::new(
    "analytics.read",
    "See the visit reports (pages, sources, campaigns, objects, funnels) of the websites in your \
     scope.",
)
.scoped();

pub(crate) fn all() -> Vec<Permission> {
    vec![ANALYTICS_READ]
}
