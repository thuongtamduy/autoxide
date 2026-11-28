# SDK TypeScript của Autoxide

Client có kiểu, không phụ thuộc thư viện nào, cho API HTTP của Autoxide. Kiểu trong `src/schema.ts` được sinh từ
`docs/openapi.json` bằng `cargo xtask sdk`, nên luôn khớp máy chủ cùng phiên bản.

Tiếng Anh: [`README.md`](README.md).

## Lấy SDK

SDK không đăng trên npm registry. Chọn một trong hai:

- chép thư mục `sdk/typescript/src` vào dự án của bạn, hoặc
- cài gói đính kèm mỗi bản phát hành, vd
  `npm install https://github.com/<owner>/autoxide/releases/download/vX.Y.Z/autoxide-sdk-X.Y.Z.tgz`.

Đây là mã nguồn TypeScript (`.ts`), cho dự án có biên dịch TypeScript (Vite, Next.js, `tsc`, Bun, Deno).
Dùng SDK cùng phiên bản với máy chủ.

## Dùng

```ts
import { AutoxideError, createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk_…", language: "en" });

// Path and query parameters, bodies and results are typed from the API contract.
const menu = await api.get("/api/v1/menus/public/{key}", { path: { key: "main" }, query: { locale: "en" } });

// Writes can be retried safely with an idempotency key.
await api.post("/api/v1/menus", {
  body: { key: "footer", items: [{ label: "Contact", url: "mailto:hello@example.com" }] },
  idempotencyKey: crypto.randomUUID(),
});

// Every item of a list, page after page.
for await (const item of api.paginate("/api/v1/menus")) console.log(item.key);

try {
  await api.delete("/api/v1/menus/{id}", { path: { id: "…" } });
} catch (error) {
  if (error instanceof AutoxideError && error.code === "menus.menu_not_found") {
    // Branch on `code`; show `detail`; `errors` lists invalid fields; `retryAfter` for 429 / 503.
  }
}
```

- Câu trả lời `{"data": …}` trả về phần `data`; một trang (`data` + `pagination`) giữ nguyên.
- `GET` kèm `ifNoneMatch: etag` trả `{ notModified: true }` khi không có gì đổi.
- `token` có thể là một hàm, để đọc access token mới cho mỗi request.

### Header và `ETag`: `requestWithHeaders` và `ifMatch`

`requestWithHeaders` trả đúng thứ `request` trả, thêm status và header:

```ts
const { data: entry, etag, status, headers } = await api.requestWithHeaders("get", "/api/v1/content/entries/{id}", {
  path: { id },
});

// Change it only if nobody changed it since: `If-Match`. Otherwise 412 `precondition_failed`.
await api.patch("/api/v1/content/entries/{id}", { path: { id }, ifMatch: etag, body: { title: "New title" } });
```

`data` chỉ có thể là `{ notModified: true }` khi bạn truyền `ifNoneMatch`.

### Tự refresh access token: `refresh`

```ts
let tokens = { access: "", refresh: "" };
const auth = createClient({ baseUrl: "https://api.example.com" }); // login, refresh, logout
const api = createClient({
  baseUrl: "https://api.example.com",
  token: () => tokens.access,
  refresh: async () => {
    const next = await auth.post("/api/v1/auth/refresh", { body: { refresh_token: tokens.refresh } });
    tokens = { access: next.access_token, refresh: next.refresh_token };
  },
});
```

Khi một request gặp `401 auth.token_expired`, client gọi `refresh` rồi gửi lại request đó một lần với token mới.
Các request lỗi cùng lúc chờ **một** lần gọi chung: refresh token chỉ dùng được một lần, hai lần refresh bằng nó
làm kết thúc cả phiên (`auth.refresh_token_reused`). `refresh` ném lỗi thì request báo đúng lỗi đó (cho người dùng
đăng nhập lại). Mã `401` khác không refresh.

Dựng ứng dụng web (CORS, lưu token, nhiều tab, lỗi, tải ảnh, cả file `api.ts`):
[`docs/guides/web-app.vi.md`](../../docs/guides/web-app.vi.md). Hướng dẫn theo tình huống, có ví dụ `curl` và SDK:
[`docs/guides/`](../../docs/guides/).

## Cho người đóng góp

`src/schema.ts` là file sinh ra: đổi API, sinh lại `docs/openapi.json`, rồi chạy `cargo xtask sdk`. Gate chạy
`cargo xtask sdk --check`; CI kiểm kiểu SDK và ví dụ của hướng dẫn bằng `npm ci && npx tsc --noEmit` (TypeScript 7, bản dùng để phát triển) và chạy lại bằng TypeScript 5.0.4, rồi chạy test của client bằng `npm test` (`node --test`, Node 22.18 trở lên, không thêm gói nào).

TypeScript hỗ trợ cho dự án nhập SDK: **5.0 trở lên** (đã kiểm ở 5.0.4, 5.9.3, 6.0.3 và 7.0.2). `moduleResolution: "Bundler"` cần 5.0.
