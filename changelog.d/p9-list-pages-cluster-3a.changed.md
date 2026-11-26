- **Breaking (pre-1.0, D-20261125-list-pages):** these `GET` endpoints of `auth` and the kernel keep their path but
  now return one page, `{"data": […], "pagination": {"limit", "next_cursor"}}`, instead of the whole list in `data`.
  They take `limit` (1–200, default 50), `cursor` (the `next_cursor` of the previous page), `order` (`asc` / `desc`)
  and `count=true` (adds `pagination.total` on the first page). To adapt, read the items from `data` as before and,
  while `pagination.next_cursor` is not `null`, request again with `cursor=<next_cursor>` and the same filters.
  - In a natural order (`order` defaults to `asc`): `/api/v1/auth/permissions` (by name), `/auth/users/{id}/roles`
    (by name), `/auth/users/{id}/memberships` and `/auth/me/memberships` (by unit id), `/auth/memberships/counts`
    (the root unit first, then by unit id), `/auth/me/tenants` (your own tenant, then delegations, then support
    grants; within each by tenant id), `/auth/delegations/tenants` (level by level, each level by slug),
    `/kernel/blocks` (by name), `/kernel/connections/webhooks` (by path), `/kernel/custom-fields` (by `position`,
    then id; before: then key), `/kernel/modules` (start order), `/kernel/placements` (by name),
    `/kernel/reasons/{list}` (by position, then code), `/kernel/records/types` (by name), `/kernel/sandboxes/promotable`
    (by kind), `/kernel/settings` and `/kernel/tenants/{id}/settings` (by name), `/kernel/sync/resources` (by name),
    `/kernel/tags` (by name ignoring case and accents), `/kernel/texts` (by key), `/kernel/whats-new` (newest version
    first). `/kernel/whats-new` without `since` now lists every version (before: the five newest): send `limit=5` for
    the old answer.
  - Newest first (`order` defaults to `desc`): `/api/v1/auth/delegations` (both directions), `/auth/me/identities`
    (before: oldest first), `/auth/oidc-providers` (before: oldest first), `/auth/oidc/providers` (before: oldest
    first), `/auth/roles/counts` (in the order of `/auth/roles`; before: by role id, oldest first),
    `/auth/scim/group-mappings` (before: oldest first), `/auth/scim/tokens`, `/auth/support-access`,
    `/auth/support-grants` (before: at most 200 rows without `tenant_id`), `/kernel/approvals` (before: at most 200),
    `/kernel/jobs`, `/kernel/placements/migrations` (before: at most 50), `/kernel/tenants/{id}/exports` (before: at
    most 50), `/kernel/records/{kind}/{id}/notes` (before: at most 200), `/kernel/records/{kind}/{id}/activity` (by
    when each line was recorded; before: by `at`), `/kernel/devtools/events`.
  - `limit` of `/kernel/jobs`, `/kernel/records/{kind}/{id}/activity` and `/kernel/devtools/events` is now the shared
    paging parameter (same range 1–200, default 50); an invalid value is reported with the other problems of the
    request in one `422`.
  - MCP tools changed the same way (same paging parameters as the endpoint, result `{"data", "pagination"}`):
    `auth.get_user_roles`, `auth.list_permissions`, `auth.count_memberships`, `auth.count_role_members`,
    `auth.list_oidc_providers`, `auth.list_my_identities`, `kernel.list_approvals`, `kernel.list_connection_webhooks`,
    `kernel.list_custom_fields`, `kernel.list_modules`, `kernel.list_promotable`, `kernel.list_settings`. Follow
    `pagination.next_cursor` with the same arguments.
- `auth.list_users`, `auth.list_invitations` and `auth.list_memberships` (MCP) now also take `count` (`true` adds
  `pagination.total` on the first page), like REST.
