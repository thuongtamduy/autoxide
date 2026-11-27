- OAuth 2.1 authorization server for MCP clients (MCP authorization 2026-07-28): Claude, ChatGPT and other MCP
  clients connect without pasting a key. Clients identify with a Client ID Metadata Document (or the deprecated
  dynamic registration at `POST /api/v1/auth/oauth/register`), the browser goes through
  `GET /api/v1/auth/oauth/authorize` (PKCE `S256` only, `iss` in every answer) to the interface's consent page
  (`AUTOXIDE_AUTH_OAUTH_CONSENT_URL`), which reads and decides the request with `/api/v1/auth/oauth/requests/{token}`.
  Opaque access tokens (`axo1_…`) carry only the permissions the person chose, never more than they currently have;
  refresh tokens rotate, and reusing a code or an old refresh token revokes the approval. People list and revoke
  their approvals at `/api/v1/auth/me/oauth-grants`; administrators with `auth.oauth_grants.manage` at
  `/api/v1/auth/oauth-grants`. Locking an account, signing out everywhere or changing the password with sign-out
  revokes its approvals; automation rules made through a client pause when its approval is revoked.
