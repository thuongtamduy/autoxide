- Public content search (`GET /api/v1/content/public/search`) can no longer slow down the whole server: each
  server process runs at most `AUTOXIDE_CONTENT_PUBLIC_SEARCH_CONCURRENCY` (default 8) public searches at once
  (`429 content.public_search_busy` past it), one client address may search
  `AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_ADDRESS_PER_MINUTE` times a minute (default 20,
  `429 content.public_search_rate_limited`), and a search is stopped after 3 seconds
  (`503 content.public_search_timeout`).
