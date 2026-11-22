- `content`: custom content types (team, projects, branches…) — `/api/v1/content/types` (permission
  `content.types.manage`); their fields are custom fields of target `content.type.<key>`; entries are created with
  `POST /entries` (`kind: "custom"`, `type_id`) and keep drafts, history, translations and SEO; types with pages
  publish at `/<path_prefix>/<slug>`; `GET /public/types/{key}/entries` lists them publicly. Switching a type off
  takes its entries off the website.
- Kernel: `CustomFieldTarget::with_subtypes()` lets a module accept custom fields on record types it creates at run
  time (`<target>.<key>`).
