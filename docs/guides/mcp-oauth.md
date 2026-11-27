# Connecting MCP clients with OAuth sign-in

Vietnamese: [`mcp-oauth.vi.md`](mcp-oauth.vi.md).

With OAuth sign-in on, a person adds your server to Claude, ChatGPT, an IDE or any MCP client by its address alone. The client opens a browser, the person signs in to **your interface** and picks what the client may do, and the client receives tokens that never carry more than that. Nobody copies or pastes a key. Without OAuth, clients still connect with an API key ([Connecting an AI agent over MCP](mcp.md)).

Autoxide follows the [MCP authorization specification](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization) (2026-07-28): OAuth 2.1 with PKCE, Protected Resource Metadata (RFC 9728), Authorization Server Metadata (RFC 8414), resource indicators (RFC 8707), client metadata documents and, for older clients, dynamic client registration (RFC 7591).

## 1. Turn it on

OAuth sign-in is on when both of these are set:

| Setting | Value |
|---|---|
| `AUTOXIDE_HTTP_PUBLIC_URL` | The address clients reach your server at, **without a path**, e.g. `https://api.example.com`. It is the OAuth issuer. With a path (`https://example.com/api`) OAuth stays off and the log says why. |
| `AUTOXIDE_AUTH_OAUTH_CONSENT_URL` | The page of your interface where a signed-in person approves a client, with `{token}` where the request token goes, e.g. `https://app.example.com/connect?request={token}`. |

Optional:

| Setting | Default | Meaning |
|---|---|---|
| `AUTOXIDE_AUTH_OAUTH_DYNAMIC_REGISTRATION` | `true` | Let clients register themselves at `POST /api/v1/auth/oauth/register`. MCP 2026-07-28 deprecates this in favour of client metadata documents; keep it on while the clients you care about still use it. |
| `AUTOXIDE_AUTH_OAUTH_GRANT_DAYS` | `90` | How long an approval lasts at most (1–365 days). The person approving can choose less. Afterwards the client asks again. |
| `AUTOXIDE_AUTH_OAUTH_ALLOW_PRIVATE_NETWORKS` | `false` | Fetch client metadata documents from private addresses and over `http://`. **Tests only**: anyone can name a client, so in production this would let them make your server call your internal network. |

Check with `autoxide doctor` (the `mcp_oauth` line names the issuer, or says why OAuth is off), then:

```bash
curl -sS https://api.example.com/.well-known/oauth-protected-resource/mcp
curl -sS https://api.example.com/.well-known/oauth-authorization-server
```

The first names the resource (`https://api.example.com/mcp`) and its authorization server; the second lists the endpoints (`/api/v1/auth/oauth/authorize`, `/token`, `/revoke`, `/register`). While OAuth is off both answer `404` with code `oauth_unavailable`.

Clients that run in someone else's cloud (Claude and ChatGPT on the web) reach your server from the internet: `AUTOXIDE_HTTP_PUBLIC_URL` must be an `https://` address they can open. A client on the same machine (an IDE, Claude Code) can use a local address.

## 2. What happens when a client connects

1. The client calls `/mcp` without a token and gets `401` with `WWW-Authenticate: Bearer resource_metadata="…/.well-known/oauth-protected-resource/mcp"`. From there it finds the authorization server.
2. The client identifies itself: by a **client metadata document** (its `client_id` is an `https://` URL of a JSON document the server fetches), or by registering at `/register`.
3. The client opens the browser at `/api/v1/auth/oauth/authorize` with PKCE (`S256`). The server checks every parameter and sends the browser to your consent page with a request token. Any parameter error answers `400` right there and never redirects back to the client.
4. On the consent page the person signs in (if needed), sees who is asking and chooses permissions. The page approves or denies through the API, and the browser goes back to the client with a code.
5. The client exchanges the code at `/token` and calls `/mcp` with `Authorization: Bearer axo1_…`.

