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

| Permission        | Allows                                                                           |
| ----------------- | -------------------------------------------------------------------------------- |
| `content.read`    | Reading drafts, revisions and schedules                                          |
| `content.write`   | Creating and changing drafts, translations, restoring revisions, preview links    |
| `content.publish` | Publishing, taking down, archiving, deleting, moving pages, scheduling; the terms and authors of a published entry |
| `content.manage`  | Categories, tags, authors, menus, redirects                                      |

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
  code (`status: failed`) and sends the event `content.schedule_failed`.

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
  are refused (`content.redirect_loop`).

## Languages

Languages are set by `AUTOXIDE_CONTENT_LOCALES` (default `en,vi`, BCP 47 codes) and
`AUTOXIDE_CONTENT_DEFAULT_LOCALE` (default `en`).

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
  entry can have several, in order: `PUT /api/v1/content/entries/{id}/authors`.
- **Menus** (`/api/v1/content/menus/{key}?locale=`): a tree of items (at most 4 levels, 200
  items), each pointing to an entry, a term, or an address. `PUT` replaces the whole tree
  (`If-Match: *` to create one).

## The website: public endpoints

No sign-in needed. Only published versions are ever returned — drafts never.

| Endpoint                                              | Gives                                                      |
| ----------------------------------------------------- | ---------------------------------------------------------- |
| `GET /public/resolve?locale=&path=`                   | What lives at an address: `entry`, `redirect` or `gone`    |
| `GET /public/posts?locale=&term_id=&author_id=&q=`    | Published posts, newest first, paged                       |
| `GET /public/menus/{key}?locale=`                     | A menu with computed `href`s; unpublished targets hidden   |
| `GET /public/terms/{taxonomy}/{slug}?locale=`         | A category or tag page                                     |
| `GET /public/authors/{slug}`                          | An author page                                             |
| `GET /public/sitemap?locale=`                         | Every published address, with `hreflang` alternates, paged |
| `GET /public/feed?locale=&limit=`                     | The newest posts with body, for RSS, Atom or JSON Feed     |

(All under `/api/v1/content`.)

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

## Draft preview

Editors can show a draft to someone without an account:

1. Set `AUTOXIDE_CONTENT_PREVIEW_SECRET` (a random string of at least 32 characters). Without
   it, previews are off (`503`, code `content.preview_disabled`).
2. `POST /api/v1/content/entries/{id}/preview-token` (permission `content.write`) returns a
   signed `token` and its `expires_at` (`AUTOXIDE_CONTENT_PREVIEW_TTL_SECONDS`, default 10
   minutes).
3. The website's preview page calls `GET /api/v1/content/public/preview?token=…` and renders
   the draft. The answer is `Cache-Control: no-store`; it always shows the latest draft. An
   invalid or expired token is `404` (`content.preview_token_invalid`). Changing the secret
   revokes every token.

## AI agents (MCP)

Tools `content.list`, `content.get`, `content.create_draft`, `content.update`,
`content.publish`, `content.unpublish`, `content.list_revisions`, `content.restore_revision`
and `content.resolve`, with the same permissions as the REST endpoints. Results that contain
an entry include its `version`; the tools that change an entry require it back in their
`version` argument, exactly like `If-Match`.

## When an account is deleted

Posts and pages belong to the site, not to the person who wrote them: they stay. Author
profiles linked to the account are deleted (and removed from entries), schedules the person
made are canceled, and `created_by` becomes `null`. The personal data export lists the
person's author profiles and the entries they created.

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
