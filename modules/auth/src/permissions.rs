//! Permission của module `auth`.

use autoxide_kernel::access::{Permission, Risk};

pub(crate) const ROLES_READ: Permission = Permission::new(
    "auth.roles.read",
    "See roles, the permissions they grant, and the catalog of every permission.",
);

pub(crate) const ROLES_MANAGE: Permission = Permission::new(
    "auth.roles.manage",
    "Create, change and delete roles, and assign roles to users. Whoever holds this can grant \
     any permission, including to themselves: give it to administrators only.",
);

pub(crate) const USERS_READ: Permission =
    Permission::new("auth.users.read", "See user accounts and the roles assigned to them.")
        .with_risk(Risk::SensitiveData);

pub(crate) const USERS_MANAGE: Permission = Permission::new(
    "auth.users.manage",
    "Create user accounts, rename them, disable or enable them, and set the first password \
     of an account created without one.",
);

pub(crate) const PASSKEYS_MANAGE: Permission = Permission::new(
    "auth.passkeys.manage",
    "See and remove the passkeys of other users. Give it only to people who handle account \
     recovery.",
)
.with_risk(Risk::Destructive);

/// Mọi permission module `auth` khai.
pub(crate) fn all() -> Vec<Permission> {
    vec![ROLES_READ, ROLES_MANAGE, USERS_READ, USERS_MANAGE, PASSKEYS_MANAGE]
}
