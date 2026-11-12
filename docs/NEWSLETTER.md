# Newsletter

*Tiếng Việt: [NEWSLETTER.vi.md](NEWSLETTER.vi.md)*

The `newsletter` module keeps mailing lists and their subscribers, takes sign-ups from the
public with a confirmation mail (double opt-in), unsubscribes in one click, and sends
campaigns in batches at a rate you choose, without ever sending one person the same campaign
twice. It keeps a suppression list fed by bounces and complaints, can turn each new post of the
`content` module into a campaign, and exports or erases everything about an email address. It
does **not** track opens or clicks. It is API only: the website shows the sign-up form, the
confirmation page and the preferences page; the admin screens are a separate project. It lives
under `/api/v1/newsletter`. The full contract is in [`openapi.json`](openapi.json).

No third-party account is needed: spam is stopped by the core's shared anti-spam layer, and
mail goes through the `mail` module (your SMTP relay).

## Before you send

1. **An SMTP relay that signs with DKIM.** Gmail and Yahoo refuse bulk mail that is not
   authenticated, and the one-click unsubscribe headers (RFC 8058) only count when the DKIM
   signature covers them. Set `AUTOXIDE_MAIL_SMTP_URL` and `AUTOXIDE_MAIL_FROM` to a relay that
   signs your domain (SPF and DMARC too).
2. **`AUTOXIDE_HTTP_PUBLIC_URL`** — the address of this API, e.g. `https://api.example.com`.
   Every campaign mail carries `List-Unsubscribe: <https://api.example.com/api/v1/newsletter/public/unsubscribe?token=…>`.
3. **`AUTOXIDE_NEWSLETTER_CONFIRM_URL`** — your website's confirmation page, with `{token}`.
   Set it even if no list uses double opt-in: suspicious sign-ups and people re-joining a list
   they left are confirmed by mail on every list.
4. **`AUTOXIDE_NEWSLETTER_PREFERENCES_URL`** — your website's preferences page, with `{token}`;
   the footer of every campaign mail links there.
5. **`AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS`** — the sender's name and postal address, printed in
   the footer; required for advertising campaigns.

Nothing here is needed to start the server: a feature that needs a missing setting answers
`503` with a code naming it (`public_url_missing`, `newsletter.confirm_url_missing`,
`newsletter.preferences_url_missing`, `newsletter.postal_address_missing`).

## Permissions

