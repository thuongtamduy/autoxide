//! Role: một nhóm permission đặt tên, gán cho người dùng. Người dùng có hợp các
//! permission của mọi role của mình.

pub(crate) mod entity;
mod service;

pub(crate) use service::{
    AdminRoleSeed, MEMBERS_LIMIT, MemberCount, MemberCounter, Membership, MembershipFilter,
    MembershipRow, Role, RoleInput, RoleService, active_admin_ids, add_roles, count_active_admins,
    count_active_admins_unlocked, forget_scim_grants, grant_admin, grant_problems, has_membership,
    has_second_factor_condition, holds_any_role, holds_mfa_role, last_admin, member_reference,
    passkey_required, passkey_required_error, require_grantable, role_by_name, role_grants,
    roles_blocked_by_mfa, root_roles,
};
