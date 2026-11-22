- `content`: website header and footer as block layouts with draft and publish (`/api/v1/content/site-regions`),
  per language, inherited down the organization tree, optionally per theme variant; and
  `GET /api/v1/content/public/site` — one public call for what every page shares (theme tokens with the running
  variant, decoration, header, footer, legal footer, other modules' parts), cached until the theme changes next.
  Modules plug their part in through `content::api::SiteExtensions`.
