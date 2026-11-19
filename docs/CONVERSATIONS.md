# Customer chat (module `conversations`)

The `conversations` module lets signed-in customers chat with a shop — or with a seller's store on a marketplace —
about a product, an order or anything else. Staff answer from a shared inbox. A conversation belongs to an
organization unit, so chains and marketplaces split the work by branch like every other record.

Vietnamese: [`CONVERSATIONS.vi.md`](CONVERSATIONS.vi.md).

Calls (talking to the shop from the browser, "call me back") go through a voice provider connector — a separate
module that needs an account with the provider; none ships yet (tests use `FakeVoiceProvider`).

## Permissions

| Permission              | Allows                                                                                     |
| ----------------------- | ------------------------------------------------------------------------------------------ |
| `conversations.read`    | Seeing the conversations of the units in your scope and receiving their events in real time |
| `conversations.reply`   | Answering, taking a conversation, closing it, AI drafts, handing it over as a support ticket |
| `conversations.manage`  | Assigning colleagues, blocking customers, reviewing reports of abuse                       |
| `conversations.call`    | Call-back requests, calling customers back, the calls of a conversation (numbers masked)   |
| `conversations.recordings.read` | Listening to call recordings (every listen is recorded)                            |
| `conversations.voice.manage` | The phone line of a unit, recording on or off, deleting recordings on request          |

With scope `own`, staff see the conversations assigned to them. Customers need no permission: any signed-in account
uses `/api/v1/conversations/me/threads`.

## Conversations and messages

- `POST /me/threads` starts a conversation with a unit (`org_id`, default: the shop) about a record (`subject`,
  `<kind>:<id>`, e.g. `catalog.product:<id>`). When the customer already has an open conversation with the same unit
  about the same record, the message goes there instead.
- Every message carries a `client_id` the app makes up. Sending again with the same `client_id` (a retry after a lost
  connection) returns the first message with status 200 and never adds a second one.
- Messages are plain text plus up to 5 images from the media library. They cannot be edited; the author may recall a
  message within 2 minutes (its text and images are removed, its place stays).
- Staff list the inbox at `GET /threads` (filters `status`, `assignee=me|none|<id>`, `unread`, `needs_agent`,
  `org_id`, `subject`), mark a conversation read (`POST /threads/{id}/read`; read cursors are shown to the other
  side), take or assign it (`POST /threads/{id}/assign`) and close it. A closed conversation takes no more messages; the
  customer's next message starts a new one.

## Real time and notices

Open `GET /api/v1/kernel/events/stream?names=conversations.*` (the server must set `AUTOXIDE_EVENTS_STREAM=true`).
Customers and assigned agents receive the events of their conversations without any permission; staff with
`conversations.read` for the whole tenant receive all of them. Events never carry message text: on
`conversations.message_created`, read the message over REST. `POST …/typing` sends `conversations.typing`.

A message still unread after `AUTOXIDE_CONVERSATIONS_NOTIFY_AFTER_MINUTES` (default 5) produces one notice for the
customer, or for the assigned agent — one notice per conversation and window, however many messages came.

## Help for agents

- Automatic answers: each customer message is passed to the automatic answering of module `support` (same approved
  answers, same rules, switched on per unit there, off by default). Automatic messages have `auto_reply: true`; show a
  "Talk to a person" button that calls `POST /me/threads/{id}/handoff`.
- `POST /threads/{id}/suggest-reply` asks the AI for a draft (`draft_reply`); nothing is sent by itself.
- `POST /threads/{id}/escalate` opens a support ticket with the latest messages and a link back; calling again returns
  the same ticket.

## Abuse and the law

- `AUTOXIDE_CONVERSATIONS_MESSAGES_PER_MINUTE` (default 20) per person; more answer 429.
- `PUT /threads/{id}/block` stops a customer writing to that unit; customers and staff report abuse with
  `POST …/reports`, reviewed at `GET /reports`.
- Unit settings: `conversations.require_verified_account` (customers must have a verified phone number, Decree
  147/2024), `conversations.mask_contacts` and `conversations.mask_terms` (hide phone numbers, emails, links and listed
  words — marketplaces use it to keep deals on the platform).
- Personal data (Law 91/2025/QH15): conversations are deleted `AUTOXIDE_CONVERSATIONS_RETENTION_DAYS` (default 365) after
  the last message; deleting an account erases that customer's messages and unlinks their conversations; the personal
  data export includes them.

## Calls

- `PUT /voice/lines/{org_id}` sets the unit's line: provider, hotline, call-back hours (default 08:00–20:00 every day,
  UTC+7), recording (off by default). The first response carries `webhook_path` for the provider — keep it.
- Customers read `GET /me/threads/{id}/voice` first (hide call buttons when `available` is false). `POST
  /me/threads/{id}/calls` starts a browser call and returns a token for the provider's WebRTC SDK. `POST
  /me/threads/{id}/callbacks` asks to be called back at the account's verified phone number, in a window within the
  call-back hours and the next `AUTOXIDE_CONVERSATIONS_CALLBACK_DAYS_AHEAD` (7) days; one open request per conversation;
  `AUTOXIDE_CONVERSATIONS_CALLS_PER_DAY` (5) calls and requests per day.
- Agents see open requests in the work inbox and at `GET /callbacks` (numbers masked), and call with `POST
  /callbacks/{id}/call` (with an `Idempotency-Key`); the customer sees the shop's hotline. When a call ends, one summary
  line is added to the conversation. Unanswered calls reopen the request; requests 2 hours past their window expire and
  the customer is told once.
- Recording: switch it on only with a provider that plays a recording notice; customers must then send
  `recording_ack: true`. Recordings move from the provider to your file storage, are kept
  `AUTOXIDE_CONVERSATIONS_RECORDING_RETENTION_DAYS` (90) days, deleted with `DELETE /calls/{id}/recording` or with the
  customer's account. Personal data law (Law 91/2025/QH15): tell customers before recording — check with a legal expert
  whether consent is also needed.
