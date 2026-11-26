- New 403 answers where `auth.roles.manage` alone used to be enough: `PUT /api/v1/auth/users/{id}/roles` and
  `PUT /api/v1/auth/users/{id}/memberships` when a role given or taken away is beyond the caller, `DELETE
  /api/v1/auth/roles/{id}` when the role is beyond the caller (`auth.role_beyond_you`), and `PUT
  /api/v1/auth/roles/{id}` when the change adds, widens or unbans a permission the caller lacks
  (`auth.permission_beyond_you`) or turns off `requires_mfa` of a role the caller could not give
  (`auth.role_beyond_you`). Allowed before 1.0 by the compatibility policy (D-20261125-api-break-before-1-0).
