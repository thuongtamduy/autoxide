- **Breaking (pre-1.0, D-20261125-list-pages):** these `GET` endpoints keep their path but now return one page,
  `{"data": […], "pagination": {"limit", "next_cursor"}}`, instead of the whole list in `data`. They take `limit`
  (1–200, default 50), `cursor` (the `next_cursor` of the previous page), `order` (`asc` / `desc`) and `count=true`
  (adds `pagination.total` on the first page). To adapt, read the items from `data` as before and, while
  `pagination.next_cursor` is not `null`, request again with `cursor=<next_cursor>` and the same filters.
  - In a natural order (`order` defaults to `asc`): `/api/v1/plans/catalog` (by code), `/plans/store/products` (by
    the store's product id; before: by store, then product id), `/plans/usage` and
    `/plans/tenants/{tenant_id}/usage` (by limit key; before: catalog order), `/tax/jurisdictions` (by code),
    `/tax/categories`, `/tax/rates`, `/tax/withholding-rates`, `/tax/rules` (the same order as before),
    `/organizations/public/stores` (by the position the tenant set, then unit id; before: then display name),
    `/organizations/settings` and `/organizations/units/{id}/settings` (by key), `/organizations/unit-types` (by
    code), `/organizations/units/{id}/ancestors` (nearest first, as before), `/analytics/event-types` and
    `/analytics/funnels` (by key), `/me/flags` (by key), `/me/permissions` (by name), `/me/pins` (display order),
    `/notifications/templates` (by kind, then the order the texts were first saved; before: by kind, channel,
    language).
  - Rankings, most first (`order=asc`, the default; `desc` puts the fewest first): `/api/v1/analytics/pages`,
    `/analytics/sources`, `/analytics/campaigns`, `/analytics/types`, `/analytics/objects`. Their `limit` now
    follows the shared rule (1–200, default 50; before 1–100, default 10), and every ranked value can be reached by
    following `next_cursor` (before: only the top `limit`).
  - Newest first (`order` defaults to `desc`): `/api/v1/plans/billing-periods` and
    `/plans/tenants/{tenant_id}/billing-periods` (before: at most 200 rows), `/plans/tenants/{tenant_id}/overrides`
    (before: oldest first), `/plans/store/purchases` (most recently changed first, as before), `/tax/packs` (most
    recently imported first; before: by code, newest version first), `/customers/me/wishlist` (most recently added
    first, as before), `/me/views` (before: at most 500 rows).
  - MCP tools changed the same way (same paging parameters as the endpoint, result `{"data", "pagination"}`):
    `plans.list_usage`, `tax.rates`, `organizations.get_settings`, `analytics.top_pages`,
    `analytics.top_objects` (their `limit` is now 1–200, default 50). Follow `pagination.next_cursor` with the same
    arguments.
- Every paged MCP list tool of these modules now also takes `count` (`true` adds `pagination.total` on the first
  page), like REST: `customers.list_groups`, `customers.search_customers`, `notifications.list_messages`,
  `notifications.list_my_inbox`, `organizations.list_groups`, `organizations.list_units`, `plans.list_plans`,
  `plans.list_signups`, `kernel.list_sandboxes`, `kernel.list_sandbox_messages`, `kernel.request_log`.
- Wishlist items and notification template texts get an internal `id` (UUIDv7) column used by the cursor; existing
  rows get one when the migration runs. The API responses do not show it.
