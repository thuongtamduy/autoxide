- `GET /api/v1/kernel/whats-new` without `since`: before, the five newest versions; now every version, one page at a
  time — send `limit=5` for the old answer.
- `GET /api/v1/auth/me/identities`, `/auth/oidc-providers`, `/auth/oidc/providers`, `/auth/scim/group-mappings`:
  before, oldest first; now newest first (`order=asc` for the old order).
- `GET /api/v1/auth/roles/counts`: before, by role id (oldest first); now in the order of `/auth/roles`.
- `GET /api/v1/kernel/records/{kind}/{id}/activity`: before, ordered by `at`; now by when each line was recorded.
- `GET /api/v1/kernel/custom-fields`: fields with the same `position` were ordered by key; now by id.
- `limit` of `GET /api/v1/kernel/jobs`, `/kernel/records/{kind}/{id}/activity` and `/kernel/devtools/events`: before,
  each endpoint's own parameter; now the shared paging parameter (1–200, default 50), with `cursor` for the next page.
- `GET /api/v1/auth/support-grants`, `/kernel/approvals`, `/kernel/placements/migrations`, `/kernel/tenants/{id}/exports`
  and `/kernel/records/{kind}/{id}/notes`: before, silently cut at 50 or 200 rows; now every row, page by page.
- `GET /api/v1/kernel/approvals` and `/kernel/custom-fields` (and the MCP tools `kernel.list_approvals`,
  `kernel.list_custom_fields`) read a bounded number of rows per request: a page may hold fewer than `limit` items,
  even none, while `pagination.next_cursor` is set — keep following the cursor until it is `null`. With `count=true`,
  `pagination.total` is left out when the list is too long to count in one request.
- `PUT /api/v1/kernel/reasons/{list}/{code}`: a tenant adds at most `AUTOXIDE_REASONS_TENANT_MAX` reasons of its own to
  one list (default 200); one more is refused with `409 kernel.reason_limit_reached`.
- Upgrading runs migration `auth` m0047: a membership row without a unit (`org_id` empty, from old or hand-edited data)
  is attached to the root unit, where it already granted its role; duplicates of an existing root membership are
  removed.
