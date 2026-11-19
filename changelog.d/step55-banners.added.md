- Module `content`: display placements and banners for storefronts (step 55, D-236). A placement
  (`/api/v1/content/placements`, `content.manage`) says how the website draws it (`single`,
  `carousel`, `grid`, `stack`, `strip`, `popup`), which banners and picture sizes it takes, on which
  devices and kinds of pages; new tenants get a standard set (`POST /placements/presets` adds it to
  existing ones). Banners (`/api/v1/content/banners`, new permission `content.banners` per
  organization unit) have a time window, a fixed order (`PUT /placements/{key}/order` with
  `If-Match`), links to an address or to any object of the object directory, and can be limited to
  some pages. `GET /api/v1/content/public/placements?keys=` reads every placement of a page in one
  call, with `max-age` up to the next banner start or end. Banners linking to a promotion are
  archived when it ends. MCP tools `content.list_banners`, `content.save_banner` (drafts only),
  `content.order_placement`.