Tokens:

- The access token (`axo1_…`) is short-lived; the refresh token (`axr1_…`) gives a new pair once and is then used up. Clients must refresh **one at a time**: sending an old refresh token again (or two refreshes at once) is treated as theft and revokes the approval.
- A client for `/mcp` may also use `/mcp/<module>`. REST at `/api/v1` is a separate resource the client must ask for (`resource=https://api.example.com/api/v1`).
- The client acts as the person, with kind `agent`, never with more than the approved permissions **and** what the person can do right now. If the person loses a permission, the client loses it too.
- `tools/list` shows only the tools the approval allows. Calling something the person could do but did not approve answers `403` with `WWW-Authenticate: Bearer error="insufficient_scope", scope="…"`, so the client can ask for more.

Only staff accounts approve clients (customers of a shop cannot). The approval belongs to the tenant the person was signed in to on the consent page.

## 3. The consent page of your interface

Autoxide has no web pages of its own: the consent page lives in your interface, at the address in `AUTOXIDE_AUTH_OAUTH_CONSENT_URL`. It needs a **signed-in session of the person** (password, passkey, single sign-on…). An API key or an OAuth token cannot read or decide requests (`403 auth.session_required`), so a client can never approve itself.

1. **Keep the request token.** Read `request` from the address. If the person is not signed in, sign them in first and come back with the token. Requests wait 10 minutes.
2. **Show the request**: `GET /api/v1/auth/oauth/requests/{token}`.

   ```json
   {
     "data": {
       "client_id": "https://client.example.com/oauth/metadata.json",
       "client_host": "client.example.com",
       "client_name": "Example assistant",
       "registration": "cimd",
       "redirect_uri": "https://client.example.com/oauth/callback",
       "redirect_host": "client.example.com",
       "localhost_redirect": false,
       "resource": "https://api.example.com/mcp",
       "requested_scope": ["content.*"],
       "permissions": [
         {"name": "content.read", "description": "…", "risk": "read", "requested": true},
         {"name": "content.write_own", "description": "…", "risk": "write", "requested": true}
       ],
       "current_permissions": null,
       "expires_at": "2026-11-27T10:10:00Z"
     }
   }
   ```

   - Show `client_host` and `redirect_host` prominently. `client_name` is chosen by the client and not verified: show it next to the host, never alone.
   - `registration` is `cimd` (metadata document: the host really published it) or `dynamic` (self-registered: anyone could have chosen that name).
   - When `localhost_redirect` is `true`, warn: the code goes back to a program on this device, and any program there could pretend to be this client.
   - `permissions` are the ones the person holds, narrowed to what the client asked for. Pre-select those with `requested: true`; let the person untick any. A prefix such as `content.*` is only a request: the approval stores the exact names chosen, so permissions added to the server later never slip into an old approval.
   - `current_permissions` is the person's existing approval of this client for this resource, if any.
3. **Approve**: `POST /api/v1/auth/oauth/requests/{token}/approve` with

   ```json
   {"permissions": ["content.read", "content.write_own"], "days": 30}
   ```

   `days` is optional (default and maximum: `AUTOXIDE_AUTH_OAUTH_GRANT_DAYS`). A permission whose `risk` is `destructive`, `sensitive_data` or `money` also needs `"reauth_token"` from `POST /api/v1/auth/me/reauth` (the person confirms with their password, a passkey or a code); without it the answer is `422` and the request keeps waiting.
4. **Or deny**: `POST /api/v1/auth/oauth/requests/{token}/deny`.
5. **Send the browser on**: both answer `{"data": {"redirect_to": "…"}}`. Set `window.location` to it: the client receives the code (or `error=access_denied`) with its `state` and the issuer `iss`.

## 4. Connecting clients

