- MCP endpoint: protocol errors answered with HTTP 400 or 404 now carry the JSON-RPC error body the
  MCP specification requires, instead of a Problem Details body, so clients that support several
  protocol versions recognize the server as a `2026-07-28` server.
