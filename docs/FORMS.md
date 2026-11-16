# Forms

*Tiếng Việt: [FORMS.vi.md](FORMS.vi.md)*

The `forms` module keeps form definitions, takes submissions from the public (contact forms,
sign-ups for a consultation, simple surveys), checks every answer on the server, keeps them
in an admin inbox, mails the people in charge, and exports or erases them. It is API only:
the website shows the form and sends it; the admin screens are a separate project. It lives
under `/api/v1/forms`. The full contract is in [`openapi.json`](openapi.json).

No third-party account is needed: spam is stopped by the core's shared anti-spam layer (a
hidden trap field, a signed form token, rate limits, duplicate detection; an optional
self-hosted proof-of-work challenge).

## Permissions

| Permission     | Allows                                                                       |
| -------------- | ---------------------------------------------------------------------------- |
| `forms.read`   | Reading forms and their submissions (personal data)                          |
| `forms.manage` | Creating, changing, opening, closing and deleting forms                       |
| `forms.review` | Marking submissions read, archived, spam or not spam; deleting one           |
| `forms.export` | CSV export; exporting and erasing every submission of an email address       |
| `forms.submit` | Sending submissions without the anti-spam checks for anonymous visitors (a website server's own API key) |

People who fill in a form need none: everything under `/api/v1/forms/public/` is open.

- Setting or changing `notify_emails` needs `forms.read` as well as `forms.manage`: the
  notification mails carry the answers, so someone who may not read submissions cannot send
  them to their own inbox.
- Give `forms.submit` only to the API key of a website server; being signed in is not enough
  to skip the anti-spam checks (see [The website](#the-website)).

## Define a form

```bash
curl -X POST http://localhost:8080/api/v1/forms \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -H "Idempotency-Key: $(uuidgen)" \
  -d '{"key": "contact", "name": "Contact us",
       "fields": [
         {"key": "name", "type": "text", "label": {"en": "Name", "vi": "Tên"}, "required": true},
         {"key": "email", "type": "email", "label": {"en": "Email"}, "required": true},
         {"key": "message", "type": "textarea", "label": {"en": "Message", "vi": "Lời nhắn"}},
         {"key": "agree", "type": "consent", "required": true,
          "label": {"en": "You may contact me about my request.", "vi": "Tôi đồng ý được liên hệ."}}
       ],
       "allowed_origins": ["https://www.example.com"],
       "notify_emails": ["sales@example.com"],
       "success_message": {"en": "Thank you!", "vi": "Cảm ơn bạn!"}}'
```

- **Field types** (closed list): `text`, `email`, `phone`, `textarea`, `number` (`min`,
  `max`), `select`, `radio`, `checkbox` (several values), `date` (`YYYY-MM-DD`), `hidden`
  (filled by the page, e.g. a campaign tag), `consent` (a box that must not be ticked in
  advance; its label is the text shown). `text`, `textarea` and `hidden` take `min_length` /
  `max_length`. Labels, help texts, placeholders and option labels are `{"en": …, "vi": …}`.
- Every problem of a definition is reported at once (`fields[2].options`…).
- **Versions.** Changing `fields` makes a new `version`. Each submission keeps the version it
  was sent with and is always shown with that version's labels (`answers[]`, and for choice
  fields the chosen options with their labels in `answers[].choices`), so old submissions stay
  readable after the form changes.
- `PATCH` and `DELETE` of a form need `If-Match` (the `ETag` you read). Closing a form
  (`{"status": "closed"}`) keeps its submissions; deleting it deletes them too.
- `notify_emails` are mailed for each new submission, with `Reply-To` set to the sender's
  address (left out if the mail service cannot use that address). They are never shown by the
  public endpoints, and recipients never come from what a visitor sends. Setting them needs
  `forms.read` too.
- Email answers are checked the way the mail service counts them: at most 64 bytes before the
  `@` and 254 bytes in all, measured in UTF-8 after lower-casing (`é` takes two bytes).

## The website

1. `GET /api/v1/forms/public/contact?locale=vi` — the fields with texts in one language, the
   `token_scope` and the success message. No admin settings are included.
2. Each time the form is shown (never in cached HTML):
   `GET /api/v1/antispam/token?scope=forms:contact` — a form token. Show a hidden trap input
   that people leave empty.
3. `POST /api/v1/forms/public/contact/submissions`:

```json
{"data": {"name": "Ada", "email": "ada@example.com", "message": "Hello", "agree": true},
 "locale": "vi", "page_url": "https://www.example.com/contact",
 "form_token": "…", "trap": ""}
```

- The answer is `201 {"data": {"id": …, "message": "Cảm ơn bạn!"}}` — the **same** whether the
  submission was accepted, held as spam (sent too fast, duplicate) or dropped (trap filled,
  too many from one network or address): bots learn nothing.
- Wrong answers: `422` with every problem in `errors` (`data.email` → `email`, unknown keys →
  `unknown`; past 50 unknown keys, the rest are counted in one `data` entry). A missing or expired token: `400 antispam.token_*` — reload the form. A closed
  form: `409 forms.closed`. A page whose `Origin` is not in `allowed_origins`: `403
  forms.origin_not_allowed` (an empty list allows any site).
- Sending the same `form_token` again (double click, flaky network) returns the first `id`
  and stores nothing new.
- A website server may send the form with its own API key instead, if that key has the
  permission `forms.submit`: no token needed, limited to
  `AUTOXIDE_FORMS_SUBMISSIONS_PER_USER_PER_HOUR`; use an `Idempotency-Key` header. Any other
  signed-in caller (for example a self-registered account) goes through the same checks as an
  anonymous visitor: without a form token it gets `400 antispam.token_missing`.

## The inbox

- `GET /api/v1/forms` and `GET /api/v1/forms/{id}` give each form's `submissions`: how many are
  `new`, `read`, `archived` and `spam`.
- `GET /api/v1/forms/{id}/submissions?status=&q=` — newest first, `spam` left out unless
  asked; `q` searches the answers ignoring case and accents.
- `PATCH /api/v1/forms/submissions/{id}` `{"status": "read" | "archived" | "spam" | "new"}`
  with `If-Match`. Moving a submission out of `spam` mails the people in charge as for a new
  one.
- `GET /api/v1/forms/{id}/submissions/export?format=csv` — CSV for spreadsheets. Cells that
  start with `=`, `+`, `-`, `@`, tab or carriage return get a leading `'` so they are never run
  as formulas.

### Large exports

For a form with many submissions, export in the background instead of holding a request open:

1. `POST /api/v1/forms/{id}/submissions/exports` (body optional: `{"status": "new"}` like the
   `status` of the export above) answers `201` at once, with the export's address in `Location`.
   A form has one export running at a time; asking again meanwhile answers `409`
   `forms.export_in_progress`.
2. `GET /api/v1/forms/exports/{export_id}` gives `state`: `pending`, `running`, then `ready` (or
   `failed`: ask for a new one). Once `ready`, `download_url` is a link to the file — the same
   content as the export above, byte for byte.
3. The link needs no sign-in, so it can be opened in a browser; it works for
   `AUTOXIDE_FORMS_EXPORT_LINK_MINUTES` (default 60). Do not share it. Ask for the export again
   for a fresh link. It is an absolute URL when `AUTOXIDE_HTTP_PUBLIC_URL` is set.

Both need `forms.export`. The file is written to the shared file storage (module `files`, local
disk or S3) under `forms/exports/` and deleted after `AUTOXIDE_FORMS_EXPORT_KEEP_HOURS` (default
24) by the daily cleanup job.

## Personal data

- `GET /api/v1/forms/data?email=…` lists, and `DELETE /api/v1/forms/data?email=…` deletes for
  good, every submission of an address, in every form (permission `forms.export`) — for
  requests of the person the data is about. A submission counts when the address is its
  sender's email, or when any answer is exactly that address (ignoring case), such as a
  second email field ("your friend's email"). An address only mentioned inside a longer answer
  (a message) is not found: search the inbox with `q` for those.
- Network addresses are never stored: only a keyed hash of the network block, cleared after
  `AUTOXIDE_FORMS_IP_RETENTION_DAYS`.
- A ticked consent box stores the exact text shown, the form version, the time and the source.
- Events (and so webhooks) carry ids and the form key only, never answers or addresses; other
  systems read details through the API with a permission.
- When an account is deleted, forms it created stay; `created_by` becomes `null`.
- Deleting a form deletes its submissions. A submission whose form no longer exists is
  deleted by the daily cleanup job.
- Large exports hold personal data. Deleting the submissions of an address, or deleting a form,
  also deletes every export file of the forms concerned. Export files left are deleted after
  `AUTOXIDE_FORMS_EXPORT_KEEP_HOURS`. Backups of the file storage copy `forms/exports/` too
  while the files exist. The event `forms.submissions_exported` records who exported which
  form (for the audit log), without the file.

## Legal

Retention times are settings, not rules written in the code. Whether a contact form needs
consent, how long submissions may be kept, and how fast data requests must be answered depend
on the law that applies to you (for example Vietnam's Personal Data Protection Law of 2025 and
Decree 356/2025): **check the original texts for your case** and set
`AUTOXIDE_FORMS_RETENTION_DAYS` (or the tenant's `forms.retention_days`, or each form's
`retention_days`) accordingly.

## AI agents (MCP)

Tools, with the same permissions and checks as REST:

- `forms.list`, `forms.get` (`forms.read`); `forms.create` and `forms.update` (`forms.manage`;
  setting `notify_emails` also needs `forms.read`).
  `forms.get` returns the form with an `etag`; pass it back as `etag` to `forms.update`, like
  `If-Match` (it is not the form's `version`, which counts changes to the fields). Close a
  form with `forms.update` and `status: closed`.
- `forms.list_submissions`, `forms.get_submission` (`forms.read`) and
  `forms.set_submission_status` (`forms.review`; pass the submission's `version` you read).

There is no tool to delete a form (that also deletes its submissions, and cannot be undone;
use REST) and no tool to send a form.

## Settings

| Setting                                         | Default |
| ----------------------------------------------- | ------- |
| `AUTOXIDE_FORMS_MAX_SUBMISSION_BYTES`           | `65536` |
| `AUTOXIDE_FORMS_MAX_FIELDS`                     | `50`    |
| `AUTOXIDE_FORMS_SUBMISSIONS_PER_IP_PER_HOUR`    | `10`    |
| `AUTOXIDE_FORMS_SUBMISSIONS_PER_EMAIL_PER_HOUR` | `5`     |
| `AUTOXIDE_FORMS_SUBMISSIONS_PER_USER_PER_HOUR`  | `1000`  |
| `AUTOXIDE_FORMS_RETENTION_DAYS`                 | `0` (keep) |
| `AUTOXIDE_FORMS_SPAM_RETENTION_DAYS`            | `30`    |
| `AUTOXIDE_FORMS_IP_RETENTION_DAYS`              | `90`    |
| `AUTOXIDE_FORMS_NOTIFY_LANGUAGE`                | `en`    |
| `AUTOXIDE_FORMS_EXPORT_LINK_MINUTES`            | `60`    |
| `AUTOXIDE_FORMS_EXPORT_KEEP_HOURS`              | `24`    |

A form may set its own `ip_limit_per_hour`, `email_limit_per_hour` and `retention_days`. The
anti-spam layer itself (`AUTOXIDE_ANTISPAM_*`) is shared by every module. All are described in
[`.env.example`](../.env.example).

With several tenants, a tenant can replace `AUTOXIDE_FORMS_RETENTION_DAYS` with its own
`forms.retention_days` (`0` to `36500`; its administrator with
`PUT /api/v1/kernel/settings/forms.retention_days`, permission `kernel.settings.manage`; platform
staff with `PUT /api/v1/kernel/tenants/{id}/settings/forms.retention_days`). The order is: the
form's `retention_days`, then the tenant's setting, then the server's. A wrong value is refused when
it is set; `DELETE` the setting to go back to the server's value.
