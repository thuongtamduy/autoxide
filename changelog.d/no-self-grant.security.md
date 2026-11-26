- `PUT /api/v1/auth/users/{id}/roles` (and the MCP tool `set_user_roles`) now follow the same rule as memberships,
  invitations and SCIM: a role you give or take away must only have permissions you have yourself, as widely. This
  applies to yourself too, so holding `auth.roles.manage` no longer lets someone make themselves an administrator.
  Editing a role (`PUT /api/v1/auth/roles/{id}`, tool `replace_role`) can no longer add, widen or unban a permission
  that the editor does not have. Administrators (role `admin`) are not affected.
  **Action needed:** people who manage roles without being administrators may now get 403
  `auth.role_beyond_you` or `auth.permission_beyond_you`; give such changes to an administrator, or give the manager
  the permissions they hand out.
