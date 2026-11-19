# Customer support (module `support`)

The `support` module takes support requests from customers — questions about an order, a product, an account or a
payment — and lets the shop's staff answer them until they are done. Each request is a **ticket** with messages back
and forth. Tickets belong to an organization unit (a shop, a branch, a seller's store on a marketplace), so chains and
marketplaces split the work by branch like every other record.

Vietnamese: [`SUPPORT.vi.md`](SUPPORT.vi.md).

What it does not do: consumer complaints with legal deadlines live in module `compliance` (a ticket is handed over
there, never copied), disputes between buyers and sellers in `marketplace`, returns in `orders`. Live chat is a later
module.

## Permissions

| Permission                  | Allows                                                                                       |
| --------------------------- | -------------------------------------------------------------------------------------------- |
| `support.tickets.read`      | Seeing the tickets of the units in your scope, with messages and internal notes             |
| `support.tickets.reply`     | Answering, internal notes, opening tickets for customers, status, priority, assignee, links |
| `support.tickets.manage`    | Hiding messages, handing tickets over as consumer complaints or to the platform             |
| `support.settings.manage`   | Support policy, teams, routing rules and saved replies of the units in your scope           |
| `support.auto_reply.manage` | Approving saved replies for automatic answers                                                |

Every permission works per organization unit: staff of a branch see the tickets of that branch and the branches
below it; a marketplace seller with a grant at their store sees only that store's tickets. With scope `own`, staff
see the tickets assigned to them.

Customers need no permission: any signed-in account sees its own tickets at `/api/v1/support/me/tickets`.

## Tickets

- **From the website** — `POST /api/v1/support/public/tickets`. Signed in: the ticket belongs to the account. Guest:
  give `email` or `phone` and the anti-spam fields of the form (`GET /api/v1/antispam/token?scope=support:tickets`).
  The answer carries `access_token` once — the guest keeps it to see (`GET /public/tickets/{id}?token=`) and answer
  (`POST /public/tickets/{id}/messages`) this one ticket. A 6-digit code goes to the email or phone; the guest
  confirms it (`POST /public/tickets/{id}/verify`) before writing a second message.
- **By staff** — `POST /api/v1/support/tickets` with the customer's account, email or phone (a phone call, a visit).
- **From another module** — through the trait `SupportIntake` (e.g. the order page); the same `reference` never makes
  two tickets.
- **After sign-up** — `POST /api/v1/support/me/claim` attaches the verified guest tickets with the account's verified
  phone number, or with the email / phone of a verified guest ticket whose token the customer sends.

Statuses: `open` → `pending_agent` (the customer wrote) ⇄ `pending_customer` (staff answered) → `solved` → `closed`.
A customer writing on a `solved` ticket opens it again; `closed` is final (open a new ticket with `follow_up_of`).
Only allowed steps are taken (`support.invalid_transition`).

Messages are never edited. A mistaken message is hidden with a reason (`POST /tickets/{id}/messages/{message_id}/hide`,
`support.tickets.manage`); the customer sees that a message was hidden. Internal notes (`internal: true`) are for staff
only and notify the colleagues listed in `mentions`. Signed-in customers attach media items they uploaded themselves
(at most 5 per message).

## Settings per unit

All settings are data of an organization unit and apply to the units below it unless they have their own.

- **Policy** — `PUT /api/v1/support/policies/{org_id}`: ticket categories, minutes to the first answer and to solve
  per priority (`low`, `normal`, `high`, `urgent`, counted in business time), business hours and days off, UTC offset,
  days before a solved ticket closes, whether guests may open tickets, email replies, automatic answers. Defaults:
  Monday–Friday 08:00–17:00 (UTC+7), first answer within 8 business hours and solved within 3 business days for
  `normal`.
- **Teams** — `POST /api/v1/support/teams`, members with `PUT /teams/{id}/members/{user_id}` (`accepting` new tickets;
  members switch it themselves). A member must be able to answer tickets of the unit.
- **Routing rules** — `POST /api/v1/support/routing-rules` (`category`, `channel` → team). A new ticket goes to the
  team of the first matching rule (nearest unit first) and round-robin to the member waiting longest who still has
  `support.tickets.reply` at the ticket's unit.
- **Saved replies** — `POST /api/v1/support/macros` with text per language and the variables `{customer_name}`,
  `{ticket_number}`, `{subject}`; fill one in for a ticket with `POST /tickets/{id}/macros/{macro_id}/render`.

## Deadlines and work

The first-answer and solve deadlines are computed when a ticket opens and when its priority changes. While the
ticket waits for the customer, the clock stops; when the customer writes, the solve deadline moves by the business
time waited. The job `support.sla_check` (every minute) marks late tickets once (`breached`, event
`support.ticket_breached`); `support.auto_close` closes solved tickets after the policy's days. Tickets waiting for
staff show in the work inbox (`GET /api/v1/kernel/work`, source `support.tickets`). `GET
/api/v1/support/reports/summary?from=&to=` counts tickets, solved and late ones, median times and the average rating.

## Links, hand-over, ratings

- `POST /tickets/{id}/links {"target": "orders.order:<id>"}` links a record of any module (it must exist); the
  order page lists related tickets with `GET /tickets?link=orders.order:<id>`.
- `POST /tickets/{id}/escalate {"to": "consumer_complaint"}` hands the ticket to `compliance` as a consumer
  complaint with legal deadlines: the ticket keeps `case_id` and closes as `escalated`; calling again never opens a
  second case. Customers who want to complain send it directly to `POST /api/v1/compliance/public/complaints`.
  `{"to": "platform"}` hands a seller's ticket to the marketplace's staff.
- Once solved, customers rate the support from 1 to 5 (`POST /me/tickets/{id}/satisfaction`, or the public endpoint
  with the token).

## AI and email

- `POST /tickets/{id}/reply-suggestions` drafts an answer with module `ai` from the ticket's messages (emails and
  phone numbers removed) and the unit's saved replies. It is only a draft: staff edit it and send it with
  `ai_assisted: true`, and the customer sees the label.
