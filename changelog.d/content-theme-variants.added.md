- `content`: scheduled theme variants — override part of a website's theme for a holiday, a season or an hour
  (`/api/v1/content/theme-variants`, schedules `once` / `daily` / `weekly` in the website's time zone, priority,
  draft → active → paused, copy to next year), nine ready-made presets (`GET /theme-presets`: Tết, Valentine, 8/3,
  30/4, Mid-Autumn, 20/10, Black Friday, Christmas, golden hour) and `GET /site/{org_id}/preview?at=` to see the
  website's look at any moment. New setting `AUTOXIDE_CONTENT_THEME_VARIANTS_MAX`.
