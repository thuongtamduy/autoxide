- `content`: website health checks now run every day for every website, and count missing addresses asked through
  `GET /public/resolve` (per day, nothing stored about the visitor) to suggest redirects (`not_found_path`, setting
  `AUTOXIDE_CONTENT_SITE_HEALTH_NOT_FOUND_MIN_HITS`). New MCP tools for the page builder: `content.list_blocks`,
  `get_page_layout`, `update_page_layout`, `list_theme_variants`, `save_theme_variant` (always a draft),
  `list_types`, `save_type`, `compose_page`, `edit_page`, `site_health`.
