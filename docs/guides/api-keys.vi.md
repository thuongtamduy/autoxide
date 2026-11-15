# API key cho chương trình và AI agent

Tiếng Anh: [`api-keys.md`](api-keys.md). Ví dụ dùng `curl` với biến `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`), và SDK TypeScript (`sdk/typescript`). Chúng chạy như test với mỗi bản build, nên khớp máy chủ của phiên bản này.

Chương trình (bản build của website, script, AI agent) dùng **API key** thay cho đăng nhập. Mỗi key chỉ cấp đúng permission nó cần: key không bao giờ làm được nhiều hơn bạn, và request vượt permission trả `403`.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

## Tạo key

Tạo key phải nhập mật khẩu hiện tại, để access token bị đánh cắp không tự tạo được key sống lâu hơn nó. Key chỉ hiện **một lần**: cất ngay.

<!-- example: create-key -->
```bash
curl -sS "$API/api/v1/auth/api-keys" -H "Authorization: Bearer $TOKEN" \
  --json '{"name":"Website build","permissions":["content.read"],"current_password":"correct horse battery staple"}'
# expect: 201
# save: KEY=.data.key
# save: KEY_ID=.data.id
```

## Dùng key

Gửi như token: `Authorization: Bearer axk_…`.

<!-- example: use-key -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $KEY"
```

<!-- example: beyond-its-permissions -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $KEY" \
  --json '{"kind":"post","title":"Not allowed"}'
# expect: 403
```

Request bằng API key có giới hạn riêng (`AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE`, mặc định 120). Vượt thì trả `429` kèm `Retry-After`; xem [lỗi](errors.vi.md).

## Xem và thu hồi

<!-- example: list-keys -->
```bash
curl -sS "$API/api/v1/auth/api-keys" -H "Authorization: Bearer $TOKEN"
```

<!-- example: revoke-key -->
```bash
curl -sS -X DELETE "$API/api/v1/auth/api-keys/$KEY_ID" -H "Authorization: Bearer $TOKEN"
# expect: 204
```

<!-- example: revoked-key-is-refused -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $KEY"
# expect: 401
```

## Với SDK

```ts
import { createClient } from "@autoxide/sdk";

// A build script. Never write the key in the code: read it from your secret store.
const api = createClient({ baseUrl: "https://api.example.com", token: "axk_…" /* read it from your secret store */ });

for await (const entry of api.paginate("/api/v1/content/entries", { query: { status: "published" } })) {
  console.log(entry.title);
}
```
