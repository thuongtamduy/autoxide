//! Role: một nhóm permission đặt tên, gán cho người dùng. Người dùng có hợp các
//! permission của mọi role của mình.

mod entity;
mod service;

pub(crate) use service::{
    AdminRoleSeed, Membership, MembershipFilter, MembershipRow, Role, RoleInput, RoleService,
    count_active_admins, count_active_admins_unlocked, grant_admin, last_admin, mfa_pending,
    role_grants, root_roles,
};
