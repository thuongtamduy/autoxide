# Uploading large files straight to storage

Vietnamese: [`direct-uploads.vi.md`](direct-uploads.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`), and the TypeScript SDK (`sdk/typescript`). They run as tests against every build, so they match the server of this version.

When files are stored on S3 (or an S3-compatible service), a large file does not have to pass through the API server: you ask for a **signed upload link**, send the file straight to the storage, then ask the server to **complete** the upload. The server reads the file back and checks it before creating the record. It works for plain files (`/api/v1/files/uploads`) and for the media library (`/api/v1/media/uploads`, which also checks the real type of the content, as an upload through the server does).

When files are stored on the server's disk there is no link: the server answers `409` (`files.direct_upload_unavailable`) and you upload through `POST /api/v1/files` (or `POST /api/v1/media`) as usual.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
```

## 1. Ask for a link

Give the file's name, media type, exact size in bytes and SHA-256 (64 hex digits, e.g. `sha256sum file`). Every problem with them is reported at once (`422`). This example server stores files on disk, so it answers `409`; on S3 it answers `201` with the link.

<!-- example: start-upload -->
```bash
curl -sS "$API/api/v1/files/uploads" -H "Authorization: Bearer $TOKEN" \
  --json '{"name":"hello.txt","content_type":"text/plain","size":5,"sha256":"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"}'
# expect: 409
```

On S3 the answer looks like this (`Location` is the file the upload will become):

```json
{"data": {"id": "0193f2a0-…", "method": "PUT", "url": "https://my-bucket.s3.amazonaws.com/files/0193f2a0-…?X-Amz-Signature=…",
  "headers": {"content-length": "5", "content-type": "text/plain", "x-amz-checksum-sha256": "LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ="},
  "expires_at": "2026-11-18T10:15:00Z"}}
```

## 2. Send the file to the link

Send the content with exactly the given method and headers before `expires_at` (15 minutes by default, `AUTOXIDE_FILES_DIRECT_UPLOAD_SECONDS`). The signature covers the type and the checksum: Amazon S3 refuses another file. The request goes to the storage, not to the API server.

```bash
curl -sS -X PUT "$URL" -H 'content-type: text/plain' \
  -H 'x-amz-checksum-sha256: LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=' --data-binary @hello.txt
```

## 3. Complete the upload

```bash
curl -sS -X POST "$API/api/v1/files/uploads/$ID/complete" -H "Authorization: Bearer $TOKEN"
```

- `201` with the new file (the same shape as `POST /api/v1/files`); calling it again answers `200` with the same file, so retrying is safe.
- `409 files.upload_incomplete`: the storage does not have the file yet — send it, then call again.
- `422 files.upload_mismatch`: the file's size or SHA-256 differs from what you declared. It was removed; send the right file to the same link and complete again before the link expires.
- `409 files.upload_expired`: start a new upload.

Uploads that are never completed are removed by a job an hour after they expire. Plain files reserve their declared size in the tenant's storage quota from step 1 until they are completed or removed.

## Media library

`POST /api/v1/media/uploads` takes the same fields plus `folder_id`, `tags` and `org_id`, and `POST /api/v1/media/uploads/{id}/complete` creates the item. Completing reads the whole item back and checks it like an upload through the server: the content must really be of the declared type (`415 media.content_mismatch`), an SVG must not contain active content, images get their width and height. Media items up to `AUTOXIDE_MEDIA_DIRECT_MAX_BYTES` (1 GiB by default) can be uploaded this way; for large items raise `AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS` so the check fits in one request.

## For AI agents (MCP)

The tools `files.create_upload` / `files.confirm_upload` and `media.create_upload` / `media.confirm_upload` do the same; the agent sends the file to the link itself.

## Set up the bucket

A browser can only send the file if the bucket allows it (CORS): allow `PUT` from your website's origin with the headers `content-type` and `x-amz-checksum-sha256` — see "Uploads straight to S3" in [`docs/DEPLOY.md`](../DEPLOY.md).

## With the SDK

```ts
import { createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk1_…" /* read it from your secret store */ });
declare const file: File; // e.g. from <input type="file">

const bytes = new Uint8Array(await file.arrayBuffer());
const digest = await crypto.subtle.digest("SHA-256", bytes);
const sha256 = [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");

const link = await api.post("/api/v1/files/uploads", {
  body: { name: file.name, content_type: file.type, size: file.size, sha256 },
});
const headers = { ...link.headers };
delete headers["content-length"]; // the browser sets it
await fetch(link.url, { method: link.method, headers, body: bytes });
const saved = await api.post("/api/v1/files/uploads/{id}/complete", { path: { id: link.id } });
```
