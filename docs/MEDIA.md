# Media library

*Tiếng Việt: [MEDIA.vi.md](MEDIA.vi.md)*

The `media` module keeps the images, videos, audio files and documents of a website or
app in nested folders, and serves them, publicly or to signed-in users. It lives under
`/api/v1/media`. The full contract, with every parameter and error code, is in
[`openapi.json`](openapi.json) (and at `/api/docs` when `AUTOXIDE_HTTP_API_DOCS=true`).

Content is stored where the `files` module stores files (`AUTOXIDE_FILES_DIR` on disk, or
an S3 bucket), under keys starting with `media/`. Details (name, type, size, image size,
texts, folder) are in the database. `autoxide backup` and `autoxide restore` carry both.

## Permissions

| Permission    | Allows                                                                    |
| ------------- | ------------------------------------------------------------------------- |
| `media.read`  | Listing and reading items (private ones too), folders and the trash      |
| `media.write` | Uploading, editing, replacing, moving, publishing, trashing; folders     |
| `media.purge` | Deleting items in the trash for good                                      |

Give them to a role with `PUT /api/v1/auth/roles/{id}`. The `admin` role has them all.

## Upload

Send the raw file as the body (not a multipart form), its type as `Content-Type` and its
name as `name`:

```bash
curl -X POST "http://localhost:8080/api/v1/media?name=team.jpg" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: image/jpeg" \
  -H "Idempotency-Key: $(uuidgen)" \
  --data-binary @team.jpg
```

- The server checks the content really is of the declared type, from its first bytes. An
  HTML page sent as `image/png` is refused (`415`, code `media.content_mismatch`). Without
  `Content-Type` (or with `application/octet-stream`) the server detects the type.
- Accepted types are set by `AUTOXIDE_MEDIA_ALLOWED_TYPES` (default: JPEG, PNG, GIF, WebP,
  AVIF, SVG, MP4, WebM, every audio type, PDF); the largest size by
  `AUTOXIDE_MEDIA_MAX_BYTES` (default 100 MiB).
- Raster images (JPEG, PNG, GIF, WebP, AVIF) get `width` and `height`. The server keeps
  the original only: it makes no thumbnails or other sizes. Resize on the website (for
  example with an image component of your framework) or in front of the server.
- The answer (`201`, with `Location`) lists `duplicates`: items already in the library
  with exactly the same content (same SHA-256). The upload is kept either way.
- `folder_id=<id>` puts the item in a folder. New items are `private`.

## Read, list, search

- `GET /api/v1/media/{id}` — one item, with an `ETag`.
- `GET /api/v1/media` — a page of items. Filters: `folder_id` (`root` or an id; only that
  folder, not its subfolders), `type` (`image/png` or `image/*`), `visibility` (`public` /
  `private`), `trashed=true` (the trash), `q` (name contains, ignoring case). Order:
  `sort=created_at|name|size` with `order=asc|desc`. Follow `pagination.next_cursor` with
  the same filters.

## Change

Changes need the `ETag` you read, in `If-Match` (`*` to change whatever the version).
Without it the answer is `428` (code `precondition_required`); if someone changed the item
in between, `412` (code `precondition_failed`) — read it again and retry.

- `PATCH /api/v1/media/{id}` — `name`, `alt`, `title`, `caption`, `folder_id` (`null`
  for the root), `visibility`.
- `PUT /api/v1/media/{id}/content` — replace the content, keeping the id (and every place
  that uses the item). Same checks as an upload. The `url`, `version_id` and `ETag` change;
  the old content is deleted.

## Serve the content

Every item has a `url`: `/api/v1/media/{id}/content?v=<version_id>`.

- **Who**: items with `visibility: public` (and not in the trash) load without signing in
  — use the `url` directly in `<img src>` or `<video src>`. Other items need `media.read`;
  without it the answer is `404`, as if the item did not exist.
- **Caching**: with the current `v` the answer may be cached for a year (`immutable`):
  replacing the content changes `v`, hence the address. Without `v`, or with an old one,
  it is cached for a minute (public) or revalidated each time (private). Making an item
  private does not remove copies already in caches (browsers, CDN); replace its content if
  that matters.
- **Safety**: only raster images open in the browser (`Content-Disposition: inline`);
  everything else, SVG and PDF included, is downloaded (`attachment`). Every answer carries
  `X-Content-Type-Options: nosniff` and `Content-Security-Policy: default-src 'none';
  sandbox`, so content cannot run scripts on your domain.
- **Video and large files**: one `Range: bytes=…` is answered with `206` (seeking in a
  video works); a range outside the content with `416`. `HEAD` gives the headers only.
  `If-None-Match` with the `ETag` gives `304`.

## Folders

`/api/v1/media/folders`: create (`POST`, `name` and `parent_id`), list (`GET`, filter
`parent_id=root|<id>`, `sort=created_at|name`), read, rename or move (`PATCH`, with
`If-Match`), delete (`DELETE`). Folders nest. Names are unique in a parent, ignoring case.
A folder cannot move into itself or its subfolders (`409`, `media.folder_cycle`). Only
empty folders can be deleted — items in the trash count (`409`, `media.folder_not_empty`).

## Trash

- `DELETE /api/v1/media/{id}` moves an item to the trash: its public link stops working,
  `media.read` still sees it (`GET /api/v1/media?trashed=true`).
- `POST /api/v1/media/{id}/restore` takes it out.
- `POST /api/v1/media/{id}/purge` (permission `media.purge`) deletes it for good.
- Items stay in the trash `AUTOXIDE_MEDIA_TRASH_DAYS` days (default 30), then a daily job
  deletes them. Another daily job deletes stored content no item points to any more
  (left by an interrupted upload, say), only when it is older than a day.

## AI agents (MCP)

Tools `media.list`, `media.get`, `media.update` and `media.trash`, with the same
permissions as the REST endpoints. `media.update` takes the `updated_at` you read instead
of `If-Match`. Uploading goes through REST: binary content does not fit well in a tool
call.

## For module authors

Other modules read media through the `autoxide_media::api::Media` trait (register after
`media`): `find_many(&ids)` returns, in one query, the id, visibility, type, image size,
`alt` and `url` of each item not in the trash. Keep only the item id in your own tables.

## When an account is deleted

Items belong to the library, not to the person who uploaded them: they stay, and their
`uploaded_by` becomes `null`. The personal data export lists the items the person uploaded.
