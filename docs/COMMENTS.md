# Comments

*Tiếng Việt: [COMMENTS.vi.md](COMMENTS.vi.md)*

The `comments` module lets people comment on anything the website shows — a post, a page, a
media item — by its kind and id (`target_type`, `target_id`). Threads have two levels: a
top-level comment and its replies (a reply to a reply joins the same thread and remembers whom
it answers). Signed-in users and guests can comment; every comment goes through a fixed chain
of checks and, by default, guests wait for approval. People can report comments, moderators
approve, reject, mark as spam, ban and close targets, and authors may change their comment for
a short while and delete it at any time. It is API only: the website shows the comment box;
the admin screens are a separate project. It lives under `/api/v1/comments`. The full contract
is in [`openapi.json`](openapi.json).

No third-party account is needed: spam is stopped by the core's shared anti-spam layer (a
hidden trap field, a signed form token, rate limits, duplicate detection; an optional
self-hosted proof-of-work challenge) plus the module's own word lists and link limit.

## Permissions

| Permission          | Allows                                                                          |
| ------------------- | ------------------------------------------------------------------------------- |
| `comments.post`     | Commenting while signed in, changing and deleting one's own comments, one's display name |
| `comments.read`     | Reading every comment in any status, with author emails, network fingerprints and reports |
| `comments.moderate` | Approving, rejecting, marking as spam, holding again; deleting for good          |
| `comments.manage`   | Banning and unbanning; opening, closing and hiding targets                      |
| `comments.export`   | Exporting and erasing every comment written with an email address              |

Guests need none: everything under `/api/v1/comments/public/` is open.

## Targets

`AUTOXIDE_COMMENTS_TARGET_TYPES` (default `content.entry`) lists the kinds that take comments.
When another module resolves a kind (the `content` module resolves `content.entry`), a comment
is only taken when the target exists and is public; other kinds are taken as given. A target
gets its row on its first comment, holding its state (`open` / `closed`, `hidden`,
`closes_at`) and its counts. When `content` archives an entry its comments close; when it
deletes one they are also hidden. `AUTOXIDE_COMMENTS_AUTO_CLOSE_DAYS` closes targets some days
after their first comment.

## The website

1. `GET /api/v1/comments/public/settings` — limits and switches (guests allowed, edit minutes,
   maximum length, whether reply mails are on).
2. `GET /api/v1/comments/public/threads?target_type=content.entry&target_id=…&order=desc&limit=20&replies_limit=3`
   — shown top-level comments with their first replies and `reply_count`; follow
   `pagination.next_cursor`. More replies: `GET /api/v1/comments/public/comments/{id}/replies`.
   Counts for a list page: `GET /api/v1/comments/public/counts?target_type=content.entry&target_ids=a,b,c`
   (up to 100). Whether the box is open: `GET /api/v1/comments/public/targets/{type}/{id}`.
3. Show `body_html` (everything escaped, line breaks as `<br>`, `http(s)` addresses as links
   with `rel="nofollow ugc noopener"`); never insert `body` as HTML. Label guests as guests
   (`author.kind`).
4. Each time the box is shown (never in cached HTML): `GET /api/v1/antispam/token?scope=comments:content.entry`.
   Show a hidden trap input that people leave empty.
5. `POST /api/v1/comments/public/comments`:

```json
{"target_type": "content.entry", "target_id": "0190…", "parent_id": null,
 "body": "Nice post!\nThanks.", "author_name": "Ada", "email": "ada@example.com",
 "notify_replies": true, "locale": "en", "form_token": "…", "trap": ""}
```

- The answer is `201` with the comment, `status` (`published`, or `pending`: waiting for
  approval) and, for guests, an `edit_token` shown only here. A comment held for approval, held
  as spam or dropped by the anti-spam checks all answer `pending`: bots learn nothing.
- Wrong input: `422` with every problem in `errors` (empty body, too long, reserved name, bad
  email…). A banned word: `422 comments.banned_word`. A missing or expired token: `400
  antispam.token_*` — reload the form. A closed target: `409 comments.target_closed`. A banned
  author: `403 comments.banned`.
- Sending the same `form_token` again returns the same comment (with a new `edit_token`; the
  first one stops working) and stores nothing new.
- Signed-in users send their access token instead of a form token, need `comments.post`, and
  first choose a display name with `PUT /api/v1/comments/profile {"display_name": "…"}`.
