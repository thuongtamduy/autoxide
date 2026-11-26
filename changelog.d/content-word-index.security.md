- Module `content`: public text search (`GET /api/v1/content/public/posts?q=`, the word part of
  `GET /api/v1/content/public/search`, and content suggestions in the shared search box) uses a real full-text
  index on both databases (PostgreSQL GIN on `tsvector`, SQLite FTS5) instead of scanning every published entry.
  A search statement is stopped after 2 seconds (`503 content.public_search_timeout`).
- Content search also loads each meaning index once when several searches ask for it at the same time.
- A public search whose meaning part was cut short no longer uses up the tenant's per-minute budget.
- The CPU work of a cancelled search stops with it.
