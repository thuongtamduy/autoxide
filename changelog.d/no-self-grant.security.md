- Managing roles now follows one rule everywhere: you can only give or take away what you have yourself. A role
  you give or take away (`PUT /api/v1/auth/users/{id}/roles`, `PUT /api/v1/auth/users/{id}/memberships`, the MCP tools
  `set_user_roles` and `set_memberships`) must only have permissions you hold as widely, for anyone including
  yourself; taking a role away, or deleting it (`DELETE /api/v1/auth/roles/{id}`, tool `delete_role`), also needs
  every permission the role bans, since lifting a ban widens access. Editing a role (`PUT /api/v1/auth/roles/{id}`,
  tool `replace_role`) can no longer add, widen or unban a permission the editor lacks, nor turn off `requires_mfa`
  of a role the editor could not give. Holding `auth.roles.manage` no longer lets someone make themselves an
  administrator. An administrator with every permission and no API key fence or banned permissions can still do all
  of this; one whose own permissions are limited (a fenced API key, a role that bans something) is held to the
  same rule.
  **Action needed:** people who manage roles without holding everything they hand out may now get 403
  `auth.role_beyond_you` or `auth.permission_beyond_you`; give such changes to an administrator, or give the manager
  the permissions involved.