- Change: `PATCH /api/v1/comments/public/comments/{id} {"body": "…", "edit_token": "…"}` within
  `AUTOXIDE_COMMENTS_EDIT_MINUTES` (the text is checked again; a shown comment with suspect
  words or too many links waits for approval again). Delete: `POST
  /api/v1/comments/public/comments/{id}/delete {"edit_token": "…"}` (signed-in authors send
  `{}`) — text, name and email go, the place in the thread stays.
- Report: `POST /api/v1/comments/public/comments/{id}/reports {"reason": "spam" | "abuse" |
  "illegal" | "other", "note": "…"}`.

## Moderation

The checks run in this order: target closed → length → content (control and invisible
characters refused) → duplicate → bans → banned words (refused) / suspect words (held) → too
many links (held) → anti-spam → approval mode. Modes are `pre` (every comment waits),
`first_time` (until one comment of the **account** was approved) and `post` (shown at once);
defaults are `pre` for guests and `first_time` for users, and both can be set per target kind
(`AUTOXIDE_COMMENTS_MODERATION_BY_TYPE`). A guest's email is never proof of who they are — anyone
can type any address — so it never lets a comment skip approval, even once confirmed.

- `GET /api/v1/comments?status=pending&target_type=&target_id=&q=` — the queue (`q` searches
  text and names ignoring case and accents), with emails, network fingerprints, hold reasons
  and report counts.
- `PATCH /api/v1/comments/{id} {"status": "published" | "rejected" | "spam" | "pending"}` with
  `If-Match`. `DELETE /api/v1/comments/{id}` deletes for good (a top-level comment with its
  thread).
- Reaching `AUTOXIDE_COMMENTS_REPORT_THRESHOLD` reports holds a shown comment once; after a
  moderator approves it, more reports do not hide it. `GET /api/v1/comments/{id}/reports`.