| Permission          | Allows                                                                                  |
| ------------------- | --------------------------------------------------------------------------------------- |
| `newsletter.read`   | Reading lists, subscribers (personal data), campaigns with progress, suppressions       |
| `newsletter.manage` | Lists, importing and changing subscribers, writing draft campaigns, suppressions, bounces |
| `newsletter.send`   | Sending, scheduling, pausing, resuming and cancelling campaigns; test mails; `new_posts: send` on a list |
| `newsletter.export` | Exporting or erasing everything about an email address                                  |
| `newsletter.subscribe` | Adding sign-ups without the anti-spam checks for anonymous visitors (a website server's own API key) |

An agent with `newsletter.manage` but not `newsletter.send` can write campaigns that a person
then reviews and sends: changing a scheduled campaign moves it back to draft, and setting a list
to send new posts automatically needs `newsletter.send`, so nothing goes out that nobody with
`newsletter.send` approved. Visitors need nothing: everything under
`/api/v1/newsletter/public/` is open. Give `newsletter.subscribe` only to the API key of a
website server; being signed in is not enough to skip the anti-spam checks.

## Lists

```bash
curl -X POST http://localhost:8080/api/v1/newsletter/lists \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -H "Idempotency-Key: $(uuidgen)" \
  -d '{"key": "weekly", "name": {"en": "Weekly news", "vi": "Tin hằng tuần"},
       "description": {"en": "Every Monday."}, "public": true}'
```

- `double_opt_in` (default `true`): public sign-ups get a confirmation mail and receive nothing
  until they open its link.
- `public` (default `false`): shown by `GET /public/lists` and open to public sign-up.
- `new_posts`: `off` (default), `draft` or `send` — see "New posts". Setting `send` needs
  `newsletter.send`.
- `PATCH` and `DELETE` need `If-Match`. A list with an unfinished campaign cannot be deleted
  (`409 newsletter.list_in_use`); deleting a list deletes its subscriptions.

## The website: sign-up

1. `GET /api/v1/newsletter/public/lists?locale=vi` — the public lists and the `token_scope`
   (`newsletter:subscribe`).
2. Each time the form is shown (never in cached HTML):
   `GET /api/v1/antispam/token?scope=newsletter:subscribe` — a form token. Show a hidden trap
   input that people leave empty, and a consent box that is **not ticked in advance**.
3. `POST /api/v1/newsletter/public/subscribe`:

```json
{"email": "ada@example.com", "lists": ["weekly"], "locale": "vi",
 "consent_text": "Gửi tôi bản tin hằng tuần.", "source": "footer",
 "form_token": "…", "trap": ""}
```

- The answer is always `202 {"data": {"accepted": true}}` — new address, waiting for
  confirmation, already subscribed, blocked, trap filled, too many requests: nobody can learn
  which addresses are on a list. Tell the visitor to check their inbox.
- The request only checks the data and the anti-spam token, then queues the sign-up; a
  background job stores it and sends the confirmation mail a moment later. So the time the
  answer takes does not depend on the address either. The job worker must be running (it is,
  in the default `all` process role).
- Wrong data: `422` with every problem in `errors` (a key that is not a public list is
  `lists[0]`). A missing or expired token: `400 antispam.token_*` — reload the form.
- The consent text, its version, the time, the source and a hash of the network block are kept
  with the subscription as proof.
- The confirmation mail contains the list names and the link — never anything the visitor
  typed, so the form cannot be used to send spam to someone else. At most
  `AUTOXIDE_NEWSLETTER_CONFIRM_MAILS_PER_DAY` such mails go to one address a day.
- The confirmation mail is always needed, even on a list without double opt-in, for a sign-up
  the anti-spam checks find suspicious (sent too fast…) and for an address that unsubscribed
  from that list or is still waiting for confirmation: nobody can sign someone else back up.
- Every stored sign-up request counts towards `AUTOXIDE_NEWSLETTER_CONFIRM_MAILS_PER_DAY` for
  its address, whether it needs a mail or not, so answers take about as long either way. If the
  mail cannot be queued (for example `AUTOXIDE_NEWSLETTER_CONFIRM_URL` is not set and the list
  has no double opt-in), the answer is still `202` and the reason goes to the server log.
- A website server may call with its own API key instead, if that key has the permission
  `newsletter.subscribe`: no token needed, limited to
  `AUTOXIDE_NEWSLETTER_SUBSCRIBE_PER_USER_PER_HOUR`. Any other signed-in caller (for example a
  self-registered account) goes through the same checks as an anonymous visitor: without a form
  token it gets `400 antispam.token_missing`.

## The website: confirmation and preferences pages

- **Confirmation.** The mail links to `AUTOXIDE_NEWSLETTER_CONFIRM_URL`. The page sends
  `POST /api/v1/newsletter/public/confirm {"token": "…"}` (opening the link alone, as mail
  scanners do, changes nothing). `200` lists the confirmed lists; `400
  newsletter.confirmation_invalid` means the link expired (after
  `AUTOXIDE_NEWSLETTER_CONFIRM_HOURS`) or was used.
- **Preferences.** Every campaign mail links to `AUTOXIDE_NEWSLETTER_PREFERENCES_URL`. The page
  reads `GET /api/v1/newsletter/public/preferences?token=…` (the address, name, language, and
  every public list and list the person is on) and saves with
  `PUT /api/v1/newsletter/public/preferences?token=…`
  `{"lists": [{"key": "weekly", "subscribed": false}], "unsubscribe_all": false}`.
  Unsubscribing takes effect at once; joining a public list from here is confirmed at once (the
  token came to the person's inbox).
- **One click.** Mail apps call `POST /api/v1/newsletter/public/unsubscribe?token=…` with the
  form body `List-Unsubscribe=One-Click`; the person leaves the list of that mail at once. The
  answer is the same for any token.

Unsubscribe links never expire. Each mail has its own random token; only its hash is stored.

## Campaigns

```bash
# Draft (newsletter.manage)
curl -X POST http://localhost:8080/api/v1/newsletter/campaigns \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"list_id": "…", "subject": "This week", "body_text": "Hello…",
       "body_html": "<p>Hello…</p>", "locale": "en"}'
# Read it, then send exactly that version (newsletter.send)
curl -X POST http://localhost:8080/api/v1/newsletter/campaigns/$ID/send \
  -H "Authorization: Bearer $TOKEN" -H 'If-Match: "1762934400000000"'
```

- `POST /campaigns/{id}/test {"email": …}` sends one test mail.
- `send` takes every confirmed subscriber of the list at that moment (the snapshot), then hands
  mails to the mail queue in batches of `AUTOXIDE_NEWSLETTER_BATCH_SIZE`, at most
  `AUTOXIDE_NEWSLETTER_SEND_RATE_PER_MINUTE` a minute. Right before each mail, suppressed,
  blocked and since-unsubscribed people are skipped.
- `schedule {"at": "2026-12-01T08:00:00Z"}`, `pause`, `resume`, `cancel`. Pausing stops after
  the batch being sent; resuming sends to the recipients left — nobody gets a campaign twice,
  even when jobs are retried or run on several servers. Cancelling skips the mails not sent yet.
- `send` and `schedule` need `If-Match`, so that the version you reviewed is the one sent. Only
  a draft or scheduled campaign can be changed; only one that never sent can be deleted.
- Changing a scheduled campaign (`PATCH`, with any real change) moves it back to `draft` and
  drops its schedule; the jobs already waiting for the old schedule do nothing. Someone with
  `newsletter.send` must schedule or send it again.
- `body_html` is stored and sent as is — it is **not sanitised**. An admin interface must never
  insert it into its own page: preview it only inside a sandboxed `<iframe sandbox>` (no
  scripts, no same origin).
- `GET /campaigns/{id}` shows `progress`: `total`, `pending`, `sent` (handed to the mail
  queue), `failed`, `skipped`.
- Every mail gets a footer (in the campaign's `locale`) with the preferences link and the
  postal address, and the headers `List-Unsubscribe` and
  `List-Unsubscribe-Post: List-Unsubscribe=One-Click`.
- One gap cannot be closed: if a server dies right after handing a mail to the queue and before
  recording it, that recipient is marked `failed` (`interrupted`) rather than risk a second mail.

## Suppressions and bounces

- A suppressed address gets no campaign mail, whatever its subscriptions.
- A permanent SMTP failure of a mail sent by this server (`mail.delivery_failed`) suppresses the
  address (`hard_bounce`) and marks that mail `failed`.
- Mail services that report bounces call `POST /api/v1/newsletter/bounces`
  `{"email": …, "type": "hard" | "soft" | "complaint", "reason": …}` with an API key that has
  `newsletter.manage`. `hard` and `complaint` suppress at once; `soft` after
  `AUTOXIDE_NEWSLETTER_SOFT_BOUNCE_LIMIT` reports.
- `GET`/`POST /suppressions`, `DELETE /suppressions/{id}` manage the list by hand.

## Subscribers

- `GET /subscribers?email=&list_id=&status=` — search by part of the address, list, status.
- `POST /subscribers/import` — up to 1000 people into lists as `confirmed`, with a required
  `consent_source` (where they agreed). Someone who unsubscribed from a list stays unsubscribed.
- `PATCH /subscribers/{id}` (`If-Match`) — name, language, attributes, `blocked`,
  `unsubscribe_list_ids`. `DELETE /subscribers/{id}` deletes them for good.

## New posts

A list with `new_posts` set to `draft` gets a draft campaign (title, summary and link of the
post) each time the `content` module publishes a **post** (pages are left out); `send` sends it
at once (setting `send` needs `newsletter.send`). `new_post_locale` keeps only posts in one language. The link is
`AUTOXIDE_NEWSLETTER_SITE_URL` followed by the post's path; without it (or without the settings
needed to send) campaigns stay drafts. A post makes at most one campaign per list, ever —
republishing it makes nothing new. Importing many old posts publishes them all: keep `new_posts`
at `off` or `draft` while you do.

## Personal data

- `GET /api/v1/newsletter/data?email=…` exports, and `DELETE /api/v1/newsletter/data?email=…`
  erases for good, everything about an address: subscriber, subscriptions with consent records,
  mails sent, confirmation links (permission `newsletter.export`). A **suppression of the
  address is kept** so that an address that bounced, complained or asked never to be mailed is
  not mailed again; delete it with `DELETE /suppressions/{id}` if the person asks.
- Subscribers are not tied to user accounts: requests go by email. When an account is deleted,
  lists and campaigns it created stay; `created_by` becomes `null`.
- Network addresses are never stored: only a keyed hash of the network block, inside the
  consent record.
- Events (and so webhooks) carry ids and list keys only, never addresses or names.
- A daily job deletes subscriptions left unconfirmed after `AUTOXIDE_NEWSLETTER_UNCONFIRMED_DAYS`.

## Legal

What the law asks of a newsletter depends on where you and your readers are. The module gives
you settings, not rules written in the code; **the texts below were not checked against the
original legal texts — check them for your case before you rely on them.**

- **Vietnam, Decree 91/2020/ND-CP (spam):** advertising mail only to people who agreed before;
  a label at the start of the subject of advertising mail; at most 3 advertising mails to one
  address in 24 hours unless the person agreed otherwise; sender details and a way to refuse
  more. The module's defaults follow this reading: mark a campaign `advertising: true`, and set
  `AUTOXIDE_NEWSLETTER_ADVERTISING_LABEL` (default `[QC]`; some sources say `[ADV]`),
  `AUTOXIDE_NEWSLETTER_ADVERTISING_MAX_PER_DAY` (default `3`) and
  `AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS`. Whether an editorial newsletter counts as advertising is
  for you to decide. A decree replacing 91/2020 was in draft at the time of writing.
- **Vietnam, Personal Data Protection Law 91/2025/QH15 and Decree 356/2025:** consent must be
  freely given, clear and provable, silence is not consent, withdrawing must be as easy as
  giving, and people may ask for their data to be erased. The module keeps the consent text,
  version, time and source; double opt-in is on by default; unsubscribing is one click; export
  and erasure go by email. Check the deadlines for answering requests.
- **Elsewhere:** for example the EU GDPR and ePrivacy rules, or the US CAN-SPAM Act (opt-outs
  within 10 business days — the module unsubscribes at once).

## AI agents (MCP)

Tools `newsletter.list_lists`, `newsletter.find_subscribers`, `newsletter.get_campaign`
(progress), `newsletter.create_campaign` (draft), `newsletter.send_campaign` and
`newsletter.schedule_campaign` (pass the `version` you read, like `If-Match`), with the same
permissions as REST.

## Settings

| Setting                                            | Default                  |
| -------------------------------------------------- | ------------------------ |
| `AUTOXIDE_NEWSLETTER_CONFIRM_URL`                  | — (needed for confirmation mails) |
| `AUTOXIDE_NEWSLETTER_PREFERENCES_URL`              | — (needed to send)       |
| `AUTOXIDE_NEWSLETTER_SITE_URL`                     | — (new-post links)       |
| `AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS`               | — (needed for advertising) |
| `AUTOXIDE_NEWSLETTER_CONFIRM_HOURS`                | `168`                    |
| `AUTOXIDE_NEWSLETTER_CONFIRM_MAILS_PER_DAY`        | `3`                      |
| `AUTOXIDE_NEWSLETTER_UNCONFIRMED_DAYS`             | `30`                     |
| `AUTOXIDE_NEWSLETTER_SUBSCRIBE_PER_IP_PER_HOUR`    | `10`                     |
| `AUTOXIDE_NEWSLETTER_SUBSCRIBE_PER_EMAIL_PER_HOUR` | `5`                      |
| `AUTOXIDE_NEWSLETTER_SUBSCRIBE_PER_USER_PER_HOUR`  | `1000`                   |
| `AUTOXIDE_NEWSLETTER_SEND_RATE_PER_MINUTE`         | `300`                    |
| `AUTOXIDE_NEWSLETTER_BATCH_SIZE`                   | `100`                    |
| `AUTOXIDE_NEWSLETTER_SOFT_BOUNCE_LIMIT`            | `3`                      |
| `AUTOXIDE_NEWSLETTER_ADVERTISING_LABEL`            | `[QC]`                   |
| `AUTOXIDE_NEWSLETTER_ADVERTISING_MAX_PER_DAY`      | `3`                      |

`AUTOXIDE_HTTP_PUBLIC_URL` is needed to send. The anti-spam layer (`AUTOXIDE_ANTISPAM_*`) is
shared by every module. All are described in [`.env.example`](../.env.example).