- Email replies (off by default; turn on `inbound_email` in the policy): an inbound mail module gives replies to
  `support+<key>@your-domain` to the trait `InboundMail`; the key of a ticket comes from
  `InboundMail::reply_address_key`. Mail from anyone but the customer, to an unknown key or a closed ticket is dropped;
  mail never opens a new ticket. Quoted earlier mail is cut.

## Automatic answers (off by default)

Turn on `auto_reply` in a unit's policy. When a customer writes, the AI only **classifies** the question; rules in the
server decide, and only two kinds of text ever reach the customer:

1. **An approved saved reply**, word for word with its variables filled in. Approve one with
   `PUT /api/v1/support/macros/{id}/auto-reply {"enabled": true, "questions": {...}, "review_due_on": "YYYY-MM-DD"}`
   (`support.auto_reply.manage`); after the review day it is no longer used, and editing its text withdraws the
   approval.
2. **A sentence built from exact data** by a module that attached a `FactSource` (e.g. the status of the customer's
   own order); it answers nothing when it is not sure the customer owns the data.

An answer is sent only when the classification is confident enough (`auto_reply_min_confidence_bp`, default 9000),
carries no risk flag, the question has no blocked word (built-in: complaints, refunds, returns, cancellations, payment
problems, personal data, legal words, asking for a person — plus the unit's `auto_reply_blocked_terms`), and fewer than
`auto_reply_max_per_thread` (default 2) automatic answers came in a row. Otherwise staff answer as usual, with an AI
draft prepared. Automatic messages have `auto_reply: true`: label them "Automatic answer" and show a "Talk to a person"
button (`POST /me/tickets/{id}/handoff`, or the public one with the token), which stops automatic answers on that
ticket. Customers rate an answer (`…/auto-replies/{reply_id}/feedback`); a saved reply reported wrong
`AUTOXIDE_SUPPORT_AUTO_REPLY_FLAG_LIMIT` (3) times in 7 days stops being used. Try a question with
`POST /api/v1/support/auto-reply/preview` (nothing is sent) and read the log at `GET /api/v1/support/auto-replies`.

## For AI agents (MCP)

Tools, with the same permissions as REST: `support.list_tickets`, `support.get_ticket`, `support.reply_ticket`,
`support.add_internal_note`, `support.update_ticket`, `support.link_ticket`, `support.suggest_reply` (a draft only),
`support.preview_auto_reply`.

## Settings

| Variable                                  | Default | Meaning                                                    |
| ----------------------------------------- | ------- | ---------------------------------------------------------- |
| `AUTOXIDE_SUPPORT_GUEST_PER_HOUR`         | 5       | Guest tickets per hour per network address, email, phone   |
| `AUTOXIDE_SUPPORT_MAX_BODY_CHARS`         | 10000   | Longest message                                            |
| `AUTOXIDE_SUPPORT_TOKEN_DAYS_AFTER_CLOSE` | 30      | Days a guest's ticket token works after the ticket closes |
| `AUTOXIDE_SUPPORT_GUEST_URL`              | —       | Page showing a ticket to a guest (`{id}`, `{token}`)       |
| `AUTOXIDE_SUPPORT_AUTO_REPLY_FLAG_LIMIT`  | 3       | Wrong reports in 7 days that retire an automatic answer    |

## Personal data

Deleting an account anonymizes its tickets (name, contact and the customer's text are removed) and keeps them for
statistics. The customer's tickets and messages are part of the personal data export. Legal basis to check with a
qualified person: Law 91/2025/QH15 on personal data protection, Law 19/2023/QH15 on consumer protection, Law
134/2025/QH15 on artificial intelligence (labelling AI-written answers).
