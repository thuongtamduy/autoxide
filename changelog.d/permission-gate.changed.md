- The kernel enforces each operation's `x-permissions` before the handler runs: an authenticated caller who has none
  of the listed permissions is refused with 403 `permission_denied`. Modules, including modules built outside this
  repository, must list in `x-permissions` every permission their service accepts for the operation (one of them is
  enough); an operation open to any signed-in user keeps `x-permissions: []`.
- New OpenAPI extension `x-buyers: true`: a buyer account (which has no permissions) may call the operation even
  though it lists `x-permissions`; staff still need one of them. Used by the buyer endpoints of `promotions`.
- The kernel's `x-permissions` enforcement is a change to the module contract and also applies to modules built
  outside this repository: check that every operation of such a module lists each permission its service accepts.
  `HEAD` requests are checked like `GET`, and a caller that cannot be authenticated gets 401 before the request
  body or headers are checked. The per-tenant request limit now runs before the permission check, so refused
  requests count against the tenant's budget.
