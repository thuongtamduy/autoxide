# Showing posts and pictures on a website

Vietnamese: [`website.vi.md`](website.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`), and the TypeScript SDK (`sdk/typescript`). They run as tests against every build, so they match the server of this version.

The website reads **public** endpoints, which need no sign-in and only ever return published versions. The admin steps below (upload, write, publish) set up something to read.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

## Upload a picture and make it public

<!-- example: upload -->
```bash
curl -sS "$API/api/v1/media?name=photo.png" -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: image/png' --data-binary @photo.png
# expect: 201
# save: MEDIA=.data.id
```

<!-- example: read-picture -->
```bash
curl -sS "$API/api/v1/media/$MEDIA" -H "Authorization: Bearer $TOKEN"
# save: MEDIA_ETAG=header:etag
```

Changes take the `ETag` you read in `If-Match`, so two people never overwrite each other (`412` if someone changed it meanwhile).

<!-- example: make-public -->
```bash
curl -sS -X PATCH "$API/api/v1/media/$MEDIA" -H "Authorization: Bearer $TOKEN" -H "If-Match: $MEDIA_ETAG" \
  --json '{"visibility":"public"}' 
```

## Write and publish a post

<!-- example: create-post -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" \
  --json '{"kind":"post","title":"Hello world","featured_media_id":"'"$MEDIA"'"}'
# expect: 201
# save: POST=.data.id
```

<!-- example: read-post -->
```bash
curl -sS "$API/api/v1/content/entries/$POST" -H "Authorization: Bearer $TOKEN"
# save: POST_ETAG=header:etag
```

<!-- example: publish -->
```bash
curl -sS "$API/api/v1/content/entries/$POST/publish" -H "Authorization: Bearer $TOKEN" -H "If-Match: $POST_ETAG" \
  --json '{}' 
```

## Read it as the website does

The newest posts, one page at a time:

<!-- example: public-posts -->
```bash
curl -sS "$API/api/v1/content/public/posts?locale=en"
```

For an address the visitor opened, `resolve` says what to show: `entry` (render it), `redirect` (send the visitor on) or `gone` (answer 410). A missing translation is 404.

`featured_media` and `body_media` give each picture its `url`, `alt` (in the post's language), size, and `variants`: smaller copies, each with a WebP twin in `sources`. Use them for `<picture>` and `srcset`:

```html
<picture>
  <source type="image/webp" srcset="…/content?v=…&w=480&format=webp 480w, …">
  <img src="…/content?v=…" srcset="…&w=480 480w, …" sizes="(max-width: 600px) 100vw, 600px" alt="…" width="…" height="…">
</picture>
```

Smaller copies are made in the background a moment after the upload; until then `variants` is empty and the original is served.

<!-- example: resolve -->
```bash
curl -sS "$API/api/v1/content/public/resolve?locale=en&path=/blog/hello-world"
# save: PAGE_ETAG=header:etag
```

## Cache: ETag and 304

Every public answer has an `ETag`. Send it back in `If-None-Match`: when nothing changed the answer is `304` without a body, and you keep using your copy. Picture URLs carry their version (`?v=…`) and are cached for a year (`Cache-Control: public, max-age=31536000, immutable`): a changed picture gets a new URL.

<!-- example: not-modified -->
```bash
curl -sS "$API/api/v1/content/public/resolve?locale=en&path=/blog/hello-world" -H "If-None-Match: $PAGE_ETAG"
# expect: 304
```

## With the SDK

```ts
import { createClient } from "@autoxide/sdk";

// Public endpoints need no token.
const api = createClient({ baseUrl: "https://api.example.com" });

const page = await api.get("/api/v1/content/public/posts", { query: { locale: "en", limit: 10 } });
for (const post of page.data) {
  console.log(post.title, post.path, post.featured_media?.url);
}

let etag: string | undefined;
const answer = await api.request("get", "/api/v1/content/public/resolve", {
  query: { locale: "en", path: "/blog/hello-world" },
  ifNoneMatch: etag,
});
if ("notModified" in answer) {
  // Keep showing the copy you have.
} else if (answer.type === "entry") {
  console.log(answer.entry.title);
}
```
