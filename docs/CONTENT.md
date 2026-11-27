# Content

*Tiếng Việt: [CONTENT.vi.md](CONTENT.vi.md)*

The `content` module keeps the posts and pages of a website, in several languages, with
drafts, history, scheduling, categories, tags, authors, menus and redirects. It is API only:
the admin screens and the website itself are separate projects that call it. It lives under
`/api/v1/content`. The full contract, with every parameter and error code, is in
[`openapi.json`](openapi.json) (and at `/api/docs` when `AUTOXIDE_HTTP_API_DOCS=true`).

Images are items of the media library ([`MEDIA.md`](MEDIA.md)): an entry keeps only their
ids, and they must be public before the entry can be published.

## Permissions

| Permission            | Allows                                                                       |
| --------------------- | ---------------------------------------------------------------------------- |
| `content.read`        | Reading drafts, revisions and schedules                                      |
| `content.write`       | Creating and changing drafts, translations, restoring revisions, preview links, asking for a review |
| `content.write_own`   | The same as `content.write`, but only on entries the caller created (same user **and** same API key) |
| `content.publish`     | Publishing, taking down, archiving, moving pages, scheduling; the terms and authors of a published entry |
| `content.publish_own` | Publishing and taking down only entries the caller created; no archive, schedule or moving published pages |
| `content.delete`      | Deleting entries for good                                                    |
| `content.manage`      | Categories, tags, authors, menus, redirects, display placements              |
| `content.banners`     | Banners of display placements, per organization unit                         |

Give them to a role with `PUT /api/v1/auth/roles/{id}`. The `admin` role has them all.
Readers of the website need none: everything under `/api/v1/content/public/` is open.

## The model

An **entry** is a post (`kind: post`) or a page (`kind: page`, can have a parent page). Each
entry has a **draft** — always there, the version editors change — and, while it is
published, a separate **published version** that readers see. Changing the draft never
changes what readers see until it is published again (`has_unpublished_changes` tells).

- `status` is `draft`, `published` or `archived`.
- `slug` is the last part of the address: made from the title when not given (`"Đường đến Đà
  Lạt"` → `duong-den-da-lat`; `-2`, `-3`… when taken), lower-case letters and digits joined by
  `-`, unique among the entries of the same kind, language and parent.
