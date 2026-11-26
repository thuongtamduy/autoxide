- Rate limit per tenant, on top of the limits per client address and per API key:
  `AUTOXIDE_HTTP_RATE_LIMIT_TENANT_PER_MINUTE` / `_BURST` (off by default), or the plan's limit
  `kernel.api_rate_per_minute`; over it REST and MCP answer 429 `tenant_rate_limited` with
  `Retry-After`. Counted in memory or shared through the database like the API key limit. Only
  signed-in callers acting in their own tenant count: anonymous requests (whatever tenant they name),
  inbound webhooks, platform staff and support access never use a tenant's budget, and a sync batch
  counts once.
- AI daily token limit per plan: limit `ai.daily_tokens`; a plan that does not set it keeps
  `AUTOXIDE_AI_DAILY_TOKEN_LIMIT`. `GET /api/v1/ai/status` adds `daily_token_limited`, telling "no
  limit" apart from a limit of 0.
- Tenant settings `catalog.locales`, `menus.locales` and `menus.default_locale` (languages of product
  texts and website menus per tenant, like `content.locales`).
