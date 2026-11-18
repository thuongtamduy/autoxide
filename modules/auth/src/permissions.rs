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

pub(crate) const API_KEYS_MANAGE: Permission = Permission::new(
    "auth.api_keys.manage",
    "See the API keys of every user (never their secrets) and revoke any of them, e.g. when a \
     key leaks or its owner leaves. Give it only to administrators.",
)
.with_risk(Risk::Destructive);

pub(crate) const CUSTOMERS_READ: Permission = Permission::new(
    "auth.customers.read",
    "See the customer accounts of the tenant's shop (email, name, status), never their passwords.",
)
.with_risk(Risk::SensitiveData);

pub(crate) const CUSTOMERS_MANAGE: Permission = Permission::new(
    "auth.customers.manage",
    "Rename, disable and enable customer accounts of the tenant's shop.",
);

pub(crate) const INVITATIONS_MANAGE: Permission = Permission::new(
    "auth.invitations.manage",
    "Invite people by email to work at the organization units in your scope, with roles whose \
     permissions you have; see, resend and revoke those invitations.",
)
.scoped();

pub(crate) const SUPPORT_ACCESS_READ: Permission = Permission::new(
    "auth.support_access.read",
    "See every time platform staff were granted access to this tenant's data, and when they used it.",
);

pub(crate) const SUPPORT_ACCESS_REVOKE: Permission = Permission::new(
    "auth.support_access.revoke",
    "Revoke a platform staff support access to this tenant that is still running (not one \
     granted for a request of a competent authority).",
);

pub(crate) const SUPPORT_GRANT: Permission = Permission::new(
    "auth.support.grant",
    "Platform tenant only: grant platform staff time-limited access to a tenant's data for \
     support, with a reason; see and revoke those grants.",
)
.with_risk(Risk::Destructive);

pub(crate) const DELEGATIONS_READ: Permission = Permission::new(
    "auth.delegations.read",
    "See the delegations of this tenant: given to child tenants, and received from parents.",
);

pub(crate) const DELEGATIONS_MANAGE: Permission = Permission::new(
    "auth.delegations.manage",
    "Let holders of one of this tenant's roles work in a child tenant with that role, and \
     revoke it. Whoever holds this extends this tenant's roles to its children.",
)
.with_risk(Risk::Destructive);

pub(crate) const OIDC_READ: Permission = Permission::new(
    "auth.oidc.read",
    "See the sign-in providers of the tenant and how they are set up (never their client secrets).",
);

pub(crate) const OIDC_MANAGE: Permission = Permission::new(
    "auth.oidc.manage",
    "Add, change and remove sign-in providers of the tenant (Google, Microsoft, GitHub, OpenID \
     Connect). Whoever holds this decides who can sign in.",
)
.with_risk(Risk::Destructive);

/// Mọi permission module `auth` khai.
pub(crate) fn all() -> Vec<Permission> {
    vec![
        ROLES_READ,
        ROLES_MANAGE,
        USERS_READ,
        USERS_MANAGE,
        PASSKEYS_MANAGE,
        API_KEYS_MANAGE,
        CUSTOMERS_READ,
        CUSTOMERS_MANAGE,
        DELEGATIONS_READ,
        DELEGATIONS_MANAGE,
        INVITATIONS_MANAGE,
        SUPPORT_ACCESS_READ,
        SUPPORT_ACCESS_REVOKE,
        SUPPORT_GRANT,
        OIDC_READ,
        OIDC_MANAGE,
    ]
}
