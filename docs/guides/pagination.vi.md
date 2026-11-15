# Đọc danh sách dài

Tiếng Anh: [`pagination.md`](pagination.md). Ví dụ dùng `curl` với biến `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`), và SDK TypeScript (`sdk/typescript`). Chúng chạy như test với mỗi bản build, nên khớp máy chủ của phiên bản này.

Mọi danh sách trả một trang: `{"data": [...], "pagination": {"limit", "next_cursor"}}`. Lấy trang kế bằng `cursor` là `next_cursor`; `null` nghĩa là hết. `limit` từ 1 tới 200 (mặc định 50); `order` là `desc` (mới trước, mặc định) hoặc `asc`. Con trỏ vẫn đúng trong lúc bạn đọc, kể cả khi có bản ghi mới.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

<!-- example: first-post -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" --json '{"kind":"post","title":"One"}'
# expect: 201
```

<!-- example: second-post -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" --json '{"kind":"post","title":"Two"}'
# expect: 201
```

<!-- example: third-post -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" --json '{"kind":"post","title":"Three"}'
# expect: 201
```

<!-- example: first-page -->
```bash
curl -sS "$API/api/v1/content/entries?limit=2&order=asc" -H "Authorization: Bearer $TOKEN"
# save: CURSOR=.pagination.next_cursor
```

<!-- example: next-page -->
```bash
curl -sS "$API/api/v1/content/entries?limit=2&order=asc&cursor=$CURSOR" -H "Authorization: Bearer $TOKEN"
```

Giữ nguyên các tham số khác (`limit`, `order`, bộ lọc) giữa các trang; con trỏ của danh sách khác trả `422`.

## Với SDK

`paginate()` tự đi theo con trỏ:

```ts
import { createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk_…" /* read it from your secret store */ });

let count = 0;
for await (const entry of api.paginate("/api/v1/content/entries", { query: { limit: 100, order: "asc" } })) {
  count += 1;
  console.log(entry.id, entry.title);
}
console.log(`${count} entries`);
```
