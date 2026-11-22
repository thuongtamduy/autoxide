- OpenAPI: every operation of an experimental module carries `"x-stability": "experimental"` and
  says so in its description; `docs/mcp-tools.json` has `stability` for every tool, and MCP tool
  descriptions of experimental modules say they may change.
- Moving a tenant to another database, and splitting or merging tenants, answer
  `409 tenant_move_modules_off` while the tenant has data of a module that is turned off, instead
  of leaving that data behind. The startup warning and `autoxide doctor` now also look at the
  database of every placement for such data.
