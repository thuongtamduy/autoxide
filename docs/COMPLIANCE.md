# Compliance (Vietnam)

[Tiếng Việt](COMPLIANCE.vi.md)

The `compliance` module helps a tenant run a commerce website in Vietnam without missing the basic legal
duties. It is always on (it is not sold by plan). Every legal point that changes over time — lists of
documents, deadlines, which logo domain is accepted — is **data with an effective date** (rules in
`modules/compliance/rules/vn.json`, imported into the database), not code.

> **Not legal advice.** The rules, checklists and templates shipped here were written from secondary
> sources. Every item says `needs_legal_review: true`: check it against the original legal text, with a
> legal professional, before you rely on it.

What the module covers:

| Part | Endpoints (under `/api/v1/compliance`) |
| --- | --- |
| Identity verification (KYC) of sellers, livestreamers, affiliates — profiles, checks and the gate `KycGate`; on a marketplace, `catalog` blocks listing goods of unverified sellers (see below) | `/kyc/…`, `/me/kyc…` |
| Account verification for posting (phone / identity) | `/me/verification`, `/me/phone-verifications…` |
| Versioned legal documents and proof of acceptance | `/documents…`, `/me/acceptances`, `/public/documents/{kind}` |
| Consent ledger | `/me/consents` |
| **Personal data**: purposes and legal bases, retention, processing register, data residency | `/purposes…`, `/retention`, `/processing-register`, `/residency`, `/public/purposes` |
| **Personal data breaches** (72 hours) and **data subject requests** from other channels | `/incidents…`, `/subject-requests…` |
| **Tracking tags and cookie consent** | `/tracking-tags…`, `/cookie-settings/{org_id}`, `/public/consent-config`, `/public/cookie-consents` |
| **Reports of infringements** (intellectual property, counterfeit) and **consumer complaints**, disclosures to authorities | `/cases…`, `/respondent/cases`, `/disclosures`, `/public/reports`, `/public/complaints`, `/public/cases/{id}` |
| Legal rules with effective dates, tenant policies | `/rules`, `/policies…` |
| **Website legal profile and the Ministry of Industry and Trade (MOIT)** | `/legal-profiles…`, `/websites…`, `/public/badge`, `/public/owner` |

This guide explains the Ministry of Industry and Trade procedure with the mandatory page templates of
`content`, then personal data (with the 72-hour breach runbook), reports of infringements with consumer
complaints, and tracking tags with cookie consent.

## The Ministry of Industry and Trade procedure

Vietnamese law (from 1 July 2026 the E-commerce Law 122/2025/QH15 and Decree 248/2026/ND-CP, which
replaces Decrees 52/2013/ND-CP and 85/2021/ND-CP; before that, those two decrees — check the originals)
requires the items below. The rules `moit.checklist.*` version 2 apply from 1 July 2026. A website notified or
registered before then files again under the new decree within the transition period (one summary says 30 June
2027 — not confirmed); electronic identity checks of sellers and livestreamers apply from 1 January 2027. Which
authority receives the files, whether online.gov.vn and the logos stay, is not confirmed yet — check.

- a **sales website** (you sell your own goods) to be **notified** to the ministry — it then shows a
  blue "Đã thông báo Bộ Công Thương" logo;
- a **marketplace or other intermediary** (others sell through you) to be **registered** — red "Đã đăng
  ký Bộ Công Thương" logo; the file includes a service project plan and operating regulations;
- the website to show the owner's information and its policies (general conditions, shipping, payment,
  returns, inspection, personal data protection, complaints).

A person submits on [online.gov.vn](https://online.gov.vn); there is no public API. Autoxide keeps the
data, tells you what is missing, drafts the dossier and shows the logo once you record the result.

### 1. Legal profile of the owner

One profile per organization unit. The tenant's own profile belongs to the root unit (leave `org_id`
out); a seller on a marketplace gets a profile on its unit. A website uses the profile of its own unit,
or of the nearest unit above that has one.

