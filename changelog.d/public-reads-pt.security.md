- Module `content`: public categories and tags (`GET /api/v1/content/public/terms` and
  `GET /api/v1/content/public/terms/{taxonomy}/{slug}`) only show a term that at least one entry shown on the
  website carries — not one used only by drafts, by entries hidden while a report is handled, or by another
  website of the tenant (PT-11).
- Module `analytics`: a new daily limit per network block, `AUTOXIDE_ANALYTICS_EVENTS_PER_DAY` (default 20000), on
  top of the per-minute one, bounds how much one network can skew the reports (PT-12).
- Module `newsletter`: the daily cap on confirmation mails counts per mailbox: letter case, a `+tag` after the name
  and dots in a Gmail name no longer give extra mails (PT-18).
- Module `auth` (SCIM): changing a linked account through a SCIM token also requires the token's creator to be able to
  grant each role the account holds at its unit, not only to have the same permission names (PT-19).
