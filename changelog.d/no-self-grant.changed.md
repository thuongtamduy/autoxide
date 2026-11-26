- `PUT /api/v1/auth/users/{id}/roles` and `PUT /api/v1/auth/roles/{id}` answer 403 (`auth.role_beyond_you`,
  `auth.permission_beyond_you`) when the change gives, takes away or unbans permissions the caller does not have; before,
  `auth.roles.manage` alone was enough. Allowed before 1.0 by the compatibility policy (D-20261125-api-break-before-1-0).
