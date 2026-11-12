//! Permission của module `webhooks`.

use autoxide_kernel::access::Permission;

pub(crate) const WEBHOOKS_MANAGE: Permission = Permission::new(
    "webhooks.manage",
    "Create, change and delete webhook endpoints and see their deliveries. An endpoint receives \
     every event it subscribes to, including users' personal data: give it to administrators \
     only.",
);

pub(crate) fn all() -> Vec<Permission> {
    vec![WEBHOOKS_MANAGE]
}
