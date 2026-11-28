- **Breaking (API):** giving a role beyond your own rights now answers `403` with code
  `auth.role_beyond_you` (as taking one away already did) instead of `422 validation_failed`, on
  `PUT /api/v1/auth/users/{id}/memberships`, `POST /api/v1/auth/invitations`,
  `POST /api/v1/auth/scim/group-mappings` and the default role of `POST` / `PATCH
  /api/v1/auth/oidc-providers…`, and on the MCP tools `set_memberships`, `create_invitation`,
  `create_oidc_provider` and `update_oidc_provider`. The field error `more_than_you_have` (with
  `role`, `role_id`, `org_id`) stays in `errors`. Other input errors still answer `422` first.