- `body` is a [ProseMirror](https://prosemirror.net) / [Tiptap](https://tiptap.dev) JSON
  document. The server checks it against an allowlist of nodes, marks and attributes
  (paragraphs, headings 2–4, lists, quotes, code, tables, images, embeds from
  `AUTOXIDE_CONTENT_EMBED_HOSTS`; bold, italic, underline, strike, code, links), refuses
  `javascript:` and other unsafe links, keeps images as media ids only, and reports every
  problem at once with its place (`body.content[2].attrs.href`). It stores no HTML: the
  website renders the JSON.
- The server computes `word_count`, `reading_time_minutes` and a plain-text copy used by
  search (accents and case ignored: `da lat` finds `Đà Lạt`).
- SEO fields: `meta_title`, `meta_description`, `og_media_id` (sharing image),
  `canonical_url`, `noindex`.

## Write and publish

```bash
# Create a draft
curl -X POST http://localhost:8080/api/v1/content/entries \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -H "Idempotency-Key: $(uuidgen)" \
  -d '{"kind": "post", "locale": "en", "title": "Hello",
       "body": {"type": "doc", "content": [{"type": "paragraph",
                "content": [{"type": "text", "text": "First post."}]}]}}'

# Read it: note the ETag header
curl -i http://localhost:8080/api/v1/content/entries/$ID -H "Authorization: Bearer $TOKEN"

# Change the draft: If-Match is required
curl -X PATCH http://localhost:8080/api/v1/content/entries/$ID \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -H 'If-Match: "1731400000000000"' -d '{"excerpt": "A short summary."}'

# Publish
curl -X POST http://localhost:8080/api/v1/content/entries/$ID/publish \
  -H "Authorization: Bearer $TOKEN" -H 'If-Match: "1731400000000001"'
```

- **`If-Match` is required** to change or delete an entry, to change its state (publish,
  unpublish, archive, move, restore, schedule), to change a term, author or redirect, and to
  replace a menu: send the `ETag` you read (`*` to change it whatever its version). Missing:
  `428 precondition_required`; someone changed it since: `412 precondition_failed` — read it
  again and reapply your change. This keeps two editors (or AI agents) from overwriting each
  other. Deleting a term, author or redirect and setting an entry's terms or authors do not
  need it: nothing is overwritten.
- `POST …/unpublish` takes an entry down (it is a draft again, its address answers 404);
  `POST …/archive` takes it down for good (its address answers 410); `DELETE` deletes it
  (published entries are taken down first, their address answers 410).
- A page cannot be published before its parent page, nor taken down or deleted while it has
  published subpages.

## History and scheduling

- A **revision** is a full copy of the editable fields. One is kept on creation, on each
  publish and take-down, on restore, when asked (`"save_revision": true`), and automatically
  when the last one is more than 10 minutes old. `GET …/revisions` lists them,
  `POST …/revisions/{revision_id}/restore` copies one back into the draft (it does not
  publish). The newest `AUTOXIDE_CONTENT_MAX_REVISIONS` (default 50) are kept per entry, plus
  the published one and those of pending schedules; a daily job prunes the rest.
- **Scheduling**: `POST …/schedules` with `{"action": "publish" | "unpublish", "run_at": …}`.
  A scheduled publish pins the current draft as a revision: later edits are not published by
  it. If the entry was published by hand after the schedule was made, the schedule fails
  (`content.schedule_outdated`) instead of overwriting it. A failed schedule keeps its error
  code (`status: failed`) and sends the event `content.schedule_failed`. Taking an entry down
  cancels its pending scheduled publish, and archiving cancels every pending schedule of the
  entry (each sends `content.schedule_canceled`): a schedule made earlier never brings back an
  entry that was taken down.

## Pages and addresses

The server computes the address (`path`) of every published entry:

- a post: `/<AUTOXIDE_CONTENT_POST_PATH_PREFIX>/<slug>` (default `/blog/hello`);
- a page: the slugs of its parent pages, then its own (`/about/team`); at most 5 levels.

The website only adds its domain and, if it uses one, a language prefix (`/vi/…`).

- `POST …/move` puts a page under another parent (or at the root). It takes effect at once,
  for the draft and the published version, and for every subpage. A page cannot move under
  itself or one of its subpages (`content.tree_cycle`).
- **When a published address changes** (new slug, move), the server records a `301` redirect
  from the old address to the entry — for the entry and every subpage — so old links keep
  working. Archiving or deleting records a `410 Gone`.
- Redirects can also be made by hand (`/api/v1/content/redirects`, permission
  `content.manage`): `301`, `302`, `307`, `308` to an entry or an address, or `410`. Loops
  are refused (`content.redirect_loop`). An address in the site starts with a single `/`:
  `//host` (which browsers read as another site) is refused, here and in menus.

## Languages

Languages are set by `AUTOXIDE_CONTENT_LOCALES` (default `en,vi`, BCP 47 codes) and
`AUTOXIDE_CONTENT_DEFAULT_LOCALE` (default `en`).

With several tenants, these are the default of every tenant: a tenant can set its own
`content.locales` and `content.default_locale` (its administrator with
`PUT /api/v1/kernel/settings/{name}`, permission `kernel.settings.manage`; platform staff with
`PUT /api/v1/kernel/tenants/{id}/settings/{name}`). Its languages then apply to everything of that
tenant: what entries, terms, menus and redirects accept, the `locale` of public routes, and the
`content` part of `GET /api/v1/kernel/client-config` for requests that name the tenant. When the
tenant's default language is not one of its languages, the server's default is used if it is one of
them, otherwise the first of them. Entries already written in a language a tenant drops stay as
they are. Wrong codes are refused when they are set; `DELETE` a setting to go back to the
server's value.

- Each translation is its own entry, with its own slug, status and history, linked to the
  others by `translation_group_id`. Create one with `POST …/entries/{id}/translations`
  (`{"locale": "vi", "title": …}`). A subpage can be translated only once its parent is.
- **Posts and pages never fall back to another language**: a missing translation is `404`, so
  canonical and `hreflang` stay right. Published entries list their translations
  (`translations`, for `hreflang`).
- Names of categories, tags and menus fall back to the default language; the answer says which
  language it used (`locale`).

## Categories, tags, authors, menus

Permission `content.manage`.

- **Terms** (`/api/v1/content/terms`): `category` (can have a parent) or `tag`, with a name,
  slug, description and SEO texts per language. Attach them with
  `PUT /api/v1/content/entries/{id}/terms` (`{"term_ids": [...]}`, the first category is the
  main one). This takes effect at once, also on the published version — so for a published
  entry it needs `content.publish` as well as `content.write` (the same for its authors).
- **Authors** (`/api/v1/content/authors`): profiles of the content module — display name, slug,
  bio, avatar (a media id), website — optionally linked to a user account (`user_id`). An
  entry can have several, in order: `PUT /api/v1/content/entries/{id}/authors`. Setting the
  terms or authors of an entry needs the right to edit it, not `content.read`.
- **Menus** (`/api/v1/content/menus/{key}?locale=`): a tree of items (at most 4 levels, 200
  items), each pointing to an entry, a term, or an address. `PUT` replaces the whole tree
  (`If-Match: *` to create one). **Deprecated** (sunset 14 May 2027): website menus moved to the
  `menus` module, whose items can point to objects of any module; copy yours with
  `autoxide task menus.import_content_menus` ([`MENUS.md`](MENUS.md)).

## Stores and websites

In a tenant with an organization tree (module `organizations`), each entry belongs to a unit:
`org_id` when creating it (omitted: your only unit, or the root when your role covers the whole
tenant). The permissions above follow the scope of the role: someone with `content.write` on a
store (`subtree`) writes only entries of that store and the units below it; entries outside their
scope answer 404. Filter the admin list with `owner_org` (one unit) or `site` (what a website
shows).

Each unit can have a website: its domain bound to the unit (`PATCH /api/v1/kernel/domains/{id}`
with `org_id`), or `?site=<unit id>` on the public endpoints. A website shows the entries of its
unit and of every unit above it — head-office posts appear on every store's website, a store's
posts never on another store's.

Each website has its own addresses and slugs: two stores can both have `/about`. When a unit and
a unit above it both publish the same address, the website shows the one nearest to it (the
store's `/about` hides the head office's on the store's website; the sitemap lists it once). The
languages a website offers are the setting `organizations.languages` of its unit
(`PUT /api/v1/organizations/units/{id}/settings/organizations.languages` with `["vi", "en"]`; the
first is its default), kept to the tenant's languages; unset, the tenant's languages. Public
reads, the sitemap, the feed and public search follow the website's languages and show only what
the website shows.

## The website: public endpoints

No sign-in needed. Only published versions are ever returned — drafts never.

| Endpoint                                              | Gives                                                      |
| ----------------------------------------------------- | ---------------------------------------------------------- |
| `GET /public/resolve?locale=&path=`                   | What lives at an address: `entry`, `redirect` or `gone`    |
| `GET /public/posts?locale=&term_id=&author_id=&q=`    | Published posts, newest first, paged (`q`: at most 200 characters) |
| `GET /public/menus/{key}?locale=`                     | A menu with computed `href`s; deprecated, use `GET /api/v1/menus/public/{key}` |
| `GET /public/terms/{taxonomy}/{slug}?locale=`         | A category or tag page                                     |
| `GET /public/authors/{slug}`                          | An author page                                             |
| `GET /public/sitemap?locale=`                         | Every published address, with `hreflang` alternates, paged |
| `GET /public/feed?locale=&limit=`                     | The newest posts with body, for RSS, Atom or JSON Feed     |

(All under `/api/v1/content`.)

A category or tag is public only once an entry shown on the website carries it: the term page answers
`404 content.term_not_found` before that, and the term list leaves it out. This is on purpose — a term
being prepared is not revealed — so a menu item pointing to an empty category leads to a 404 page until
a post carries the category.

**Several sites on one server.** On a server with several tenants, each tenant is its own site:
the public endpoints answer for the tenant of the request — its own domain, or the
`Autoxide-Tenant` header (its short name). Two tenants may use the same slugs, addresses,
author and term slugs, menus and redirects; one never sees the other's entries. An `ETag` from
one tenant never matches in another.

**Rendering an address.** For each request, the website calls `resolve` with the language
and the path, and acts on `type`:

- `entry` → render `data.entry` (title, body JSON, images with their URLs, SEO, terms,
  authors, translations);
- `redirect` → answer `data.status` with `Location: data.location`;
- `gone` → answer `410`;
- a `404` response → answer `404`.

**Caching.** Public answers carry `Cache-Control: public, no-cache` and, where it makes sense,
an `ETag`: a cache may keep them but must ask again; sending the `ETag` back in
`If-None-Match` gets a cheap `304 Not Modified`.

**Refreshing the cache when content changes.** Subscribe a webhook
(`POST /api/v1/webhooks`, see the webhooks section of the [README](../README.md)) to
`content.entry_published`, `content.entry_unpublished`, `content.entry_archived`,
`content.entry_deleted` and `content.entry_moved`. Their `data.payload.published` gives what
to purge:

```json
{
  "entry_id": "…", "kind": "page", "locale": "en", "translation_group_id": "…",
  "path": "/company/team",
  "stale_paths": ["/about/team", "/about/team/jobs"],
  "moved_subpages": [{"entry_id": "…", "old_path": "/about/team/jobs", "path": "/company/team/jobs"}]
}
```

`path` is the new address (`null` when no longer published), `stale_paths` every address
that changed, subpages included. Menus, terms and authors send `content.menu_changed`,
`content.term_changed` and `content.author_changed`; these carry no list of affected pages
(a tag can be on thousands). Tag each cached page with its term and author ids (for example
`term:<id>`, `author:<id>`) and revalidate those tags when such an event arrives — renaming or
deleting a tag or an author, including when an account is deleted, then refreshes every page
that shows it.

**Sitemap and feed.** Build `sitemap.xml` from `/public/sitemap` (follow `next_cursor`; skip
items with `noindex: true`; `content_modified_at` is `lastmod`; `translations` gives the
`hreflang` alternates). Build RSS, Atom or JSON Feed from `/public/feed`, using each post's
`id` (never changes) as the item id.

### Hidden by a case (451)

When the `compliance` module handles a report of an infringement (counterfeit, intellectual property…)
about an entry and hides it, `content` receives the event `compliance.target_restricted`: the published
entry answers **451** `content.restricted` at `GET /public/resolve` (answer 451 to the reader), and is
left out of `/public/posts`, the sitemap, the feed, search and other modules (resource directory). It
stays hidden when published again. `compliance.target_restored` shows it as before. See
[`COMPLIANCE.md`](COMPLIANCE.md#reports-of-infringements-and-consumer-complaints).

## Banners and display placements

A **placement** is a named spot of the website (`home_hero`, `top_strip`…) that tells the
website **how to draw** what is in it and tells editors **what picture fits**, so changing a
banner never breaks the layout. A **banner** is the content shown there, with a time window and an
order. Changing banners is a data change (REST or MCP), not a change of the website's code.

### Placements (`content.manage`)

`GET/POST /placements`, `GET/PUT/DELETE /placements/{key}` (`PUT` needs `If-Match`; deleting a
placement that still has banners answers 409 `content.placement_in_use`). Each has:

| Field | Meaning |
|---|---|
| `display` | `single`, `carousel`, `grid` (2–4 cells), `stack` (vertical, for side columns), `strip` (a thin text bar), `popup` |
| `display_settings` | `carousel`: `autoplay_seconds` (0 = off, else ≥ 4); `grid`: `columns` (2–4); `popup`: `frequency` (`once_per_session`, `once_per_day`, `once_per_week`), `delay_seconds` (0–60) |
| `accepts` | `image`, `text` or `any`; a `strip` takes only `text` |
| `ratio`, `mobile_ratio`, `min_width`, `min_height` | picture rules, checked when a banner is saved (2% leeway on the ratio), from the size the media library stored |
| `devices` | `all`, `desktop`, `mobile`: call the placement only on these |
| `page_kinds` | kinds of pages it is used on: `home`, `category`, `collection`, `product`, `search`, `cart`, `content_entry`, `content_type`, `landing`, or `any` |
| `max_items` | most banners at once (1–50) per store and language |

New tenants get a **standard set**: `top_strip` (strip, text), `home_hero` (carousel, 3:1, at least
1800×600; phones 4:5), `home_middle` (grid of 3, 1:1), `category_top` (4:1), `sidebar_left` and
`sidebar_right` (stack, 1:2), `inline` (4:1, image or text), `above_footer` (4:1), `popup` (once a day,
after 5 s, 4:5). Edit, switch off or delete them like your own. A tenant created before this
feature adds the missing ones with `POST /placements/presets` (it never brings back one you
deleted unless you call it).

### Banners (`content.banners`)

`GET/POST /banners`, `GET/PATCH/DELETE /banners/{id}` (`PATCH` needs `If-Match`), and
`PUT /placements/{key}/order` with `{"banner_ids": [...]}` and the placement's `ETag` in `If-Match`
to order a whole placement at once. `content.banners` is given per organization unit: the staff of
a store manage only that store's banners; `org_id` left out means every store of the tenant.

- `kind`: `image` (needs `media_id`, a **public** picture; optional `mobile_media_id`) or `text`
  (`title` required, `tone` = `primary`, `secondary`, `accent` or `neutral` — never a colour code, so
  a new theme recolours it).
- `link`: `{"url": "https://…"}` or a path `/…`, or `{"kind": "catalog.product", "id": "…"}` — any type
  of the object directory (`GET /api/v1/kernel/resources`). Saving checks the object exists; the
  public read leaves the banner out once the object is no longer public (product off sale, promotion
  over). A banner linking to a `promotions.promotion` is **archived** when the promotion ends or is
  cancelled: advertising must not promise an offer that is over (Vietnam's Law on Consumer Protection
  19/2023/QH15, Law on Advertising — check the original texts).
- `page_kinds` (within the placement's) and `page_targets` (`[{"kind": "catalog.category", "id": "…"}]`,
  at most 50): show the banner only on those pages. Example: a left-column banner only on the
  "Phones" category.
- `starts_at` / `ends_at`, `status` (`draft`, `active`, `archived`), `ai_assisted`.

### Reading on the website (public)

One call for every placement of a page:

```http
GET /api/v1/content/public/placements?keys=top_strip,home_hero,home_middle,popup&locale=vi&page_kind=home
GET /api/v1/content/public/placements?keys=category_top,sidebar_left&page_kind=category&page_ref=catalog.category:0190…
```

Each placement comes with `display`, `display_settings`, `accepts`, ratios, `devices` and its
`banners`: active, inside their time window now, of the website (`site`, or the domain) and the
units above it, in the page's language, in a **fixed order** (`position`). Images come with
`variants` and `webp_url` for `srcset` / `<picture>`; links come ready as `href`. Banners not limited
to some pages always show; without `page_kind` only those show. `Cache-Control: public, max-age=…`
lasts until the next banner starts or ends (at most 60 s), so a CDN never keeps an expired banner;
`ETag` gives 304. `GET /public/placements/{key}` reads one placement.

**Rules for the website** (part of the contract):

- Draw banners in the order given; never shuffle them (it breaks caching and the page jumps on reload).
- A `popup` always has a visible close button, respects `frequency`, and never shows on cart, checkout,
  sign-in or account pages.
- A `carousel` does not autoplay when the viewer asks for reduced motion
  (`prefers-reduced-motion`), and has controls to pause and move.
- Label banners with `ai_assisted: true` as written with AI help.

Example of a home page:

```js
const res = await fetch(`/api/v1/content/public/placements?keys=top_strip,home_hero,home_middle,popup&page_kind=home&locale=${lang}`);
const { data } = await res.json();
for (const p of data.placements) render(p.key, p.display, p.display_settings, p.banners);
```

### Store information (header and footer)

`GET /api/v1/organizations/public/storefront` (public; the website from `site` or the domain) gives the
**brand** — name, logo and site icon (ids of public media items: `/api/v1/media/{id}/content`), phone, email,
social profiles — inherited down the organization tree (a chain sets it, a store overrides what it
wants), the **location** (address, opening hours, special days, pickup, cover picture of the unit's
public profile), the unit's time zone and `open_now`. Manage the brand with the unit settings
`organizations.brand_name`, `organizations.brand_logo`, `organizations.brand_icon`,
`organizations.contact_phone`, `organizations.contact_email`, `organizations.social_links`
(`PUT /api/v1/organizations/units/{id}/settings/{key}`). For the legal footer of a shop in Vietnam,
also show `GET /api/v1/compliance/public/owner` and the badge of `GET /api/v1/compliance/public/badge`.

### MCP tools

`content.list_banners`, `content.save_banner` (drafts only: a person sets a banner `active`; text
from a tool is marked `ai_assisted` unless told otherwise), `content.order_placement`.

## Draft preview

Editors can show a draft to someone without an account:

1. Set `AUTOXIDE_CONTENT_PREVIEW_SECRET` (a random string of at least 32 characters). Without
   it, previews are off (`503`, code `content.preview_disabled`).
2. `POST /api/v1/content/entries/{id}/preview-token` (permission `content.write`) returns a
   signed `token` and its `expires_at` (`AUTOXIDE_CONTENT_PREVIEW_TTL_SECONDS`, default 10
   minutes).
3. The website's preview page calls `GET /api/v1/content/public/preview?token=…` and renders
   the draft. The answer is `Cache-Control: no-store`; it always shows the latest draft, and
   holds only its content — not who created it or asked for a review. An invalid or expired
   token is `404` (`content.preview_token_invalid`). Changing the secret revokes every token.

On a server with several tenants, a token only opens the draft of the tenant it was made in: it
names its tenant, so the preview page works on the main domain without a tenant header, and the
same token sent to another tenant (by domain or header) is `404` as an invalid token.

## AI agents

**AI helpers** (guide: [`AI.md`](AI.md)): `GET /entries/{id}/seo-check` (no AI),
`POST /entries/{id}/seo-suggestions`, `POST /entries/{id}/translate` (into the draft of the
translation, never published; follow `GET /translation-requests/{id}`), and `GET /search` /
`GET /public/search` (by meaning and words). Entries made by the server's translation carry
`translated_from_revision_id`, and `translation_outdated` once the source changes.

**MCP tools.** `content.list`, `content.get`, `content.create_draft`, `content.update`,
`content.publish`, `content.unpublish`, `content.list_revisions`, `content.restore_revision`,
`content.request_review`, `content.seo_check`, `content.suggest_seo`, `content.translate`,
`content.get_translation_request`, `content.search` and `content.resolve`, with the same
permissions as the REST endpoints. Results that contain an entry include its `version`; the tools that change an entry
require it back in their `version` argument, exactly like `If-Match`.

**Give an agent its own API key** (`POST /api/v1/auth/api-keys`) with a short `expires_at` and
only the permissions it needs:

| The agent may…                         | Permissions                                            |
| -------------------------------------- | ------------------------------------------------------ |
| write drafts, a person publishes       | `content.read`, `content.write_own`                    |
| write and publish its own posts        | `content.read`, `content.write_own`, `content.publish_own` |

With the `_own` permissions an agent only touches the entries it created with that same key:
another key — even of the same user — gets `403 content.not_owner`. It can never archive,
schedule or delete (deleting needs `content.delete`), nor move a published page; it can move
its own unpublished pages, under pages it created. More limits keep it from going around an
editor:

- `content.write_own` without `content.write`: a new page, a moved page or a translated subpage
  goes only under a parent page the agent created (`403 content.not_owner`), so it cannot block
  someone else's page from being taken down or deleted; the entry is always marked
  `ai_generated: true`, whatever is sent; on `PUT …/authors` it may add only author profiles
  linked to its own account (`user_id`) — profiles already on the entry may stay.
- `content.publish_own` without `content.publish`: it cannot publish an entry an editor
  archived (`403 content.archived`); it cannot set `published_at` (422, `not_allowed`; the
  server uses the current time, or keeps the date of an earlier publish); it cannot publish at
  an address where an editor made a redirect (`409 content.path_has_redirect`), since the
  published entry would hide it.

Every change is in the audit log with the API key that made it, and requests with an API key
are rate limited per key (`AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE`, default 120).

**Review.** When the agent is done, it calls `POST /entries/{id}/request-review` (tool
`content.request_review`): the draft gets `review_requested_at` and the event
`content.review_requested` goes out — subscribe a webhook to tell your editors.
`DELETE /entries/{id}/request-review` withdraws it; publishing clears it.

**AI-generated content.** Send `"ai_generated": true` when creating or editing content written
by AI (agents should always do so; with only `content.write_own` the server sets it anyway).
When a person — signed in, not with an API key — publishes
such an entry after reviewing it, they send `{"ai_reviewed": true}` to
`POST /entries/{id}/publish`; the published version records who and when. Public endpoints
return `ai_generated` and `ai_reviewed`, so the website can show readers a notice where the
rules ask for one (for example Article 50 of the EU AI Act, which applies from 2 August 2026;
its exception for human-reviewed text is read narrowly). Autoxide records the facts; it does
not give legal advice. The admin API also tells how an entry was created: `created_via` is
`session` or `api_key`.

**AI-assisted content.** When a person writes or edits content from AI suggestions (applies
`seo-suggestions`, rewrites an AI draft), send `"ai_assisted": true` on `POST /entries` or
`PATCH /entries/{id}` (MCP: `create_draft`, `update_draft`). The flag is copied to the published
version on publish; the admin, preview and public endpoints return `ai_assisted`, so the website
labels the content (Vietnam's Law on Artificial Intelligence 134/2025/QH15, from 1 March 2026 —
the form of the label and what counts as "edited" still need checking against the decree).
Anyone who may write sets it; removing it (`false` while it is `true`) needs `content.publish`
(else 403 `content.ai_label_requires_publish`), and restoring an older revision never removes
it. `ai_generated` (the whole text written by AI) is unchanged. Images: the media library's
`alt_ai_assisted` ([`MEDIA.md`](MEDIA.md)) comes with each public image as `alt_ai_assisted`.

## When an account is deleted

Posts and pages belong to the site, not to the person who wrote them: they stay. Author
profiles linked to the account are deleted (and removed from entries), schedules the person
made are canceled, and `created_by` becomes `null`, as do `review_requested_by` (who asked
for a review) and the record of who confirmed an AI review — the time of that confirmation
stays, so readers still see `ai_reviewed`. The personal data export lists the person's author
profiles and the entries they created.

## Settings

| Setting                                | Default                                          |
| -------------------------------------- | ------------------------------------------------ |
| `AUTOXIDE_CONTENT_LOCALES`             | `en,vi`                                          |
| `AUTOXIDE_CONTENT_DEFAULT_LOCALE`      | `en`                                             |
| `AUTOXIDE_CONTENT_POST_PATH_PREFIX`    | `blog`                                           |
| `AUTOXIDE_CONTENT_MAX_REVISIONS`       | `50`                                             |
| `AUTOXIDE_CONTENT_MAX_BODY_BYTES`      | `524288`                                         |
| `AUTOXIDE_CONTENT_WORDS_PER_MINUTE`    | `238`                                            |
| `AUTOXIDE_CONTENT_EMBED_HOSTS`         | `youtube.com,youtu.be,youtube-nocookie.com,vimeo.com` |
| `AUTOXIDE_CONTENT_PREVIEW_SECRET`      | none (previews off)                              |
| `AUTOXIDE_CONTENT_PREVIEW_TTL_SECONDS` | `600`                                            |

All are described in [`.env.example`](../.env.example).