```http
POST /api/v1/compliance/legal-profiles
{
  "entity_type": "organization",
  "legal_name": "Cong ty TNHH Vi Du",
  "tax_code": "0101234567",
  "registration_number": "0101234567",
  "registration_issued_on": "2020-01-02",
  "registration_issued_by": "Phong Dang ky kinh doanh Ha Noi",
  "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Trang Tien"},
  "representative_name": "Nguyen Van A",
  "phone": "024 1234 5678",
  "email": "lienhe@shop.example"
}
```

- `entity_type`: `individual`, `household_business` or `organization`.
- The address is checked against the administrative units of the `address` module (import them first:
  `autoxide task address.import`).
- `PATCH /legal-profiles/{id}` changes some fields (`""` clears a text, `null` clears `address`); send
  `If-Match` with the `ETag` you read.
- `kyc_profile_id` links the owner's identity verification; a marketplace checklist needs it `verified`.

Permissions: `compliance.legal.read`, `compliance.legal.manage`. Both have a scope: a seller's staff
see and change only their unit's profile and websites.

### 2. Website

```http
POST /api/v1/compliance/websites
{"domain": "shop.example", "kind": "sales"}
```

- `kind`: `sales` (notification) or `platform` (registration). Changing `kind` later starts the
  procedure again.
