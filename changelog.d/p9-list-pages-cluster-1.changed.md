- **Breaking (pre-1.0, D-20261125-list-pages):** these `GET` endpoints keep their path but now return one page,
  `{"data": […], "pagination": {"limit", "next_cursor"}}`, instead of the whole list in `data`. They take `limit`
  (1–200, default 50), `cursor` (the `next_cursor` of the previous page), `order` (`asc` / `desc`) and `count=true`
  (adds `pagination.total` on the first page). To adapt, read the items from `data` as before and, while
  `pagination.next_cursor` is not `null`, request again with `cursor=<next_cursor>` and the same filters.
  - In a natural order (`order` defaults to `asc`): `/api/v1/address/provinces` and
    `/address/provinces/{code}/communes` (by code), `/automations/templates` (by id), `/bookings/exceptions` (by
    start time), `/bookings/public/services` (by name), `/content/menus` and `/content/placements` (by key),
    `/content/templates` (by key; before: file order), `/inventory/reports/valuation` (by warehouse id, then
    currency), `/invoices/export-profiles` (by name), `/notifications/kinds` and `/notifications/me/preferences`
    (by kind), `/reporting/kinds` (by name), `/reporting/lag` (by tenant id), `/webhooks/events` (by name).
  - Newest first (`order` defaults to `desc`): `/api/v1/audit/activity`, `/content/entries/{id}/schedules`,
    `/content/public/feed`, `/conversations/threads/{id}/calls`, `/customers/me/gift-cards`,
    `/customers/me/recently-viewed`, `/customers/reminder-rules`, `/customers/referral-rules`,
    `/notifications/me/device-tokens`, `/notifications/templates/{kind}/{channel}/{locale}/versions`.
  - `/api/v1/audit/activity` and `/content/public/feed` already took `limit`: it now follows the shared rule
    (1–200, default 50; before 1–100, default 20).
  - MCP tools changed the same way (same parameters, result `{"data", "pagination"}`): `address.list_provinces`,
    `address.list_communes`, `content.legal_templates`, `conversations.list_calls`, `notifications.list_kinds`,
    `notifications.list_my_devices`, `reporting.kinds`. Follow `pagination.next_cursor` with the same arguments.
