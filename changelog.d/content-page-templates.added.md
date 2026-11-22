- `content`: page templates — `POST /api/v1/content/page-templates/presets` installs ready-made layouts for retail,
  food and drink, services, events and recruiting (English and Vietnamese); `POST /page-templates` saves a page as
  a template; `POST /entries/from-template` starts a draft page from one, leaving out blocks of modules the tenant
  does not have (`dropped_blocks`).
