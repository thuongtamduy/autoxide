# Receiving events in real time

Vietnamese: [`realtime-events.vi.md`](realtime-events.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`), and the TypeScript SDK (`sdk/typescript`). They run as tests against every build, so they match the server of this version.

`GET /api/v1/kernel/events/stream` keeps a connection open and pushes events to you as they happen, as [Server-Sent Events](https://html.spec.whatwg.org/multipage/server-sent-events.html): an admin screen refreshes when a post is published, a moderation queue when a comment comes in. Only events a module **declares streamable** are sent, with the same permission as reading the record over REST — the stream never pushes raw table changes. Systems that must not miss anything while they are offline for longer than a week use webhooks instead.

The server must set `AUTOXIDE_EVENTS_STREAM=true`; otherwise the endpoint answers `404 kernel.stream_disabled`.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
```

## Which events stream

The module catalog lists each event with its `stream_permission`; events without it are not streamed. Today: `content.entry_published` (`content.read`), `comments.comment_created` (`comments.moderate`), `media.item_uploaded` (`media.read`).

<!-- example: streamable-events -->
```bash
curl -sS "$API/api/v1/kernel/modules/content" -H "Authorization: Bearer $TOKEN"
```

## Open a stream

Name the events in `names`, comma-separated: `module.event`, or `module.*` for every streamable event of a module that you may see. This server does not enable streaming, so here it answers `404`; with `AUTOXIDE_EVENTS_STREAM=true` it answers `200` and keeps the connection open (`curl -N` prints each event as it comes).

<!-- example: open-stream -->
```bash
curl -sS "$API/api/v1/kernel/events/stream?names=content.entry_published,comments.*" \
  -H "Authorization: Bearer $TOKEN"
# expect: 404
```

Each event is one frame:

```text
id: 0193f1c2-5a7e-7c3d-8e21-4b6f0a9d2c11
event: content.entry_published
data: {"id":"0193f1c2-5a7e-7c3d-8e21-4b6f0a9d2c11","name":"content.entry_published","occurred_at":"2026-11-18T09:30:00Z","subject":"content.entry:0193…","actor_kind":"user","payload":{"after":{…}}}
```

A `: ping` comment comes every 15 seconds so proxies keep the connection open.

## Who sees what

- Only events of your tenant.
- For each event, the permission in `stream_permission`. Naming an event you may not see answers `403 permission_denied`; with `module.*` such events are left out.
- With a permission narrower than the whole tenant (a role at one organization unit), you receive only the events you caused yourself: events do not yet carry their unit.

## Reconnect without losing events

Delivery is **at least once**. When the connection drops, reconnect with the `Last-Event-ID` header set to the last `id` you received — the browser's `EventSource` does this for you — or with `last_event_id=` in the query when you cannot set headers. You then receive what you missed, and possibly a few events again: skip ids you already have.

Events are kept 7 days. When your last id is older, the stream sends `event: reset` first: reload your data over REST, then keep listening. The server also closes each stream after 30 minutes (so a removed permission takes effect) and when it shuts down; reconnect the same way. Several servers behind a load balancer are fine: every server reads events from the database, so you may reconnect to any of them.

## Limits

Each server keeps at most `AUTOXIDE_EVENTS_STREAM_MAX_PER_USER` streams per user (default 5), `AUTOXIDE_EVENTS_STREAM_MAX_PER_TENANT` per tenant (200) and `AUTOXIDE_EVENTS_STREAM_MAX_CONNECTIONS` in all (1000); more answer `429 kernel.stream_too_many`. Open one stream per screen with every event name it needs, not one per event. Behind Nginx, the response already carries `X-Accel-Buffering: no`; other proxies must not buffer `text/event-stream`.

## With the SDK

`EventSource` cannot send an `Authorization` header, so read the stream with `fetch`:

```ts
const response = await fetch(`${baseUrl}/api/v1/kernel/events/stream?names=content.entry_published`, {
  headers: { Authorization: `Bearer ${token}`, ...(lastId ? { "Last-Event-ID": lastId } : {}) },
});
const reader = response.body!.pipeThrough(new TextDecoderStream()).getReader();
let buffer = "";
for (;;) {
  const { value, done } = await reader.read();
  if (done) break; // closed by the server: reconnect with lastId
  buffer += value;
  let end;
  while ((end = buffer.indexOf("\n\n")) >= 0) {
    const frame = buffer.slice(0, end);
    buffer = buffer.slice(end + 2);
    const id = frame.match(/^id: (.*)$/m)?.[1];
    const data = frame.match(/^data: (.*)$/m)?.[1];
    if (frame.includes("event: reset")) { /* reload over REST */ }
    if (id && data && !seen.has(id)) { seen.add(id); lastId = id; handle(JSON.parse(data)); }
  }
}
```
