# Đọc lỗi

Tiếng Anh: [`errors.md`](errors.md). Ví dụ dùng `curl` với biến `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`), và SDK TypeScript (`sdk/typescript`). Chúng chạy như test với mỗi bản build, nên khớp máy chủ của phiên bản này.

Mọi lỗi theo RFC 9457 Problem Details (`application/problem+json`):

- `code` — ổn định, cho chương trình: **rẽ nhánh theo nó**, không theo câu chữ. Mã của module bắt đầu bằng tên module (`content.not_found`).
- `detail` — câu cho người đọc, theo ngôn ngữ của `Accept-Language` (mặc định `en`, hoặc `vi`).
- `errors` — với `422`, mỗi trường sai một mục: `field` (vd `items[0].url`), `code`, `detail`. Mọi lỗi của request được báo một lần.
- header `X-Request-Id` — ghi kèm khi báo lỗi.
- `Retry-After` (header) — với `429` (quá nhiều request) và `503` (máy chủ đang khởi động lại), số giây phải chờ.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

Chưa đăng nhập:

<!-- example: not-signed-in -->
```bash
curl -sS "$API/api/v1/content/entries"
# expect: 401
```

Đầu vào sai, chi tiết bằng tiếng Việt:

<!-- example: invalid-input -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" -H 'Accept-Language: vi' \
  --json '{"kind":"essay","title":"x"}'
# expect: 422
```

Bản ghi không tồn tại:

<!-- example: not-found -->
```bash
curl -sS "$API/api/v1/content/entries/0193e3a4-7b1c-7f00-9a51-6f2b8c0d4e11" -H "Authorization: Bearer $TOKEN"
# expect: 404
```

## Với SDK

Lỗi ném ra `AutoxideError`:

```ts
import { AutoxideError, createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk_…" /* read it from your secret store */, language: "vi" });

try {
  await api.post("/api/v1/content/entries", { body: { kind: "post", title: "" } });
} catch (error) {
  if (!(error instanceof AutoxideError)) throw error;
  switch (error.code) {
    case "validation_failed":
      for (const field of error.errors) console.log(field.field, field.detail);
      break;
    case "rate_limited":
      console.log(`try again in ${error.retryAfter} s`);
      break;
    default:
      console.log(error.status, error.detail, error.requestId);
  }
}
```
