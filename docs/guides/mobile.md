# Building a mobile or desktop app

Vietnamese: [`mobile.vi.md`](mobile.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`). They run as tests against every build, so they match the server of this version.

Autoxide is an API only: a phone app, a desktop app or a point-of-sale terminal calls the same endpoints as a website. This guide walks through what such an app needs, in the order you usually build it, and points to the guide with the details of each part.

## 1. Pick an SDK

The TypeScript SDK ([`sdk/typescript`](../../sdk/typescript/)) is generated from the API contract (`docs/openapi.json`), so it always matches the server of the same version. Use it for web, React Native and Node. There are no official Swift, Kotlin or Dart SDKs (decision D-20261121-drop-mobile-sdk): for a native or Flutter app, generate a client from `docs/openapi.json` (also served at `/api/v1/openapi.json`) with the OpenAPI generator of your platform, or call the REST endpoints directly. Whatever the client, three rules hold for every call:

- Send the access token in `Authorization: Bearer …`.
- Send an `Idempotency-Key` header (a new random UUID per action, kept across retries) on writes: phones retry on bad networks — see [Retrying safely](retries.md).
- Every error is a `Problem` with a stable `code`; show `detail` (send `Accept-Language: vi` for Vietnamese) and branch on `code` — see [Reading errors](errors.md).

## 2. Sign in and stay signed in

Sign in with email and password (or a passkey, or a sign-in link — see [Signing in](authentication.md)). Keep both tokens in the platform's secure store (Keychain on Apple platforms, Keystore-backed storage on Android), never in plain preferences.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

The access token lives `expires_in` seconds. Shortly before it expires, or when a call answers `401` with `auth.token_expired`, exchange the refresh token for a new pair. Each refresh token works **once**: save the new one before using the new access token. Refresh from one place in the app only (one in-flight refresh at a time); two screens refreshing with the same token look like a stolen token, and the server then ends the whole session.

<!-- example: refresh -->
```bash
curl -sS "$API/api/v1/auth/refresh" --json '{"refresh_token":"'"$REFRESH"'"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

When the refresh answers `401` (`auth.invalid_refresh_token`, `auth.refresh_token_reused`), clear the tokens and show the sign-in screen. When signing out, send the refresh token to `POST /api/v1/auth/logout`, then delete both tokens and the push token of the device (section 4).

Programs without a person (a kiosk, a sync service) use an API key instead — see [API keys](api-keys.md).

## 3. Sign in with Google, Microsoft or GitHub (PKCE)

The server runs the OAuth 2.0 authorization-code flow with PKCE for the app, and keeps the PKCE verifier, `state` and `nonce` itself: the app never holds a client secret. An administrator adds the provider with the app's return address in `redirect_uris` — a claimed HTTPS link (Universal Link on iOS, App Link on Android, e.g. `https://app.example.com/sign-in/callback`) or a custom scheme of your app (`com.example.app:/sign-in/callback`). See [Signing in with Google, Microsoft, GitHub or OpenID Connect](oidc.md).

1. The app calls `POST /api/v1/auth/oidc/{slug}/start` with `{"redirect_uri": "<the app's return address>"}`.
2. It opens `authorization_url` in the system's sign-in browser — `ASWebAuthenticationSession` on Apple platforms, Custom Tabs on Android, `flutter_web_auth_2` in Flutter — not in an embedded web view, which providers refuse.
3. The provider sends the browser back to the return address with `code` and `state`. The app sends both, unchanged, to `POST /api/v1/auth/oidc/{slug}/callback` and gets the same answer as `/login`: tokens, or `202` with an `mfa_token` when two-step sign-in is on.

The list of providers the sign-in screen shows (this server has outside sign-in turned off, so it answers `404`):

<!-- example: providers -->
```bash
curl -sS "$API/api/v1/auth/oidc/providers"
# expect: 404
```

## 4. Push notifications

After signing in, and whenever the operating system gives the app a new push token, register the device. Registering the same token again only updates it, so do it on every app start.

<!-- example: register-device -->
```bash
curl -sS "$API/api/v1/notifications/me/device-tokens" -H "Authorization: Bearer $TOKEN" \
  --json '{"platform":"ios","token":"apns-device-token-from-the-os","app_id":"com.example.app","locale":"vi"}'
# expect: 201
# save: DEVICE=.data.id
```

- `platform` is `ios` (APNs token) or `android` (Firebase Cloud Messaging token); `app_id` is the bundle id or package name; `locale` (`en`, `vi`) picks the language of the messages.
- `GET /api/v1/notifications/me/device-tokens` lists the user's devices; `DELETE /api/v1/notifications/me/device-tokens/{id}` stops pushes to one — call it when signing out.
- The server drops a token the push service reports as no longer valid; register again when the operating system hands out a new one.
- Which messages reach the phone follows the user's notification preferences (`/api/v1/notifications/me/preferences`); the same messages are in the in-app inbox (`/api/v1/notifications/me/inbox`). Sending needs a push provider (FCM, APNs) set up on the server; without one, devices are still registered.

## 5. Upload photos straight to storage

Do not send large photos through the API server. Ask for a signed upload link, send the bytes from the phone straight to S3, then complete the upload — see [Uploading large files straight to storage](direct-uploads.md). Compute `size` and `sha256` of the exact bytes you will send (after resizing on the phone). This server keeps files on its own disk, so it answers `409 media.direct_upload_unavailable`: the app then uploads through `POST /api/v1/media?name=…` instead, with the file itself as the body and its type in `Content-Type` (not a multipart form, which is refused with `415`).

<!-- example: start-upload -->
```bash
curl -sS "$API/api/v1/media/uploads" -H "Authorization: Bearer $TOKEN" \
  -H 'Idempotency-Key: 0193e3a4-7b1c-7f00-9a51-6f2b8c0d4e11' \
  --json '{"name":"shelf.jpg","content_type":"image/jpeg","size":482113,"sha256":"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"}'
# expect: 409
```

On S3 the answer is `201` with `url`, `method` and `headers`: upload with exactly those (a background upload task on iOS, `WorkManager` on Android, so it survives the app going to the background), then `POST /api/v1/media/uploads/{id}/complete`.

## 6. Work offline

An app that must keep working without a network (a shop counter, a delivery app) keeps a copy of the records on the device and sends its writes later. The server lists the kinds of records that can be kept in sync — each module of the server adds its own (the sample `notes` module adds `notes.notes`; this server has none):

<!-- example: sync-resources -->
```bash
curl -sS "$API/api/v1/kernel/sync/resources" -H "Authorization: Bearer $TOKEN"
```

Each kind has a list endpoint (`path`) and a change feed (`changes`). The loop:

1. **First time:** read the list endpoint (with [pagination](pagination.md)), then `GET <changes>` without a cursor; keep the `cursor` of the last change you applied.
2. **Catching up:** `GET <changes>?cursor=<last cursor>`. Each item is `{id, cursor, deleted, version, data}`: the record **as it is now** (apply it again safely), or `deleted: true` — remove it, also when the user lost access to it. Read pages until `pagination.next_cursor` is `null`.
3. **Too far behind:** a cursor older than the server keeps changes (`AUTOXIDE_SYNC_RETENTION_DAYS`, 30 days by default) answers `410 sync_cursor_expired`: drop the local copy and start again from step 1.
4. **Writing offline:** queue each write on the device with its own `idempotency_key` and the `version` (`ETag`) of the record it changed. When the network is back, send the queue with `POST /api/v1/kernel/sync/batch`:

```json
{
  "operations": [
    {
      "id": "w-1",
      "method": "PATCH",
      "path": "/api/v1/notes/0193f1c2-5a7e-7c3d-8e21-4b6f0a9d2c11",
      "idempotency_key": "w-1",
      "if_match": "\"3\"",
      "body": { "title": "Shelf B restocked" }
    }
  ]
}
```

Each write runs exactly as its REST endpoint would — same permissions, checks and answers — and gets its own result `{id, status, etag, location, body}`; a failed write does not stop the others. Sending the batch again (the answer was lost) does not repeat what was done, thanks to the kept `idempotency_key`. A result `412` means someone changed the record since: its `body.current` is the record on the server. Merge, or ask the user, and queue a new write with the new version — the server never merges for you.

## 7. Real-time events and live signals

Screens that must refresh as things happen subscribe to events. The server must set `AUTOXIDE_EVENTS_STREAM=true` (this one does not, so both endpoints answer `404 kernel.stream_disabled`).

- **Server-Sent Events** — `GET /api/v1/kernel/events/stream`: one-way, plain HTTP, the simplest choice. See [Receiving events in real time](realtime-events.md).
- **WebSocket** — `GET /api/v1/kernel/events/ws`: the same events, permissions and resuming, plus live signals the app sends ("typing", "online") in a topic such as a conversation. Use it for chat or a courier's live position screen.

<!-- example: websocket -->
```bash
curl -sS "$API/api/v1/kernel/events/ws?names=content.*" -H "Authorization: Bearer $TOKEN"
# expect: 404
```

Open the socket with the `Authorization` header (native WebSocket clients — `URLSessionWebSocketTask`, OkHttp, `web_socket_channel` — can set it), `names` in the query and, on reconnect, `last_event_id`. Every message is a JSON text frame:

```text
→ {"type":"subscribe","names":"comments.*"}
← {"type":"subscribed","names":["comments.*","content.*"]}
← {"type":"event","event":{"id":"0193f1c2-…","name":"content.entry_published","occurred_at":"2026-11-18T09:30:00Z","subject":"content.entry:0193…","actor_kind":"user","payload":{…}}}
→ {"type":"ack","id":"0193f1c2-…"}
→ {"type":"join","topic":"conversation:0193f1d0-…"}
← {"type":"joined","topic":"conversation:0193f1d0-…","online":["0193e0aa-…"]}
→ {"type":"typing","topic":"conversation:0193f1d0-…"}
← {"type":"signal","topic":"conversation:0193f1d0-…","user_id":"0193e0aa-…","signal":"typing"}
```

- The socket only takes control messages: `subscribe`, `unsubscribe`, `ack`, `join`, `leave`, `typing`, `ping`. **Data is written over REST** (or the sync batch); anything else answers `{"type":"error","code":"kernel.ws_unknown_command"}`.
- Delivery is at least once: skip event ids you already handled. A `reset` message means what you missed is no longer kept — reload over REST.
- Close codes: `4401` — the credentials no longer hold (the access token expired): refresh and reconnect; `4403` — the user may no longer receive any event named; `1012` — the server restarts: reconnect with `last_event_id`. Reconnect with a growing delay (1 s, 2 s, 4 s… up to a minute).
- Live signals are not stored. A topic `<kind>:<id>` exists only when a module opens it (`kernel.ws_unknown_topic` otherwise), and you may join it only when you may read the record.
- On a phone, close the socket when the app goes to the background and rely on push notifications (section 4); reopen it with `last_event_id` when the app comes back.

## 8. In-app purchases

When the app sells the tenant's subscription through the App Store or Google Play, the store collects and keeps the money; the server checks each purchase with the store and puts the tenant on the linked plan. An administrator links store products to plans (`PUT /api/v1/plans/store/products/{store}/{product_id}`), and the server needs the store's credentials.

1. Buy the product with StoreKit 2 or Google Play Billing.
2. Send what the store returned — the signed transaction (StoreKit 2) or the `purchaseToken` (Google Play), unchanged — with `plans.billing.manage`:

<!-- example: claim-purchase -->
```bash
curl -sS "$API/api/v1/plans/store/purchases" -H "Authorization: Bearer $TOKEN" \
  -H 'Idempotency-Key: 0193e3a4-7b1c-7f00-9a51-000000000002' \
  --json '{"store":"app_store","proof":"signed-transaction-from-storekit"}'
# expect: 422
```

Here you are signed in to the platform tenant, which no plan limits, so it answers `422 plans.platform_exempt`; a customer's tenant on a server without the store set up gets `503 plans.store_unavailable`. On a configured server the answer is the tenant's subscription. Sending the same purchase again changes nothing, so send it on every app start until the server confirms it (then finish the transaction in StoreKit / acknowledge it in Google Play). Renewals, expiry and refunds reach the server from the store itself (server notifications); the app does not report them. Users change or cancel the subscription in the store, not through the API.

## Checklist

- Tokens in the secure store; one refresh at a time; `401` after a refresh → sign-in screen.
- `Idempotency-Key` on every write you may retry, kept across retries.
- Push token registered on every start, deleted on sign-out.
- Large files straight to storage.
- Offline: change feed with a kept cursor, queued writes with `if_match`, `412` handled by the app.
- WebSocket or SSE only while the app is in the foreground; reconnect with `last_event_id`.
- Purchases sent to the server until confirmed.
