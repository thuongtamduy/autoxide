- MCP endpoint (`/mcp`, `/mcp/<module>`): served by Autoxide's own implementation of the MCP
  specification instead of the `rmcp` library. It speaks protocol version `2026-07-28` (no
  handshake, `server/discover`, per-request `_meta`, `Mcp-Method` / `Mcp-Name` headers) and every
  earlier version that starts with `initialize`. Tools, resources, prompts and permissions behave
  as before.
