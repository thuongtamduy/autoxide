- Module `content`: public text search (`GET /api/v1/content/public/posts?q=`, the word part of
  `GET /api/v1/content/public/search`, and content suggestions in the shared search box) uses a real full-text
  index on both databases (PostgreSQL GIN on `tsvector`, SQLite FTS5) instead of scanning every published entry,
  and the database stops a search statement after 2 seconds (`503 content.public_search_timeout`). Search now
  matches whole words, and word prefixes from 3 letters (`lat` finds "Đà Lạt"; `alat` no longer does); it covers
  the 1,000 newest matching entries. Concurrent loads of the same meaning index run once, a public search whose
  meaning part was cut short no longer uses up the tenant's per-minute budget, and the CPU work of a cancelled
  search stops with it.
