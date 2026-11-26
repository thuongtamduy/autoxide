- **Breaking (pre-1.0, D-20261125-list-pages):** these `GET` endpoints of `catalog`, `compliance`, `orders` and
  `promotions` keep their path but now return one page, `{"data": […], "pagination": {"limit", "next_cursor"}}`,
  instead of the whole list in `data`. They take `limit` (1–200, default 50), `cursor` (the `next_cursor` of the
  previous page), `order` (`asc` / `desc`) and `count=true` (adds `pagination.total` on the first page). To adapt,
  read the items from `data` as before and, while `pagination.next_cursor` is not `null`, request again with
  `cursor=<next_cursor>` and the same filters.
  - In a natural order (`order` defaults to `asc`): `/api/v1/catalog/attributes` (by code), `/catalog/categories`
    and `/catalog/public/categories` (by depth, position, then id: parents before children),
    `/catalog/categories/{id}/attributes`, `/catalog/products/{id}/attributes` and
    `/catalog/variants/{id}/attributes` (in the order of the last `PUT`; before: by effective date for categories),
    `/catalog/restriction-rules` (by jurisdiction, code, effective date), `/catalog/products/counts` (one row per
    key, in the fixed order of the keys), `/compliance/documents` (by kind, then id),
    `/compliance/documents/{id}/acceptance-stats` (by version), `/compliance/me/pending-acceptances` (by kind),
    `/compliance/me/consents` (by purpose, then id), `/compliance/rules` (by jurisdiction, key, newest effective
    version first), `/compliance/policies`, `/compliance/purposes`, `/compliance/public/purposes` and
    `/compliance/tracking-providers` (by key), `/compliance/retention` (by module, then name),
    `/orders/risk/cod-blocks` (expiring soonest first), `/orders/reports/sales` (by shop, channel or product: biggest
    `gross` first, then by `key`; by day or month: oldest first), `/promotions/legal-limits` (by promotion kind,
    program kind, jurisdiction), `/promotions/notice-rules` (by promotion kind, jurisdiction),
    `/promotions/promotions/{id}/stats` (oldest day first), `/promotions/promotions/{id}/versions` (by version),
    `/promotions/public/promotions/for-product/{product_id}` (cheapest first).
  - Short ranked lists, best first (`order=asc`, the default): `/api/v1/catalog/public/products/{id}/similar` (at
    most 12), `/catalog/public/products/{id}/frequently-bought-together` (at most 20) and
    `/catalog/public/search/suggest` (at most 10). Their `limit` now follows the shared rule (1–200, default 50;
    before 1–12 default 8, and 1–10 default 10): send `limit=8` to keep a product page as it was.
  - Newest first (`order` defaults to `desc`): `/api/v1/catalog/orders/{id}/downloads`,
    `/catalog/products/{id}/documents`, `/catalog/rounding-rules`, `/catalog/variants/{id}/digital-assets`,
    `/orders/me/orders/{id}/documents`, `/orders/public/orders/{id}/documents`, `/orders/me/warranties`,
    `/orders/warranties`, `/orders/risk/cod-rules`, `/orders/{id}/refunds`,
    `/promotions/promotions/{id}/conflicts`, `/promotions/promotions/{id}/participations` (before: oldest first
    for the documents, digital assets, downloads, product documents, rounding rules, refunds and participations).
  - MCP tools changed the same way (same paging parameters as the endpoint, result `{"data", "pagination"}`):
    `catalog.list_categories`, `compliance.documents`, `compliance.pending_acceptances`, `compliance.purposes`,
    `compliance.retention`, `orders.lookup_warranty`. Follow `pagination.next_cursor` with the same arguments.
- Every paged MCP list tool of these modules now also takes `count` (`true` adds `pagination.total` on the first
  page), like REST: `catalog.list_products`, `catalog.list_collections`, `compliance.incidents`,
  `compliance.kyc_profiles`, `compliance.legal_profiles`, `compliance.list_cases`, `compliance.subject_requests`,
  `compliance.tracking_tags`, `compliance.websites`, `orders.list_orders`, `orders.list_returns`,
  `promotions.list_promotions`.
- A list whose filter and paging parameters are both wrong (`/promotions/promotions/{id}/stats`,
  `/promotions/promotions/calendar`, `/orders/reports/sales`, `/catalog/public/search/suggest`) answers one `422`
  naming every field.
