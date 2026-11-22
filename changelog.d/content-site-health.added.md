- `content`: website health check — `POST /api/v1/content/site-health/{org_id}/run` finds missing SEO
  descriptions, duplicate titles, images without `alt`, broken links inside the site, embedded records that are no
  longer public and pages whose countdown is over, each with how to fix it; `GET /site-health` reads the last check,
  and the work inbox counts the problems (`content.site_health`).
