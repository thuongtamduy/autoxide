- **Breaking (pre-1.0, D-20261125-list-pages):** `GET /api/v1/kernel/records/{kind}/{id}/usages` now returns one
  page, `{"data": […], "pagination": {"limit", "next_cursor"}}`, instead of at most `limit` records in `data`. It
  takes the shared `limit` (1–200, default 50), `cursor`, `order` and `count`; follow `pagination.next_cursor` to read
  every record that points at the one asked about, across every module. Order: when the pointing row was created,
  newest first (before: posts by last update, then menu items in no set order).
- `POST /api/v1/compliance/purposes` answers `409` with code `compliance.purpose_limit_reached` once the tenant has
  `AUTOXIDE_COMPLIANCE_PURPOSES_TENANT_MAX` purposes of its own (default 200); changing an existing purpose, or
  overriding a default one, is not limited.
- `GET /api/v1/kernel/custom-fields` without `target`: the subtypes of a record type are matched case-sensitively on
  both databases (on SQLite, a field of `Shop.kind.x` was read, then dropped, for a reader of `shop.kind`, using up
  the rows read per request).
- `GET /api/v1/compliance/documents/{id}/acceptances`, `GET /api/v1/orders/returns` and `GET /api/v1/orders/reviews`
  check the permission before the paging parameters: a caller without it gets `403`, not `422`.
- The server refuses to start when a module declares two retention policies with the same name
  (`Module::retention`), instead of failing `GET /api/v1/compliance/retention` with `500`.
- The 31 remaining MCP list tools (`address.search_units`, `audit.list_entries`, `bookings.list_bookings`,
  `comments.queue`, `comments.unanswered`, `content.list`, `content.list_banners`, `content.list_revisions`,
  `content.list_theme_variants`, `content.list_types`, `conversations.list_callbacks`, `conversations.list_threads`,
  `forms.list`, `forms.list_submissions`, `inventory.get_levels`, `inventory.list_low_stock`,
  `inventory.list_movements`, `invoices.documents`, `ledger.entries`, `ledger.exceptions`, `ledger.holds`,
  `ledger.payouts`, `media.list`, `media.list_tags`, `newsletter.find_subscribers`, `newsletter.list_lists`,
  `reporting.facts`, `shipping.list_cod_discrepancies`, `shipping.list_shipments`, `support.list_tickets`,
  `webhooks.list_deliveries`) take `count` (`true`: also return `pagination.total` on the first page), like REST.
- Guide for API clients: [Upgrading clients: every list is a page](docs/guides/list-pages-upgrade.md)
  ([Vietnamese](docs/guides/list-pages-upgrade.vi.md)).
