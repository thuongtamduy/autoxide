- **Breaking:** public content search (`GET /api/v1/content/public/posts?q=`, the word part of
  `GET /api/v1/content/public/search`, content suggestions) matches whole words, ignoring case and accents
  (`ca phe` finds "Cà phê"), and word prefixes from 3 letters (`lat` finds "Đà Lạt"). It no longer matches any
  substring (`alat` finds nothing). It returns the 1,000 newest matching entries, after the other filters (website,
  category, author); a search that takes longer than 2 seconds answers `503` rather than a partial list. Text
  written without spaces (Chinese, Japanese, Thai…) is one long "word": a run longer than 64 characters is not
  indexed, and a run is found by its beginning.
- `GET /api/v1/content/public/posts?q=` now shares the public search limits
  (`AUTOXIDE_CONTENT_PUBLIC_SEARCH_CONCURRENCY`, `AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_ADDRESS_PER_MINUTE`) and
  answers `429` past them. Content suggestions have their own count of searches running at the same time (same
  size); past it, suggestions leave out posts and pages.
