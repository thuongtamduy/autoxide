- Public content search (`GET /api/v1/content/public/search`) can no longer slow down the whole server: each
  server process runs at most `AUTOXIDE_CONTENT_PUBLIC_SEARCH_CONCURRENCY` (default 8) public searches at once
  (`429 content.public_search_busy` past it; a search keeps its place until its work has really finished), one
  client address may search `AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_ADDRESS_PER_MINUTE` times a minute (default 20,
  `429 content.public_search_rate_limited`; a refusal because the server is busy does not count), and a search
  that has not answered after 3 seconds gets `503 content.public_search_timeout`. When the embeddings provider is
  slow, search by meaning is skipped after 1.5 seconds and the results match words only.
