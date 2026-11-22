- `content`: website themes — `GET` / `PUT /api/v1/content/themes/{org_id}` set colors, fonts, spacing, radius and
  logo as flat tokens per organization unit; a store inherits its chain's tokens and overrides only what it sets.
  `effective` gives every token the website uses; colors below WCAG AA contrast come back as `warnings`. New
  permission `content.themes.manage`.