- `POST /api/v1/comments/bans {"kind": "user" | "email" | "ip", "value": "…", "reason": "…",
  "until": null}` (`ip` takes the comment's `ip_fingerprint`), `GET`, `DELETE
  /api/v1/comments/bans/{id}`.
- `PUT /api/v1/comments/targets/{type}/{id} {"state": "closed", "hidden": false, "closes_at":
  null}`.

Every decision is an event (`comments.comment_status_changed`, `comments.comment_deleted`,
`comments.ban_created`, `comments.target_closed`…) that the `audit` module records with who and
when.

## Reply mails

Off until both `AUTOXIDE_COMMENTS_VERIFY_URL` and `AUTOXIDE_COMMENTS_UNSUBSCRIBE_URL` are set.
With `notify_replies: true` and an `email`, a confirmation mail goes to that address (at most
`AUTOXIDE_COMMENTS_VERIFY_MAILS_PER_DAY`); the website page sends its token to `POST
/api/v1/comments/public/verify`. From then on, each shown reply to that comment mails the
address with the target's title and link — never the reply's text or the replier's name, so the
feature cannot carry spam — and a link whose token goes to `POST
/api/v1/comments/public/unsubscribe` (`"all": true` stops every comment followed with that
address).

## Personal data

- `GET /api/v1/comments/data?email=…` lists, and `DELETE /api/v1/comments/data?email=…` clears,
  every comment written with an address (permission `comments.export`): text, name, email and
  network fingerprint go, the place in the thread stays, reply mails stop. Bans of the address
  are kept (they exist to keep it out); lift them by hand if asked.
- Network addresses are never stored: only a keyed hash of the network block, cleared after
  `AUTOXIDE_COMMENTS_IP_RETENTION_DAYS`. Comments waiting for approval or spam are deleted after
  `AUTOXIDE_COMMENTS_RETENTION_DAYS`.
- Events (and so webhooks) carry ids, targets and statuses only, never text, names or addresses.
- When an account is deleted, its comments are cleared the same way, its display name is
  deleted and its reports no longer point to it. The account's data export includes its
  comments, display name and reports.

## Legal

What the law asks of comments depends on where you and your readers are. The module gives you
settings, not rules written in the code; **the texts below were not checked against the
original legal texts — check them for your case before you rely on them.**

- **Vietnam, Decree 147/2024/ND-CP (internet services):** as read from secondary sources, social
  networks may only let accounts verified by a Vietnamese phone number comment; licensed
  general news sites may not be allowed reader comments at all; content that breaks the law must
  be taken down within 24 hours of a request from the authorities, and some services must keep
  moderation records (2 years is mentioned). The module offers `AUTOXIDE_COMMENTS_ENABLED`,
  `AUTOXIDE_COMMENTS_ALLOW_GUESTS`, `AUTOXIDE_COMMENTS_SIGNED_IN_ONLY_TYPES`, closing or hiding
  a target, deleting for good (`DELETE /api/v1/comments/{id}`, recorded with who and when in
  the audit log), and the `illegal` report reason. Phone verification of accounts is not part
  of this module. How long the audit log is kept is set by the `audit` module.
- **Vietnam, Personal Data Protection Law 91/2025/QH15 and Decree 356/2025:** people may ask
  for their data and its erasure; the module exports and clears by email and by account, keeps
  network data only as a hash and for a set time. Check the deadlines for answering requests,
  and whether keeping moderation records after an erasure (ids, actions and times only) is
  enough for you.
- **Elsewhere:** for example the EU GDPR (data subject rights) and the Digital Services Act
  (notice and action for illegal content).

## AI agents (MCP)

Tools `comments.queue` and `comments.get` (`comments.read`), `comments.moderate`
(`comments.moderate`; pass the `version` you read, like `If-Match`) and
`comments.set_target_state` (`comments.manage`), with the same permissions and checks as REST.
There is no tool to write a comment (that is for readers on the website) and no tool to delete
one for good (use REST).

## Settings

| Setting                                    | Default                                     |
| ------------------------------------------ | ------------------------------------------- |
| `AUTOXIDE_COMMENTS_ENABLED`                | `true`                                      |
| `AUTOXIDE_COMMENTS_TARGET_TYPES`           | `content.entry`                             |
| `AUTOXIDE_COMMENTS_ALLOW_GUESTS`           | `true`                                      |
| `AUTOXIDE_COMMENTS_SIGNED_IN_ONLY_TYPES`   | —                                           |
| `AUTOXIDE_COMMENTS_GUEST_MODERATION`       | `pre`                                       |
| `AUTOXIDE_COMMENTS_USER_MODERATION`        | `first_time`                                |
| `AUTOXIDE_COMMENTS_MODERATION_BY_TYPE`     | —                                           |
| `AUTOXIDE_COMMENTS_MAX_LENGTH`             | `4000`                                      |
| `AUTOXIDE_COMMENTS_MAX_LINKS`              | `2`                                         |
| `AUTOXIDE_COMMENTS_EDIT_MINUTES`           | `15`                                        |
| `AUTOXIDE_COMMENTS_REPORT_THRESHOLD`       | `3`                                         |
| `AUTOXIDE_COMMENTS_AUTO_CLOSE_DAYS`        | `0` (never)                                 |
| `AUTOXIDE_COMMENTS_BANNED_WORDS`           | —                                           |
| `AUTOXIDE_COMMENTS_SUSPECT_WORDS`          | —                                           |
| `AUTOXIDE_COMMENTS_RESERVED_NAMES`         | `admin,administrator,moderator,root,system` |
| `AUTOXIDE_COMMENTS_PER_IP_PER_HOUR`        | `20`                                        |
| `AUTOXIDE_COMMENTS_PER_EMAIL_PER_HOUR`     | `10`                                        |
| `AUTOXIDE_COMMENTS_PER_USER_PER_HOUR`      | `60`                                        |
| `AUTOXIDE_COMMENTS_REPORTS_PER_HOUR`       | `30`                                        |
| `AUTOXIDE_COMMENTS_RETENTION_DAYS`         | `30`                                        |
| `AUTOXIDE_COMMENTS_IP_RETENTION_DAYS`      | `90`                                        |
| `AUTOXIDE_COMMENTS_VERIFY_URL`             | — (needed for reply mails)                  |
| `AUTOXIDE_COMMENTS_UNSUBSCRIBE_URL`        | — (needed for reply mails)                  |
| `AUTOXIDE_COMMENTS_SITE_URL`               | — (page links in reply mails)               |
| `AUTOXIDE_COMMENTS_VERIFY_MAILS_PER_DAY`   | `3`                                         |

The anti-spam layer (`AUTOXIDE_ANTISPAM_*`) is shared by every module. All are described in
[`.env.example`](../.env.example).
