- Module `auth`: managing API keys follows the credential the request carries, over REST and the MCP tools alike.
  A password session manages every key of its user. An API key lists and revokes only itself: another id answers
  `404`. An OAuth token lists no key and revokes none. Rotating a key needs a password session, checked before the
  key is looked up.
- Revoking a key with `auth.api_keys.manage` (`DELETE /api/v1/auth/all-api-keys/{id}`) needs every permission of the
  key's owner, unless the owner does it from a password session (`403 auth.target_has_more_permissions`).
- `GET /api/v1/auth/me/passkeys` needs a password session (`403 auth.session_required`).