- `channel`: `website` (default) or `mobile_app`.
- `org_id`: the unit of the website (e.g. a seller's own store); left out, the whole tenant.

### 3. Mandatory pages

Write each page in `content` — the templates in the next section give a starting text — publish it, then
link it to the website:

```http
PUT /api/v1/compliance/websites/{id}/pages/return_policy
{"entry_id": "<id of the content entry>"}
```

Page kinds: `owner_info`, `privacy_policy`, `return_policy`, `shipping_policy`, `payment_policy`,
`inspection_policy`, `complaint_policy`, `terms_of_use`, `general_conditions`, and for a marketplace
`platform_rules`, `seller_agreement`. A linked page counts only once it is published.

### 4. Checklist

`GET /api/v1/compliance/websites/{id}/checklist` (MCP `compliance.website_checklist`) lists every item of
the rule `moit.checklist.sales` or `moit.checklist.platform` in force, each with a `status`:

| Status | Meaning |
| --- | --- |
| `met` | Done. |
| `missing` | To do; `hint` says which endpoint to call. |
| `unknown` | Autoxide cannot tell yet (e.g. no installed module answers "each product page shows its seller") — check it yourself. |
| `manual` | Always checked by a person (e.g. your account on online.gov.vn). |

Each item carries its `legal_basis`. When the rules change, platform staff import a new version
(`POST /api/v1/compliance/rules`) and the checklist follows — no upgrade needed.

**Selling before the procedure is done** is shown as "required by law" (`selling_before_done: "label"`),
not blocked: the sources found treat it as an administrative offence with fines, not as a ban on
transactions. Check this with a legal professional (decision D-358).

### 5. Dossier

`GET /api/v1/compliance/websites/{id}/dossier` (MCP `compliance.website_dossier`) returns, filled from
your data:

- `form`: the declaration fields (owner, website, identity verification, linked pages with full URLs,
  complaints contact);
- `documents`: Markdown drafts in Vietnamese — the declaration; for a marketplace also a draft service
  project plan and draft operating regulations;
- `checklist` and a `disclaimer`.

Complete and review the drafts with a legal professional, then submit on online.gov.vn.

### 6. Status and logo

Record each step by hand:

```http
POST /api/v1/compliance/websites/{id}/status
{"status": "submitted", "reference": "HS-001"}
```

Statuses: `not_submitted`, `submitted`, `needs_supplement`, `notified` (sales website), `registered`
(marketplace), `rejected`, `revoked`. For `notified` / `registered`, paste the logo code the ministry
gave in `snippet` (or send `link_url` and `image_url`). Only a link to `online.gov.vn` around an image
from `online.gov.vn` is accepted — scripts, event handlers, styles or other domains give
`compliance.logo_snippet_invalid` (422). The code is never stored or returned as sent: Autoxide keeps
the two addresses and rebuilds a clean snippet.

### 7. For the interface (public, no sign-in)

| Endpoint | Returns |
| --- | --- |
| `GET /api/v1/compliance/public/badge` | The logo of the website being viewed: `color` (`blue` / `red`), `link_url`, `image_url`, `alt`, ready `snippet`. 404 while the website has no logo. |
| `GET /api/v1/compliance/public/owner` | The owner's information for the footer. 404 without a legal profile. |

The website is found by `?domain=`, then the request's domain, then the website of the unit being viewed
(`?site=<unit id>` or a domain bound to a unit). Both send an `ETag`; send it back in `If-None-Match`
to get `304`.

### 8. `autoxide doctor`

The check `compliance.moit_readiness` warns (never fails) when no website has a legal profile yet, or
when a website not yet submitted still lacks checklist items — with the checklist address of each.

## Mandatory page templates (`content`)

The `content` module ships starting texts for the mandatory pages, in English and Vietnamese:

```http
GET  /api/v1/content/templates
POST /api/v1/content/templates/{key}/drafts
{"locale": "vi", "variables": {"policy": {"return_days": 7}}}
```

- Keys: `owner_info`, `privacy_policy`, `return_policy`, `shipping_policy`, `payment_policy`,
  `inspection_policy`, `complaint_policy`, `terms_of_use`, `general_conditions`; for a marketplace
  `platform_rules`, `seller_agreement`, `seller_info`.
- The draft is filled with the legal profile of the unit (`owner.*`, `contact.*`, read from `compliance`),
  then with the `variables` you send (they win). Every value goes into text only — it can never add markup.
  A required variable still missing gives `content.template_variables_missing` (422) listing them all.
- The result is a **draft** page (`content.write`); review it, publish it, then link it to the website.
- Every template starts with "Sample content — have a legal professional review it before use".

## Personal data

Personal Data Protection Law 91/2025/QH15 and Decree 356/2025/ND-CP (in force from 1 January 2026 — check
the originals with a legal professional). Each tenant is the data controller of its own data.

### Purposes and legal bases

`GET /purposes` (`compliance.privacy.read`) lists the purposes of processing: the defaults of the legal rule
`privacy.default_purposes` (`service`, `marketing`, `analytics`, `legal`) merged with the tenant's own.
`POST /purposes` adds one, `PATCH /purposes/{key}` changes one (changing a default stores the tenant's
version) — both need `compliance.privacy.manage`. Each purpose has a `category`, a `legal_basis`
(`consent`, `contract`, `legal_obligation`, `vital_interest`, `public_interest`), the kinds of data, the
recipients (name, country) and a description by language. `GET /public/purposes` (no sign-in) lists the
purposes in use for the privacy notice; active tracking providers appear there as recipients, with the
country that receives the data.

### Retention

Every module declares the personal data it keeps and for how long (`Module::retention`, read from its
settings). `GET /retention` lists them, each compared with the legal rule `privacy.retention_limits`:
`ok`, `exceeds_limit` or `forever_not_allowed` (only `accounting` — proof the law requires — may be kept
forever). The limits shipped are cautious defaults, not numbers from the law. `autoxide doctor` warns
(`compliance.retention`) when an item is not `ok`; lower the setting named in `setting`.

### Processing register

`GET /processing-register` returns purposes, legal bases, kinds of data, recipients, retention and where the
data is kept — the input of a data protection impact assessment, whose official form needs a legal
professional.

### Not covered yet (before 1.0)

Findings of the first legal review (AI-performed, not legal advice; `docs/legal/review-2026-11-r1/`) that core does
not handle yet. Until they are done, the deployer covers them:

- **Seller identity before selling.** The legal rule `kyc.required_activities` asks for verified sellers,
  livestreamers and affiliates (E-commerce Law 122/2025/QH15; Decree 248/2026/ND-CP Article 18, electronic identity
  of sellers from 1 January 2027 — check the originals). When the tenant is a marketplace
  (`kernel.business_model = marketplace`), `catalog` asks `KycGate` before a product of a seller (any branch other
  than the tenant's root) goes `active`. The seller is the owning branch or a branch above it, below the root
  (marketplace → seller → store): one verified branch in that chain is enough. An unverified seller gets
  `compliance.kyc_required` (403); goods of the root and tenants that sell their own goods are not asked. If module
  `compliance` is turned off (for example `AUTOXIDE_MODULES=official,-compliance`), identity cannot be checked, so
  goods of a seller cannot go on sale on a marketplace (`catalog.seller_verification_unavailable`, 403). Goods
  already on sale are checked again: when a seller's verification expires or is revoked (`compliance.kyc_expired`,
  `compliance.kyc_revoked`), when a branch moves in the tree, and every day (job `catalog.seller_kyc_sweep`: the
  tenant became a marketplace, the rule started to apply). Goods of a seller who no longer passes go `inactive`
  (event `catalog.product_status_changed`); after verifying again the seller puts them on sale again. A marketplace
  that gives each seller its own child tenant instead of a branch is not checked this way: each child tenant sells
  its own goods. From 1 January 2027 (rule `kyc.required_activities` version 2) a seller must also have the items
  of Article 18 on the profile to sell — individuals: date of birth (name and personal identification number are
  already kept); organisations and household businesses: head office and legal representative; a passport must be
  valid at least 6 more months counted from the verification, with its issuing country. Missing items give
  `compliance.kyc_required` with `profile_status` `incomplete` and the missing names in `missing`. The eKYC
  provider returns them, or a reviewer records them with `PUT /api/v1/compliance/kyc/profiles/{id}/details`
  (`compliance.kyc.review`); a profile never stays verified past the document's end date. **Before 1 January
  2027**, on a marketplace: no bundled eKYC provider returns these items yet, so sellers verified earlier lack them
  and the daily sweep will pause their goods from that day — have a reviewer record them first (list verified
  profiles with `GET /api/v1/compliance/kyc/profiles?status=verified` and look at the new fields). Still open: livestream
  and affiliate flows (no module yet). Check the originals before relying on this.
- **Personal data of children.** Law 91/2025/QH15 sets special rules for children and persons under guardianship
  (consent of a parent or guardian; from age 7 also the child's — the article and ages need checking). Core has no
  birth date, age check or guardian consent: a service aimed at children must collect and keep the guardian's
  consent itself. Planned after 1.0: a per-tenant setting, self-declared age at sign-up and consent given on
  someone's behalf in the consent ledger.
- **Data sent abroad by providers.** The AI providers of module `ai`, mail, SMS and file storage may process
  personal data outside Vietnam; they are not added to the processing register by themselves. Add each provider
  as a recipient (name, country) of the purposes that use it (`POST /purposes`, `PATCH /purposes/{key}`), and
  prepare the impact assessment for transfers abroad (Decree 356/2025/ND-CP — form and exemptions for small
  businesses need checking). Planned after 1.0: modules declare their processors (`Module::processors`) and
  `autoxide doctor` warns about recipients abroad.

### Data residency

A tenant can be required to keep its data in one country (Cybersecurity Law 116/2025/QH15, in force 1 July
2026, replacing the Cybersecurity Law 2018 and the Network Information Security Law 2015 — secondary sources,
to check; its implementing
decree, reported as Decree 333/2026/ND-CP, replaces Decree 53/2022/ND-CP — not confirmed. Who must keep data in
Vietnam needs a legal check):

```bash
autoxide placement country main VN          # record where a placement's servers are
```

```http
PUT /api/v1/compliance/residency
{"tenant_id": "…", "data_residency": "VN"}
```

Only platform staff with `compliance.privacy.manage` set it. It is refused (`tenant_residency_violation`,
409) while the tenant's placement is not in that country or its country is unknown; moving the tenant later
to a placement elsewhere is refused with the same code, and so is changing the placement's country. `GET
/residency` shows the tenant's state; `autoxide doctor` warns (`compliance.data_residency`) about any tenant
not met.

### Personal data breaches: the 72-hour runbook

The report to the authority is due **72 hours after detection** (legal rule `privacy.incident`, data with an
effective date). Do this as soon as a breach is suspected:

1. **Record it at once** — `POST /incidents` with `title`, `severity` (`low`…`critical`) and `detected_at`
   (when it was detected, not when it started). The answer gives `report_due_at`. Needs
   `compliance.incidents.manage`.
2. **Set who is reminded** — the tenant policy `incident.alert_emails` (`PUT /policies/incident.alert_emails`
   with a list of addresses). A job every 15 minutes sends a mail at 24, 48, 60 and 68 hours and at the
   deadline (each once), and emits `compliance.incident_deadline_near`.
3. **Contain and log** — `PATCH /incidents/{id}` with `status: "contained"`, the kinds of data and the
   number of people; add each finding and action with `POST /incidents/{id}/entries` (the log only grows).
4. **Draft the notifications** — `GET /incidents/{id}/draft?audience=authority&locale=vi` (and
   `audience=subjects` for the people affected). These are drafts filled from the incident and the
   `privacy.contact` policy: complete the missing parts, have them reviewed, send them yourself.
5. **Record the report** — `PATCH /incidents/{id}` with `authority_notified_at` and `authority_reference`
   (the breach becomes `reported`), then `subjects_notified_at` when people were told; `closed` at the end.

`GET /incidents?overdue=true` lists breaches past their deadline, and `autoxide doctor`
(`compliance.privacy_deadlines`) warns about them. The deployment guide points here
([DEPLOY.md](DEPLOY.md)).

### Requests of data subjects received through other channels

A request a user makes in their account (export, delete) runs at once. A request received by email, post,
phone or from an authority is recorded so it is not lost:

- `POST /subject-requests` with `kind` (`access`, `erasure`, `rectification`, `objection`, `restriction`,
  `withdrawal`), `channel` (`email`, `post`, `phone`, `authority`, `other`), `subject` (a user id, an email
  address or a phone number) and `received_at`. `due_at` comes from the legal rule `privacy.subject_requests`
  in force when it was received: 72 hours, stricter than the 2 working days to answer of Decree 356/2025/ND-CP
  Article 5 (which replaced Decree 13/2023/ND-CP). `fulfil_due_at` is the deadline to carry the request out:
  `fulfil_within_days` of the kind in the rule (version 3: 15 days for withdrawal, restriction, objection; 10 for
  access, rectification; 20 for erasure — from secondary sources), counted in calendar days (`fulfil_unit`) since
  the sources do not say whether working days are meant; the longer deadline with a third-party processor is not
  applied. A request is overdue while still `received` past `due_at`, or not done past `fulfil_due_at` — check the
  original texts.
- `POST /subject-requests/{id}/fulfil`: `access` of a user returns the personal data of every module (send
  it to the person; it is not stored); `erasure` / `withdrawal` of an email or phone number empties it from
  the consent ledger (the hashed key and the records stay as proof, so nothing is sent again). Erasing a
  whole account is `DELETE /api/v1/auth/users/{id}`.
- `PATCH /subject-requests/{id}` sets `in_progress` or `rejected` (with a `reason`). `GET
  /subject-requests?overdue=true` lists late ones. All need `compliance.privacy.manage`.

When an account is deleted (`auth.user_deleted`), `compliance` deletes its verified phone number and
pending codes and removes the user from cookie consent records; proof of consents, accepted documents and
the identity verification result stay for the time the law requires. The personal data export of a user
(`GET /api/v1/auth/me/export`) includes the `compliance` part.

## Reports of infringements and consumer complaints

Intellectual property infringements, counterfeits and prohibited goods are reported as **cases**;
so are consumer complaints. A case goes: intake → (hide the target) → answer of the reported party →
decision, with deadlines and a log that only grows. Basis — check the original texts: E-commerce Law
122/2025/QH15 and Decree 248/2026/ND-CP (from 1 July 2026; take-down on request of an authority, Decree
85/2021/ND-CP before), Intellectual Property Law, Consumer
Protection Law 19/2023/QH15 and Decree 55/2024/ND-CP (complaints).

| Who | Endpoints (under `/api/v1/compliance`) |
| --- | --- |
| Anyone (public form, tenant from the domain) | `POST /public/reports`, `POST /public/complaints`, `GET /public/complaints`, `GET /public/cases/{id}?token=`, `GET /public/moderation-reasons` |
| Staff (`compliance.cases.read` / `.handle`) | `GET\|POST /cases`, `GET /cases/{id}`, `POST /cases/{id}/entries`, `…/restrict`, `…/restore`, `…/decision`, `…/close` |
| The reported party (signed in) | `GET /respondent/cases`, `POST /cases/{id}/responses` |
| Staff (`compliance.disclosures.manage`) | `GET\|POST /disclosures` |

**Intake.** The public forms ask a form token first (`GET /api/v1/antispam/token?scope=compliance:reports`,
or `compliance:complaints`) and are limited per network block and email per hour
(`AUTOXIDE_COMPLIANCE_PUBLIC_REPORTS_PER_IP_PER_HOUR`, default 5). The target is the reported object as
`<module>.<type>:<id>` (e.g. `content.entry:…`, `comments.comment:…`; a product once step 41 exists);
it must exist. The sender gets `tracking_token` once — only its SHA-256 is kept — and a receipt mail
when they gave an address (link: `AUTOXIDE_COMPLIANCE_CASE_TRACKING_URL` with `{case}` and `{token}`).
Staff record cases received by letter (`POST /cases`, `reporter_kind` `rights_holder` or `authority`).

**Deadlines** come from the legal rule `cases.deadlines`: hours to acknowledge, working days to answer
and to decide by kind (consumer complaints: 3 and 7), and 24 hours to decide a request of an authority.
Working days skip Saturdays, Sundays and the holidays listed in the rule (Vietnam time); lunar new year
and make-up days are entered each year. The hourly schedule `compliance.case_deadlines` sends the event
`compliance.case_overdue` and a mail to the `complaints.contact` policy's `email`, once per deadline.

**Hiding the target.** A request of an authority hides the target at once; so does an intellectual
property or counterfeit report when the tenant sets the policy `cases.auto_restrict` to `true`. Staff
hide or show it by hand (`/restrict`, `/restore`). `compliance` never touches the tables of other
modules: it sends `compliance.target_restricted` / `compliance.target_restored` (kernel contract
`autoxide_kernel::moderation`), and the owning module hides the object — `content` answers **451**
`content.restricted` and leaves the page out of lists, sitemap, feed and search; `comments` hides the
comment and restores its former status. The target is shown again only when no other case still hides it.

**The reported party** is the owner of the target (from the kernel's resource directory, e.g. the author
of the page) and anyone with `compliance.cases.respond` on the reported unit (`respondent_org_id`, a
seller on a marketplace). They see their cases without the reporter's contact details and answer before
`respond_by`; later gives `compliance.response_closed` (409).

**Decision.** A signed-in person with `compliance.cases.handle` decides (`upheld` or `partially_upheld`
keep the target hidden; `rejected` shows it again); API keys get `compliance.session_required`. The
reason is shown to both sides and mailed to the reporter. There is no MCP tool for deciding.

**Disclosures to authorities.** `POST /disclosures` (signed-in person, `compliance.disclosures.manage`)
records the authority, its request number, the legal basis and the names of the fields given, and
returns the values once — from the seller's legal profile and identity verification kept by
`compliance`. The record and the event `compliance.disclosure_recorded` never hold the values.

MCP tools: `compliance.list_cases`, `compliance.get_case` (read), `compliance.add_case_note`,
`compliance.restrict_case_target`.

### Consumer protection

- **Hiding reviews.** Content of consumers may be hidden only for the reasons of the legal rule
  `moderation.hide_reasons` (`spam`, `illegal_content`, `personal_data`, `unrelated`, `offensive` — to
  check). Modules that moderate reviews ask the trait `ModerationPolicy` (`autoxide_kernel::moderation`)
  before hiding — `orders` maps its own reasons (`unlawful_content` → `illegal_content`, `abusive_language` →
  `offensive`, `spam_or_advertising` → `spam`, `not_about_product` → `unrelated`), so a reason a tenant adds
  to its list cannot hide a review while `compliance` is installed
  (`compliance.hide_reason_not_allowed`, 422) and record the reason in their own event; the list is
  public at `GET /public/moderation-reasons`. A negative but lawful review stays visible.
- **General conditions of trade** are a versioned legal document of kind `general_conditions`, public at
  `GET /public/documents/general_conditions`.
- **Complaint channel.** `GET /public/complaints` gives the website's complaint page the contact (policy
  `complaints.contact`) and the deadlines; complaints are cases of kind `consumer_complaint`.

### AI labels

Content written or edited from AI suggestions carries `ai_assisted` (`content`) and image alt texts from
suggestions carry `alt_ai_assisted` (`media`), returned by the public endpoints so the interface labels
them (Law on Artificial Intelligence 134/2025/QH15, from 1 March 2026 — check the decree). See
[`CONTENT.md`](CONTENT.md) and [`MEDIA.md`](MEDIA.md).

## Tracking tags and cookie consent

A tenant adds the measurement tags of advertising platforms without pasting scripts, and they run only after
the visitor consents. (Personal Data Protection Law 91/2025/QH15, Decree 356/2025/ND-CP, Consumer Protection
Law 19/2023/QH15 — whether analytics cookies need consent, and the impact assessment for transfers abroad,
need a legal check; the design takes the strict reading: ask first.)

### Providers and tags

`GET /tracking-providers` lists the built-in providers; the tenant only enters the ID:

| `provider` | Name | Category | ID form | Data goes to |
| --- | --- | --- | --- | --- |
| `ga4` | Google Analytics 4 | `analytics` | `G-XXXXXXXXXX` | US |
| `gtm` | Google Tag Manager | `marketing` | `GTM-XXXXXXX` | US |
| `google_ads` | Google Ads | `marketing` | `AW-123456789` | US |
| `meta` | Meta Pixel | `marketing` | digits | US |
| `tiktok` | TikTok Pixel | `marketing` | letters and digits | SG |
| `zalo` | Zalo Ads Pixel | `marketing` | digits | VN |

`POST /tracking-tags {org_id, provider, tracking_id}`, `PATCH /tracking-tags/{id}` (`status`: `active` /
`paused`), `DELETE /tracking-tags/{id}` — `compliance.tracking.manage` on the website's unit. A wrong ID gives
`compliance.tracking_id_invalid` with the expected form; an unknown provider
`compliance.tracking_provider_unknown`.

### Banner

`PUT /cookie-settings/{org_id}` sets `enabled`, the optional `categories` asked about (`preferences`,
`analytics`, `marketing`; `necessary` is always on and carries no tag) and `position`. Categories of active
tags are always asked. Changing the categories or the active providers raises `policy_version`: visitors are
asked again. `GET /cookie-consents/stats?org_id=` gives totals of the last 30 days, never single records.

### For the interface

```http
GET  /api/v1/compliance/public/consent-config?granted=analytics&locale=vi
POST /api/v1/compliance/public/cookie-consents
{"consent_id": "<random, kept in a necessary cookie>", "categories": ["analytics"], "policy_version": 3}
```

The contract with the interface:

- **Nothing loads before consent.** Without `granted` the config returns no tag; with it, only the active
  tags of those categories. Read the visitor's choice from your necessary cookie and pass it.
- **Reject is as easy as accept.** Show `texts.reject_all` as prominently as `texts.accept_all`, on the first
  layer; no category is preselected (`preselected: false`). `texts.legal` is not overridable.
- **Record the choice** with `POST /public/cookie-consents` (no sign-in; limited per network block by
  `AUTOXIDE_COMPLIANCE_COOKIE_CONSENTS_PER_IP_PER_HOUR`). It stores the random id, the categories, the version
  and a hash of the network block — never the raw IP — for the days of `privacy.consent_proof_days`. A
  signed-in visitor's choice also goes to the consent ledger (purposes `analytics` / `marketing`, scope
  `website:<unit id>`). An old `policy_version` gives `compliance.cookie_policy_changed` (409): show the banner
  again.
- **Do not slow the page.** Load tags `async`, after the page has rendered (`loading:
  after_consent_deferred`); add `csp_hosts` to the Content-Security-Policy; never load tags on sign-in or
  account pages; on checkout pages send only the standard conversion events (`view_item`, `add_to_cart`,
  `begin_checkout`, `purchase`).
- Sending conversions from the server to ad platforms is not built (it needs its own legal check on
  personal data transfers).
