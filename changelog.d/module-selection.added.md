- `AUTOXIDE_MODULES` chooses which modules run: `core`, `official` (the default), `all`, module
  names to add and `-name` to leave one out; modules that an enabled module needs are turned on
  with it. A module that is off serves no routes, MCP tools, jobs or events, is absent from the
  OpenAPI document served by the server and from `/llms.txt`, and its migrations do not run; data
  it already has is kept. `docs/openapi.json`, `docs/mcp-tools.json` and the SDKs still describe
  every module. See `docs/MODULES.md`.
- Modules have a tier: core (cannot be turned off: `auth`, `mail`, `idempotency`), official, and
  experimental (`bookings`, `newsletter`, `comments`, `support`, `conversations`, `analytics`,
  `reporting`, `automations`), whose API may still change between releases.
  `GET /api/v1/kernel/modules` returns `stability` for each module, and `autoxide doctor` lists
  the modules that run and those that are off.
- Module authors: `Module::stability`, `Module::essential` and `Module::depends_on` (all with
  defaults).
