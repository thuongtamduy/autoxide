# Tải file lớn thẳng lên nơi lưu

English: [`direct-uploads.md`](direct-uploads.md). Ví dụ dùng `curl` với `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`), và SDK TypeScript (`sdk/typescript`). Chúng chạy như test ở mỗi bản dựng, nên khớp với máy chủ của phiên bản này.

Khi file được lưu trên S3 (hay dịch vụ tương thích S3), file lớn không cần đi qua máy chủ API: bạn xin một **link tải lên đã ký**, gửi file thẳng tới nơi lưu, rồi nhờ máy chủ **xác nhận** lượt tải. Máy chủ đọc lại file và kiểm trước khi tạo bản ghi. Cách này dùng được cho file thường (`/api/v1/files/uploads`) và cho thư viện media (`/api/v1/media/uploads`, kiểm thêm loại thật của nội dung, như khi tải qua máy chủ).

Khi file được lưu trên ổ đĩa của máy chủ thì không có link: máy chủ trả `409` (`files.direct_upload_unavailable`) và bạn tải qua `POST /api/v1/files` (hay `POST /api/v1/media`) như thường.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
```

## 1. Xin link

Nêu tên file, loại nội dung, cỡ chính xác theo byte và SHA-256 (64 chữ số hex, vd `sha256sum file`). Mọi chỗ sai được báo cùng lúc (`422`). Máy chủ ví dụ này lưu file trên ổ đĩa nên trả `409`; trên S3 nó trả `201` kèm link.

<!-- example: start-upload -->
```bash
curl -sS "$API/api/v1/files/uploads" -H "Authorization: Bearer $TOKEN" \
  --json '{"name":"hello.txt","content_type":"text/plain","size":5,"sha256":"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"}'
# expect: 409
```

Trên S3, câu trả lời trông như sau (`Location` là file mà lượt tải sẽ thành):

```json
{"data": {"id": "0193f2a0-…", "method": "PUT", "url": "https://my-bucket.s3.amazonaws.com/files/0193f2a0-…?X-Amz-Signature=…",
  "headers": {"content-length": "5", "content-type": "text/plain", "x-amz-checksum-sha256": "LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ="},
  "expires_at": "2026-11-18T10:15:00Z"}}
```

## 2. Gửi file tới link

Gửi nội dung với đúng method và các header đã cho, trước `expires_at` (mặc định 15 phút, `AUTOXIDE_FILES_DIRECT_UPLOAD_SECONDS`). Chữ ký bao gồm loại và checksum: Amazon S3 từ chối file khác. Request đi tới nơi lưu, không tới máy chủ API.

```bash
curl -sS -X PUT "$URL" -H 'content-type: text/plain' \
  -H 'x-amz-checksum-sha256: LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=' --data-binary @hello.txt
```

## 3. Xác nhận

```bash
curl -sS -X POST "$API/api/v1/files/uploads/$ID/complete" -H "Authorization: Bearer $TOKEN"
```

- `201` kèm file mới (cùng hình dạng với `POST /api/v1/files`); gọi lại trả `200` với cùng file, nên thử lại an toàn.
- `409 files.upload_incomplete`: nơi lưu chưa có file — gửi file rồi gọi lại.
- `422 files.upload_mismatch`: cỡ hay SHA-256 của file khác khai báo. File đã bị xoá; gửi đúng file tới cùng link rồi xác nhận lại trước khi link hết hạn.
- `409 files.upload_expired`: bắt đầu lượt mới.

Lượt không bao giờ được xác nhận bị một job xoá một giờ sau khi hết hạn. File thường giữ chỗ cỡ đã khai trong hạn mức dung lượng của khách từ bước 1 tới khi xác nhận hay bị xoá.

## Thư viện media

`POST /api/v1/media/uploads` nhận cùng các trường, thêm `folder_id`, `tags` và `org_id`; `POST /api/v1/media/uploads/{id}/complete` tạo item. Xác nhận đọc lại cả item và kiểm như khi tải qua máy chủ: nội dung phải đúng là loại đã khai (`415 media.content_mismatch`), SVG không được có nội dung chạy được, ảnh được đọc chiều rộng và cao. Item tới `AUTOXIDE_MEDIA_DIRECT_MAX_BYTES` (mặc định 1 GiB) tải được theo cách này; với item lớn hãy tăng `AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS` để việc kiểm vừa một request.

## Cho AI agent (MCP)

Các tool `files.create_upload` / `files.confirm_upload` và `media.create_upload` / `media.confirm_upload` làm cùng việc; agent tự gửi file tới link.

## Cài đặt bucket

Trình duyệt chỉ gửi được file khi bucket cho phép (CORS): cho phép `PUT` từ origin của website với các header `content-type` và `x-amz-checksum-sha256` — xem mục "Tải thẳng lên S3" trong [`docs/DEPLOY.vi.md`](../DEPLOY.vi.md).

## Với SDK

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
