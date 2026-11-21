- Module `catalog`: digital products are delivered as download links. Attach a file uploaded through
  `files` to a digital variant (`POST /variants/{id}/digital-assets`); a paid order gives each line a
  download right with a limit and an expiry, revoked when the order is cancelled or refunded. Buyers
  spend a download with `POST /public/downloads/{token}` (or `POST /downloads/{id}` when signed in)
  and get a short-lived signed file address that accepts `Range`. Staff list, revoke and reissue the
  rights of an order. New settings `AUTOXIDE_CATALOG_DOWNLOAD_LINK_SECONDS`,
  `AUTOXIDE_CATALOG_DEFAULT_DOWNLOAD_LIMIT`, `AUTOXIDE_CATALOG_DEFAULT_GRANT_DAYS`.
- Module `files`: other modules can read an uploaded file through the new `Files` trait, with the
  same viewing rules as `/api/v1/files`.
