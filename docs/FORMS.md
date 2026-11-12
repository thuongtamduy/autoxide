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

People who fill in a form need none: everything under `/api/v1/forms/public/` is open.

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
  was sent with and is always shown with that version's labels (`answers[]`), so old
  submissions stay readable after the form changes.
- `PATCH` and `DELETE` of a form need `If-Match` (the `ETag` you read). Closing a form
  (`{"status": "closed"}`) keeps its submissions; deleting it deletes them too.
- `notify_emails` are mailed for each new submission, with `Reply-To` set to the sender's
  address. They are never shown by the public endpoints, and recipients never come from what a
  visitor sends.

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
  `unknown`). A missing or expired token: `400 antispam.token_*` — reload the form. A closed
  form: `409 forms.closed`. A page whose `Origin` is not in `allowed_origins`: `403
  forms.origin_not_allowed` (an empty list allows any site).
- Sending the same `form_token` again (double click, flaky network) returns the first `id`
  and stores nothing new.
- A website server may send the form with its own API key instead: no token needed, limited
  to `AUTOXIDE_FORMS_SUBMISSIONS_PER_USER_PER_HOUR`; use an `Idempotency-Key` header.

## The inbox

- `GET /api/v1/forms/{id}/submissions?status=&q=` — newest first, `spam` left out unless
  asked; `q` searches the answers ignoring case and accents.
- `PATCH /api/v1/forms/submissions/{id}` `{"status": "read" | "archived" | "spam" | "new"}`
  with `If-Match`. Moving a submission out of `spam` mails the people in charge as for a new
  one.
- `GET /api/v1/forms/{id}/submissions/export?format=csv` — CSV for spreadsheets. Cells that
  start with `=`, `+`, `-`, `@`, tab or carriage return get a leading `'` so they are never run
  as formulas.

## Personal data

- `GET /api/v1/forms/data?email=…` lists, and `DELETE /api/v1/forms/data?email=…` deletes for
  good, every submission sent with an address, in every form (permission `forms.export`) —
  for requests of the person the data is about.
- Network addresses are never stored: only a keyed hash of the network block, cleared after
  `AUTOXIDE_FORMS_IP_RETENTION_DAYS`.
- A ticked consent box stores the exact text shown, the form version, the time and the source.
- Events (and so webhooks) carry ids and the form key only, never answers or addresses; other
  systems read details through the API with a permission.
- When an account is deleted, forms it created stay; `created_by` becomes `null`.

## Legal

Retention times are settings, not rules written in the code. Whether a contact form needs
consent, how long submissions may be kept, and how fast data requests must be answered depend
on the law that applies to you (for example Vietnam's Personal Data Protection Law of 2025 and
Decree 356/2025): **check the original texts for your case** and set
`AUTOXIDE_FORMS_RETENTION_DAYS` (or each form's `retention_days`) accordingly.

## AI agents (MCP)

Tools, with the same permissions and checks as REST:

- `forms.list`, `forms.get` (`forms.read`); `forms.create` and `forms.update` (`forms.manage`).
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

A form may set its own `ip_limit_per_hour`, `email_limit_per_hour` and `retention_days`. The
anti-spam layer itself (`AUTOXIDE_ANTISPAM_*`) is shared by every module. All are described in
[`.env.example`](../.env.example).
