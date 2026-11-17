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
| Identity verification (KYC) of sellers, livestreamers, affiliates | `/kyc/…`, `/me/kyc…` |
| Account verification for posting (phone / identity) | `/me/verification`, `/me/phone-verifications…` |
| Versioned legal documents and proof of acceptance | `/documents…`, `/me/acceptances`, `/public/documents/{kind}` |
| Consent ledger | `/me/consents` |
| Legal rules with effective dates, tenant policies | `/rules`, `/policies…` |
| **Website legal profile and the Ministry of Industry and Trade (MOIT)** | `/legal-profiles…`, `/websites…`, `/public/badge`, `/public/owner` |

This guide explains the last part, and the mandatory page templates of `content` that go with it.

## The Ministry of Industry and Trade procedure

Vietnamese law (Decree 52/2013/ND-CP as amended by Decree 85/2021/ND-CP; from 1 July 2026 also the
E-commerce Law 122/2025/QH15 and its decrees — check the originals) requires:

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
