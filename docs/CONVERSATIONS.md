# Customer chat (module `conversations`)

The `conversations` module lets signed-in customers chat with a shop — or with a seller's store on a marketplace —
about a product, an order or anything else. Staff answer from a shared inbox. A conversation belongs to an
organization unit, so chains and marketplaces split the work by branch like every other record.

Vietnamese: [`CONVERSATIONS.vi.md`](CONVERSATIONS.vi.md).

Calls (talking to the shop from the browser, "call me back") are planned on top of this module; they need a voice
provider account and are not part of it yet.

## Permissions

| Permission              | Allows                                                                                     |
| ----------------------- | ------------------------------------------------------------------------------------------ |
| `conversations.read`    | Seeing the conversations of the units in your scope and receiving their events in real time |
| `conversations.reply`   | Answering, taking a conversation, closing it, AI drafts, handing it over as a support ticket |
| `conversations.manage`  | Assigning colleagues, blocking customers, reviewing reports of abuse                       |

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
