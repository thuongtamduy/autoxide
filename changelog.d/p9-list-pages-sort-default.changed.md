- **Breaking (pre-1.0, D-20261125-list-pages-kernel):** lists with a `sort` parameter now default `order` by the chosen
  sort — newest first for a time, A→Z / smallest first for a name or size. `GET /api/v1/media?sort=name|size`,
  `GET /api/v1/media/folders?sort=name` and `GET /api/v1/media/tags` (sorted by name by default) used to default to
  Z→A; send `order=desc` to keep the old order. MCP `media.list` follows the same rule.
- OpenAPI: `limit`, `cursor`, `order` and `count` of every list carry their description on the parameter again (shown
  by API explorers); `order` stays a plain `"asc" | "desc"` enum with its default.
