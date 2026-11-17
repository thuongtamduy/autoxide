//! Role: một nhóm permission đặt tên, gán cho người dùng. Người dùng có hợp các
//! permission của mọi role của mình.

mod entity;
mod service;

pub(crate) use service::{
    AdminRoleSeed, MEMBERS_LIMIT, MemberCount, MemberCounter, Membership, MembershipFilter,
    MembershipRow, Role, RoleInput, RoleService, add_roles, count_active_admins,
    count_active_admins_unlocked, grant_admin, grant_problems, has_membership,
    has_second_factor_condition, holds_mfa_role, last_admin, member_reference, role_by_name,
    role_grants, root_roles,
};
