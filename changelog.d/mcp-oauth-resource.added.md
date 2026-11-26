- OAuth 2.1 protected resource discovery for MCP and REST (MCP authorization 2026-07-28): RFC 9728 metadata at
  `/.well-known/oauth-protected-resource` (and `/mcp`, `/mcp/{module}`, `/api/v1` under it), RFC 8414 metadata at
  `/.well-known/oauth-authorization-server`, `resource_metadata` in every `WWW-Authenticate` challenge, audience
  checks for OAuth access tokens (RFC 8707) and `403 insufficient_scope` challenges for step-up. The documents
  answer `404 oauth_unavailable` until a module provides the authorization server and `AUTOXIDE_HTTP_PUBLIC_URL` is set.
