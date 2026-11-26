- The `kernel.api_requests` plan quota no longer counts requests refused with 429 (rate limit per tenant
  or the plan's own checks), and once a tenant's quota is used up only its own signed-in callers are
  refused — MCP tool calls included (they now get `quota_exhausted` like REST); a failed authentication is remembered for the rest of the request instead of being tried
  again (and counted against the API key's limit again).
