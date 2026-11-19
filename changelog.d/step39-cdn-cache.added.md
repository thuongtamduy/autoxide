- Caching by a CDN (step 39): public reads of a website now say `public, max-age=0, s-maxage=60,
  stale-while-revalidate=300` with an `ETag` and `Vary: Accept-Language, Accept-Encoding`, so a CDN keeps them for
  `AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS` (default 60; `0` to always ask again). A request with `Authorization` or a
  cookie never gets a `public` response. `docs/DEPLOY.md` explains how to set up the CDN.
