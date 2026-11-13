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
- Raster images (JPEG, PNG, GIF, WebP, AVIF) get `width` and `height`. JPEG, PNG, GIF and
  WebP images also get smaller copies (see below).
- The answer (`201`, with `Location`) lists `duplicates`: items already in the library
  with exactly the same content (same SHA-256). The upload is kept either way.
- `folder_id=<id>` puts the item in a folder, `tags=brand,logo` gives it tags. New items
  are `private`.

## Read, list, search

- `GET /api/v1/media/{id}` — one item, with an `ETag`.
- `GET /api/v1/media` — a page of items. Filters: `folder_id` (`root` or an id; only that
  folder, not its subfolders), `type` (`image/png` or `image/*`), `visibility` (`public` /
  `private`), `trashed=true` (the trash), `q` (name contains, ignoring case), `tag` (has
  this tag, ignoring case). Order:
  `sort=created_at|name|size` with `order=asc|desc`. Follow `pagination.next_cursor` with
  the same filters.

## Change

Changes need the `ETag` you read, in `If-Match` (`*` to change whatever the version).
Without it the answer is `428` (code `precondition_required`); if someone changed the item
in between, `412` (code `precondition_failed`) — read it again and retry.

- `PATCH /api/v1/media/{id}` — `name`, `alt`, `title`, `caption`, `folder_id` (`null`
  for the root), `visibility`, `tags` (replaces them all; `[]` removes them).
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
  sandbox`, so content cannot run scripts on your domain. On top of that, an SVG containing
  anything that can run (`<script>`, `on…=` attributes, `javascript:` links, embedded HTML),
  anywhere in the file, is refused at upload (`415 media.svg_active_content`).
- **Metadata**: the original is served exactly as uploaded, metadata included — a photo's
  EXIF may hold where it was taken and the camera's serial number. Remove it before
  uploading when that matters; the smaller copies below carry none.
- **Video and large files**: one `Range: bytes=…` is answered with `206` (seeking in a
  video works); a range outside the content with `416`. `HEAD` gives the headers only.
  `If-None-Match` with the `ETag` gives `304`.

## Smaller copies of images

A few seconds after an upload (or a content change), a background job makes smaller copies
of every JPEG, PNG, GIF and WebP image, at the widths in `AUTOXIDE_MEDIA_IMAGE_WIDTHS`
(default `320,640,1280,1920`; only widths smaller than the image; `off` to turn this off).
They are turned the right way up (EXIF orientation), keep the proportions, and are JPEG — or
PNG when the image has transparency. The item lists them in `variants`, each with its `url`
(the item's `url` plus `&w=<width>`), ready for `srcset`:

```html
<img src="/api/v1/media/…/content?v=…&w=640"
     srcset="…&w=320 320w, …&w=640 640w, …&w=1280 1280w" sizes="100vw" alt="…">
```

Until a copy is made, asking for it returns the original, briefly cached. Images over 40
million pixels, AVIF and SVG get no copies. Replacing the content makes new copies and
deletes the old ones. Copies follow the item's visibility and trash.

Changing `AUTOXIDE_MEDIA_IMAGE_WIDTHS` applies to new uploads. To give images uploaded
before the same widths, run this once after restarting with the new setting; the worker
makes the copies in the background (`dry_run=true` only counts):

```bash
autoxide task media.refresh_variants
```

### WebP copies

Next to each JPEG or PNG copy the job also makes a WebP copy of the same size, usually far
lighter: on a test photo about 55–60% smaller than the JPEG, and for an image with
transparency over 90% smaller than the PNG (3.3 MB down to 80 KB at 1920 px). Each entry of
`variants` gives its address in `webp_url` (the copy's `url` plus `&format=webp`) and its
size in `webp_size`. Let the browser pick: browsers that read WebP take the `<source>`, the
others the `<img>`:

```html
<picture>
  <source type="image/webp"
          srcset="…&w=320&format=webp 320w, …&w=640&format=webp 640w" sizes="100vw">
  <img src="…&w=640" srcset="…&w=320 320w, …&w=640 640w" sizes="100vw" alt="…">
</picture>
```

The public views of the content module (`featured_media`, `body_media`) carry the same
`webp_url`. `webp_url` is `null` while the WebP copy is not made; an address with
`&format=webp` asked for before then returns the JPEG or PNG copy, briefly cached. Images
whose copies were made before WebP copies existed get them from the same command:
`autoxide task media.refresh_variants` (it also queues images that only lack WebP). Making
the WebP copies takes the background job about three times as long as the JPEG ones (about
a quarter of a second more per photo for the default widths); requests are not slowed.

## Tags

Besides its one folder, an item may carry up to 20 tags (each 1 to 50 characters, without
`,`). Tags are created when first given to an item. `/api/v1/media/tags` lists them, with
the number of items (not in the trash) carrying each, renames one (`PATCH`, with `If-Match`)
or deletes one (`DELETE`, removing it from every item). Names are unique, ignoring case.

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

## Alt text suggestions

`POST /api/v1/media/{id}/alt-suggestions` with `{"locale": "vi", "context": "…"}` (permission
`media.write`) asks the server's AI model for the alt text of a JPEG, PNG, GIF or WebP image in
that language. It sends a small copy (at most 1280 px), re-encoded without metadata even when
the original is small, never the original file; private images only
when `AUTOXIDE_MEDIA_AI_SEND_PRIVATE=true`. Nothing is saved: store what a person accepts with
`PATCH`. `decorative: true` means the image adds no information — leave its alt text empty.
`media.alt` has one language; the alt text per language lives in the image nodes of a post
body. Turning AI on: [`AI.md`](AI.md).

## AI agents (MCP)

Tools `media.list`, `media.get`, `media.update`, `media.trash`, `media.list_tags`,
`media.upload` and `media.suggest_alt`, with the same permissions as the REST endpoints. `media.update` takes the
`updated_at` you read instead of `If-Match`. `media.upload` takes the content as base64, up
to 1 MiB once decoded — but the whole tool call must also fit
`AUTOXIDE_HTTP_BODY_LIMIT_BYTES` (1 MiB by default), and base64 is a third larger, so with
the default limit files of up to about 750 KiB fit; upload larger files through REST.

`media.write` lets its holder see every item it changes — for example by making a private
item public — so give it only to people who may also read the library.

## For module authors

Other modules read media through the `autoxide_media::api::Media` trait (register after
`media`): `find_many(&ids)` returns, in two queries for any number of ids, the id, visibility, type,
image size, `alt`, `url` and smaller copies of each item not in the trash. Keep only the item id in your own tables.

## When an account is deleted

Items belong to the library, not to the person who uploaded them: they stay, and their
`uploaded_by` becomes `null`. The personal data export lists the items the person uploaded.
