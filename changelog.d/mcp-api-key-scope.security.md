- Module `auth`: an API key can no longer list or revoke the other API keys of its owner, through REST
  (`GET /api/v1/auth/api-keys`, `DELETE /api/v1/auth/api-keys/{id}`) or the MCP tools `auth.list_api_keys` and
  `auth.revoke_api_key`: it sees only itself and can only revoke itself (another id answers `404`). A sign-in
  session still manages every key of its user. Revoking someone else's key with `auth.api_keys.manage`
  (`DELETE /api/v1/auth/all-api-keys/{id}`) now also needs every permission of the key's owner
  (`403 auth.target_has_more_permissions`).