The address to give every client is `https://api.example.com/mcp` (or `/mcp/<module>` for one module's tools only). Menu names change between versions; the client's own documentation is the reference.

- **Claude** (claude.ai, the desktop and mobile apps): add a custom connector with the server address. Claude opens your consent page in the browser; after approving, the tools appear in chats where the connector is enabled. Claude connects from the internet, so the address must be public `https://`.
- **Claude Code**: `claude mcp add --transport http autoxide https://api.example.com/mcp`, then run `/mcp` in a session and choose to authenticate; the browser opens on this machine.
- **ChatGPT**: in the settings for connectors / apps (developer mode where required), create a connector with the server address and OAuth authentication. ChatGPT also connects from the internet.
- **Other clients** (IDEs, agent frameworks, the [MCP SDKs](https://modelcontextprotocol.io/docs/sdk)): any client that implements MCP authorization works the same way: give it the address and let it open the browser. Clients that keep the redirect on the device use `http://127.0.0.1:<any port>/…`; the server accepts any port for `127.0.0.1` and `[::1]`, but not for the name `localhost` (RFC 8252).

A client that cannot do OAuth still connects with an API key in the `Authorization` header ([Connecting an AI agent over MCP](mcp.md)).

## 5. Seeing and revoking approvals

- The person: `GET /api/v1/auth/me/oauth-grants` lists their approvals (newest first, including revoked and expired); `DELETE /api/v1/auth/me/oauth-grants/{id}` revokes one. Tokens stop working at the next request.
- Administrators with `auth.oauth_grants.manage`: `GET /api/v1/auth/oauth-grants` and `DELETE /api/v1/auth/oauth-grants/{id}` for everyone in the tenant.
- The client itself: `POST /api/v1/auth/oauth/revoke` (RFC 7009) with the access or refresh token revokes the whole approval.
- Automatically: locking, disabling or deleting the account, account recovery, a password change that signs out other sessions, and "sign out everywhere" revoke every approval of the person. Automation rules created through a client pause when its approval is revoked.

Every approval and revocation is an event in the audit log (`auth.oauth_granted`, `auth.oauth_revoked`), and every change made through a client records which client acted for whom.

## 6. Common problems

| Symptom | Cause and fix |
|---|---|
| The client says the server has no authorization / `/.well-known/…` answers `404 oauth_unavailable` | OAuth is off: set `AUTOXIDE_HTTP_PUBLIC_URL` (no path) and `AUTOXIDE_AUTH_OAUTH_CONSENT_URL`, restart, check `autoxide doctor`. API endpoints answer `auth.oauth_disabled`. |
| The browser shows `400` with `auth.oauth_client_invalid` | The client metadata document could not be fetched or is not valid (it must be `https`, port 443, a public address, at most 5 KB, `client_id` equal to its own URL, no client secret), or dynamic registration is off and the client relies on it. The message gives the detail. |
| `400 auth.oauth_redirect_invalid` | The client's `redirect_uri` is not one it registered. Ports are flexible only for `127.0.0.1` / `[::1]`. |
| `400 auth.oauth_request_invalid` | A parameter is missing or wrong (`code_challenge_method` must be `S256`, `resource` must be `…/mcp`, `…/mcp/<module>` or `…/api/v1` of this server). |
| The consent page gets `404 auth.oauth_request_not_found` | More than 10 minutes passed, or the request was already decided. Start connecting again from the client. |
| The consent page gets `403 auth.session_required` / `auth.oauth_staff_only` | Call with the person's own signed-in session, not an API key; only staff accounts approve clients. |
| `422` on approve | A permission was not offered, or a sensitive one needs `reauth_token`. The request still waits. |
| The client is signed out after a while, `/token` answers `invalid_grant` | The approval expired or was revoked, the person can no longer sign in, or the client refreshed twice at once (a reused refresh token revokes the approval). Connect again. |
| A tool answers `403 insufficient_scope` | The person has the permission but did not approve it for this client. Reconnect and approve it. |
| `auth.oauth_rate_limited` | Too many authorization requests from one address; wait a minute. |
