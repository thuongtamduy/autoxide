- **Breaking:** public content search (`GET /api/v1/content/public/posts?q=`, the word part of
  `GET /api/v1/content/public/search`, content suggestions) matches whole words, ignoring case and accents
  (`ca phe` finds "Cà phê"), and word prefixes from 3 letters (`lat` finds "Đà Lạt"). It no longer matches any
  substring (`alat` finds nothing). It covers the 1,000 newest matching entries; when every word of the query is
  very common, it covers the matches among the 5,000 newest entries. Text written without spaces (Chinese,
  Japanese, Thai…) is one long "word": a run longer than 64 characters is not indexed, and a run is found by its
  beginning.
- `GET /api/v1/content/public/posts?q=` and content suggestions now share the public search limits
  (`AUTOXIDE_CONTENT_PUBLIC_SEARCH_CONCURRENCY`, `AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_ADDRESS_PER_MINUTE`): a list
  with `q` answers `429` past them; suggestions leave out content when the server is busy.
