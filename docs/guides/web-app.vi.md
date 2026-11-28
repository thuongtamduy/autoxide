# Dựng ứng dụng web với Autoxide

Tiếng Anh: [`web-app.md`](web-app.md). Ví dụ dùng `curl` với `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`), và SDK TypeScript (`sdk/typescript`). Chúng chạy như test với mỗi bản build, nên khớp máy chủ của phiên bản này.

Hướng dẫn này cho người làm frontend — trang quản trị, cửa hàng, website — chạy trong trình duyệt, bằng
React, Vue, Svelte hay TypeScript thuần. Autoxide chỉ là API; ứng dụng của bạn là một dự án riêng. Cuối trang
có cả một file `api.ts` để chép vào ứng dụng.

- [1. Cho trình duyệt gọi API (CORS)](#1-cho-trình-duyệt-gọi-api-cors)
- [2. Tạo client](#2-tạo-client)
- [3. Đăng nhập, giữ phiên, đăng xuất](#3-đăng-nhập-giữ-phiên-đăng-xuất)
- [4. Chỉ hiện việc người dùng được làm](#4-chỉ-hiện-việc-người-dùng-được-làm)
- [5. Sửa bản ghi mà không ghi đè người khác (ETag)](#5-sửa-bản-ghi-mà-không-ghi-đè-người-khác-etag)
- [6. Lỗi và gọi lại](#6-lỗi-và-gọi-lại)
- [7. Tải ảnh lên và hiện ảnh](#7-tải-ảnh-lên-và-hiện-ảnh)
- [8. Viết bài: định dạng thân bài](#8-viết-bài-định-dạng-thân-bài)
- [9. Trang công khai: ngôn ngữ và website](#9-trang-công-khai-ngôn-ngữ-và-website)
- [10. Việc nào gọi endpoint nào](#10-việc-nào-gọi-endpoint-nào)
- [11. Cả file `api.ts`](#11-cả-file-apits)

## 1. Cho trình duyệt gọi API (CORS)

Một trang ở origin khác (`http://localhost:5173` khi phát triển, `https://admin.example.com` về sau) chỉ gọi
được API khi máy chủ liệt kê origin đó trong `AUTOXIDE_HTTP_CORS_ORIGINS` (ngăn cách bằng dấu phẩy; `*` cho mọi
origin, không dùng cho production). Thiếu biến này, trình duyệt chặn câu trả lời và console báo "blocked by CORS
policy". Với bản demo:
```bash
AUTOXIDE_HTTP_CORS_ORIGINS=http://localhost:5173 ./autoxide demo
```

Với Docker, truyền bằng `-e AUTOXIDE_HTTP_CORS_ORIGINS=http://localhost:5173`. Xem thêm ở
[Bắt đầu](../GETTING-STARTED.vi.md#gọi-api-từ-trình-duyệt-cors).

Thông tin đăng nhập gửi trong header `Authorization`, không bao giờ qua cookie, nên không cần cấu hình gì cho
cookie (không cần `credentials: "include"`). JavaScript đọc được các header sau của câu trả lời: `ETag`,
`Location`, `Retry-After`, `X-Request-Id`, `Idempotent-Replayed`, `Content-Disposition`, `Content-Range`,
`Accept-Ranges`, `Link`, `Deprecation`, `Sunset` và `Mcp-Session-Id`.

## 2. Tạo client

Chép `sdk/typescript/src` vào dự án hoặc cài gói của bản phát hành ứng với máy chủ (xem
[README của SDK](../../sdk/typescript/README.vi.md)). Đường dẫn, tham số, thân request và câu trả lời có kiểu
lấy từ hợp đồng API, nên trình soạn thảo tự gợi ý.
```ts
import { createClient } from "@autoxide/sdk";

// Public endpoints need no token.
const api = createClient({ baseUrl: "https://api.example.com", language: "vi" });

const posts = await api.get("/api/v1/content/public/posts", { query: { locale: "vi", limit: 10 } });
console.log(posts.data.map((post) => post.title));
```

`language` chọn ngôn ngữ của thông báo lỗi (`detail`): `en` (mặc định) hoặc `vi`. Câu trả lời `{"data": …}` trả
về phần `data`; danh sách giữ nguyên `data` và `pagination` (xem [Đọc danh sách dài](pagination.vi.md)).

## 3. Đăng nhập, giữ phiên, đăng xuất

`POST /api/v1/auth/login` trả `200` với access token (khoảng 15 phút, `expires_in` giây) và refresh token,
hoặc `202` với `mfa_token` khi tài khoản bật đăng nhập hai bước: hỏi mã 6 số rồi gửi cả hai tới
`POST /api/v1/auth/login/mfa`. Chi tiết: [Đăng nhập](authentication.vi.md).

**Lưu token ở đâu.** API dùng bearer token, không dùng cookie. Giữ access token trong bộ nhớ. Refresh token
phải còn sau khi tải lại trang thì người dùng mới không bị đăng xuất: `localStorage` làm được, nhưng mọi script
chạy trên trang đều đọc được nó, nên đừng nạp script bên thứ ba trên các trang giữ token và đặt Content Security
Policy chặt. `sessionStorage` kết thúc phiên khi đóng tab. Nếu framework có máy chủ riêng (Next.js, Nuxt,
SvelteKit), máy chủ đó có thể giữ refresh token trong cookie `HttpOnly` của chính nó và gọi API thay trình duyệt.

**Refresh khi access token hết hạn.** Access token hết hạn trả `401` với mã `auth.token_expired`. Truyền cho
client một hàm `refresh`: client sẽ gọi nó, rồi gửi lại request một lần với token mới. Refresh token chỉ dùng
được **một lần**. Hai request cùng refresh một lúc bằng cùng một refresh token thì máy chủ coi là token bị dùng
lại, từ chối (`auth.refresh_token_reused`) và **kết thúc cả phiên**. Client chỉ chạy một `refresh` cho các
request lỗi cùng lúc; các tab đang mở dùng chung token đã lưu, nên `api.ts` bên dưới còn giữ một
[Web Lock](https://developer.mozilla.org/docs/Web/API/Web_Locks_API) quanh lần refresh và đọc refresh token đã
lưu ngay bên trong lock, để không tab nào refresh bằng token mà tab khác đã dùng.
```ts
import { createClient } from "@autoxide/sdk";

let access: string | undefined;
let refreshToken = localStorage.getItem("refresh_token") ?? undefined;

// Login, refresh and logout go through a client without a token.
const auth = createClient({ baseUrl: "https://api.example.com" });

const api = createClient({
  baseUrl: "https://api.example.com",
  token: () => access,
  // Called once for all the requests that got `auth.token_expired` together; they are then sent again.
  refresh: async () => {
    if (!refreshToken) throw new Error("signed out");
    const next = await auth.post("/api/v1/auth/refresh", { body: { refresh_token: refreshToken } });
    access = next.access_token;
    refreshToken = next.refresh_token;
    localStorage.setItem("refresh_token", refreshToken);
  },
});

const me = await api.get("/api/v1/auth/me");
console.log(me.name);
```

Mã `401` khác không refresh: `auth.session_ended` (đã đăng xuất, đổi mật khẩu, phiên bị thu hồi),
`auth.invalid_token`, `auth.missing_token`, và `auth.refresh_token_reused` hay `auth.invalid_refresh_token` do
chính `/refresh` trả. Khi đó quên token và hiện trang đăng nhập.

**Đăng xuất** bằng `POST /api/v1/auth/logout` với refresh token (`204`). Access token của phiên ngừng dùng được
ngay. Quên cả hai token kể cả khi lệnh gọi lỗi (mất mạng): phiên sẽ tự kết thúc khi refresh token hết hạn.

## 4. Chỉ hiện việc người dùng được làm

`GET /api/v1/auth/me` liệt kê `permissions` của người dùng (tên như `content.write`;
[tài liệu OpenAPI](../openapi.json) và hướng dẫn của từng module nêu permission mỗi endpoint cần). Khi
`all_permissions` là `true` (role `admin`), `permissions` **rỗng** và người dùng được làm mọi việc: kiểm
`all_permissions` trước.
```ts
import { createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: () => sessionStorage.getItem("access") ?? undefined });

const me = await api.get("/api/v1/auth/me");
const can = (permission: string) => me.all_permissions || me.permissions.includes(permission);

const publishButton = document.querySelector<HTMLButtonElement>("#publish");
if (publishButton) publishButton.hidden = !can("content.publish");
if (me.mfa_required) {
  // A role of this user needs two-step sign-in: until it is on, that role's permissions are left out.
}
```

- Ẩn nút chỉ để dễ dùng: máy chủ kiểm mọi request và trả `403` (`permission_denied`) khi thiếu permission.
- Tài khoản tự đăng ký (`POST /api/v1/auth/register`, bật khi máy chủ đặt
  `AUTOXIDE_AUTH_SELF_REGISTRATION=true`) **không có role và permission nào** cho tới khi quản trị viên gán role
  (`PUT /api/v1/auth/users/{id}/roles`, permission `auth.roles.manage`). Với người dùng đó, hiện màn hình "đang
  chờ cấp quyền" thay vì trang quản trị trống.
- Đọc lại `/auth/me` sau khi đổi role; không cần access token mới.

## 5. Sửa bản ghi mà không ghi đè người khác (ETag)

Đọc một bản ghi trả phiên bản của nó trong header `ETag`. Sửa bản ghi thì gửi lại phiên bản đó trong `If-Match`:
bản ghi đã bị người khác sửa trong lúc đó trả `412` (`precondition_failed`), và endpoint bắt buộc header này trả
`428` (`precondition_required`) khi thiếu. `requestWithHeaders` cho bạn header, còn `ifMatch` gửi phiên bản:
```ts
import { AutoxideError, createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: () => sessionStorage.getItem("access") ?? undefined });

async function rename(id: string, title: string) {
  const { data: entry, etag } = await api.requestWithHeaders("get", "/api/v1/content/entries/{id}", { path: { id } });
  console.log("was", entry.title);
  try {
    return await api.patch("/api/v1/content/entries/{id}", { path: { id }, ifMatch: etag, body: { title } });
  } catch (error) {
    if (error instanceof AutoxideError && error.code === "precondition_failed") {
      // Someone saved first: read again, show their version, let the user apply the change again.
    }
    throw error;
  }
}
```

Câu trả lời của lệnh sửa cũng mang `ETag` mới (`requestWithHeaders("patch", …)`), nên có thể lưu hai lần liên
tiếp mà không cần đọc lại. Xem thêm [Đọc lỗi](errors.vi.md#sửa-một-bản-ghi-if-match-428-và-412).

## 6. Lỗi và gọi lại

Mọi lỗi là một Problem có `code` ổn định; SDK ném nó thành `AutoxideError` (`status`, `code`, `detail`,
`errors`, `retryAfter`, `requestId`). [Bảng mã lỗi cho ứng dụng](errors.vi.md#ứng-dụng-nên-làm-gì) liệt kê các
mã thường gặp và cách xử lý từng mã. Tóm tắt:

| Status | Làm gì |
| ------ | ------ |
| `401` | `auth.token_expired`: refresh (client làm khi có `refresh`). Mã khác: về trang đăng nhập. `step_up_required`: hỏi lại mật khẩu hay mã (`POST /api/v1/auth/me/reauth`) |
| `403` | Không được phép: ẩn thao tác, hoặc bảo người dùng xin quyền |
| `404` | Đã mất hoặc chưa từng có: tải lại danh sách |
| `412` / `428` | Đọc lại bản ghi, rồi áp thay đổi lên bản vừa đọc |
| `422` | Hiện `errors[]` cạnh trường tương ứng (`field` là đường dẫn như `items[0].url`) |
| `429` | Chờ `Retry-After` giây; không bao giờ tự đăng nhập lại |

Lệnh ghi lỗi mạng có thể vẫn đã chạy. Gửi `idempotencyKey` (một `crypto.randomUUID()` mới cho mỗi thao tác của
người dùng, **cùng** key cho mọi lần gọi lại thao tác đó): lần lặp lại trả kết quả đầu tiên thay vì làm hai lần.
Xem [Gọi lại an toàn](retries.vi.md).
```ts
import { AutoxideError, createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: () => sessionStorage.getItem("access") ?? undefined });

async function withRetry<T>(send: () => Promise<T>, tries = 3): Promise<T> {
  for (let attempt = 1; ; attempt += 1) {
    try {
      return await send();
    } catch (error) {
      const busy = error instanceof AutoxideError && (error.status === 429 || error.status === 503);
      const network = error instanceof TypeError; // fetch failed: offline, DNS, connection reset
      if (attempt >= tries || !(busy || network)) throw error;
      const seconds = error instanceof AutoxideError ? (error.retryAfter ?? 1) : attempt;
      await new Promise((resolve) => setTimeout(resolve, seconds * 1000));
    }
  }
}

const key = crypto.randomUUID(); // one key for this one action, reused by every retry
const created = await withRetry(() =>
  api.post("/api/v1/content/entries", { body: { kind: "post", title: "Hello" }, idempotencyKey: key }),
);
console.log(created.id);
```

## 7. Tải ảnh lên và hiện ảnh

Gửi chính file làm thân của `POST /api/v1/media?name=…` (không phải form multipart); trình duyệt tự đặt
`Content-Type` theo `File`. Mục mới là `private`: đổi `visibility` thành `public` trước khi dùng trên trang công
khai hay trong bài sẽ đăng. `url` của mục là đường dẫn **tương đối** (`/api/v1/media/…/content?v=…`): ghép địa
chỉ máy chủ vào trước.
```ts
import { createClient } from "@autoxide/sdk";

const API_URL = "https://api.example.com";
const api = createClient({ baseUrl: API_URL, token: () => sessionStorage.getItem("access") ?? undefined });

async function uploadPicture(file: File, alt: string) {
  const uploaded = await api.post("/api/v1/media", {
    query: { name: file.name },
    body: file,
    idempotencyKey: crypto.randomUUID(),
  });
  const { etag } = await api.requestWithHeaders("get", "/api/v1/media/{id}", { path: { id: uploaded.id } });
  const media = await api.patch("/api/v1/media/{id}", {
    path: { id: uploaded.id },
    ifMatch: etag,
    body: { visibility: "public", alt },
  });
  return { id: media.id, src: new URL(media.url, API_URL).href };
}

const input = document.querySelector<HTMLInputElement>("input[type=file]");
const file = input?.files?.[0];
if (file) console.log((await uploadPicture(file, "A lake at dawn")).src);
```

`url` của mục riêng tư cần header `Authorization` (thiếu thì trả `404`), nên `<img>` không hiện được; hiện bản
xem trước từ `File` (`URL.createObjectURL(file)`) cho tới khi nó công khai. Bản nhỏ hơn (`variants`, kèm WebP)
có sau khi tải lên một lúc — xem [Hiện bài viết và ảnh trên website](website.vi.md). File lớn hơn giới hạn tải
lên của máy chủ thì [tải thẳng lên nơi lưu](direct-uploads.vi.md).

## 8. Viết bài: định dạng thân bài

`body` của bài viết hay trang là một **tài liệu JSON của ProseMirror** — dạng mà [Tiptap](https://tiptap.dev)
và các editor ProseMirror sinh bằng `editor.getJSON()`, với tên node của StarterKit của Tiptap (`paragraph`,
`heading`, `bulletList`, `orderedList`, `listItem`, `blockquote`, `codeBlock`, `horizontalRule`, `hardBreak`,
`text`) và bảng (`table`, `tableRow`, `tableHeader`, `tableCell`).
```json
{"type": "doc", "content": [
  {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Hello"}]},
  {"type": "paragraph", "content": [
    {"type": "text", "text": "Read the "},
    {"type": "text", "text": "guide", "marks": [{"type": "link", "attrs": {"href": "https://example.com"}}]}
  ]},
  {"type": "image", "attrs": {"mediaId": "0193a0c4-7d1e-7c2a-9b1e-3f2d5c6b7a81", "alt": "A lake", "caption": ""}}
]}
```

Máy chủ chỉ nhận một danh sách cho phép, còn lại từ chối với `422` và nêu vị trí (`body.content[2].attrs.href`):

- tiêu đề chỉ cấp **2 tới 4** (tiêu đề của trang là cấp 1);
- mark: `bold`, `italic`, `underline`, `strike`, `code`, `link` (chỉ `attrs.href`; không nhận `javascript:`);
- `image` nhận `attrs.mediaId` (một mục của thư viện media), `alt` và `caption` — **không** nhận `src`; extension
  ảnh của Tiptap cần một node tự viết hoặc chuyển đổi trước khi lưu;
- `embed` (`attrs.url` ở host máy chủ cho phép, hoặc `attrs.kind` + `attrs.id` của bản ghi module khác) là node
  riêng của Autoxide;
- thuộc tính lạ bị từ chối: tắt các phần thêm của editor sinh thuộc tính (căn lề, màu, `type` của
  `orderedList`) hoặc bỏ chúng trước khi lưu.

Danh sách đầy đủ là schema `BodyNode` trong [tài liệu OpenAPI](../openapi.json) (cũng có ở `/api/docs` khi máy
chủ bật), còn [`CONTENT.vi.md`](../CONTENT.vi.md) giải thích bản nháp, đăng bài và địa chỉ. Máy chủ không lưu
HTML: tự render JSON (`generateHTML` của Tiptap, hay component của bạn), và lấy URL ảnh từ `featured_media` /
`body_media` của câu trả lời công khai.

## 9. Trang công khai: ngôn ngữ và website

Endpoint công khai (`/api/v1/content/public/…`, `/api/v1/catalog/public/…`, `/api/v1/menus/public/…`) không cần
đăng nhập và chỉ trả nội dung đã đăng.

- `locale` **bắt buộc** ở phần lớn endpoint đọc công khai — danh sách nội dung, `resolve`, chuyên mục và feed
  (`/content/public/posts`, `/content/public/resolve`…), và mọi endpoint đọc của catalog
  (`/catalog/public/products`…): gọi thiếu thì trả `422` (`validation_failed`, trường `locale`). Tài liệu
  OpenAPI ghi rõ bắt buộc ở từng endpoint; `/content/public/site` và `/menus/public/{key}` dùng ngôn ngữ mặc
  định khi thiếu.
- Mỗi module có danh sách ngôn ngữ riêng: `AUTOXIDE_CONTENT_LOCALES`, `AUTOXIDE_CATALOG_LOCALES`,
  `AUTOXIDE_MENUS_LOCALES` (mặc định `en,vi`; `AUTOXIDE_CONTENT_DEFAULT_LOCALE` và
  `AUTOXIDE_MENUS_DEFAULT_LOCALE` nêu ngôn ngữ mặc định). Hiện **chưa có endpoint liệt kê** các ngôn ngữ này: ghi
  danh sách vào cấu hình của ứng dụng, cạnh địa chỉ API. Ngôn ngữ chưa cấu hình trả `422` (content:
  `locale_not_supported`, có `detail` nêu các ngôn ngữ được nhận; catalog: `catalog.unsupported_locale`).
- `site` chọn website (một đơn vị tổ chức) khi trang không chạy trên tên miền riêng của website đó; trên tên miền
  đã gắn với một đơn vị thì tên miền tự chọn. `GET /api/v1/organizations/public/site` cho biết tên miền là
  website nào (`unit_id` của nó là giá trị cho `site`).
- Câu trả lời công khai có thể cũ tới một phút: xem
  [Cache, và xem bài vừa đăng](website.vi.md#cache-và-xem-bài-vừa-đăng).

## 10. Việc nào gọi endpoint nào

API có hơn một nghìn endpoint, chia theo module (mỗi module một tag trong `/api/docs`). Những endpoint ứng dụng
web cần nhiều nhất:

| Việc | Endpoint | Permission |
| ---- | -------- | ---------- |
| Đăng nhập / bước hai / refresh / đăng xuất | `POST /api/v1/auth/login`, `/login/mfa`, `/refresh`, `/logout` | — |
| Tạo tài khoản (khi được phép) | `POST /api/v1/auth/register` | — |
| Quên mật khẩu | `POST /api/v1/auth/password-reset`, rồi `/password-reset/confirm` với token trong thư | — |
| Tôi là ai, được làm gì | `GET /api/v1/auth/me` | đã đăng nhập |
| Đổi tên, ngôn ngữ, múi giờ của tôi | `PATCH /api/v1/auth/me` | đã đăng nhập |
| Các thiết bị đang đăng nhập | `GET /api/v1/auth/me/sessions` | đã đăng nhập |
| Người dùng và role của họ | `GET /api/v1/auth/users`, `PUT /api/v1/auth/users/{id}/roles` | `auth.users.read`, `auth.roles.manage` |
| Role | `GET`/`POST /api/v1/auth/roles` | `auth.roles.read`, `auth.roles.manage` |
| Tải lên, liệt kê, công khai ảnh | `POST`/`GET /api/v1/media`, `PATCH /api/v1/media/{id}` | `media.write`, `media.read` |
| Liệt kê, tạo, sửa bài viết và trang | `GET`/`POST /api/v1/content/entries`, `PATCH /api/v1/content/entries/{id}` | `content.read`, `content.write` |
| Đăng, gỡ bài | `POST /api/v1/content/entries/{id}/publish`, `/unpublish` | `content.publish` |
| Chuyên mục và thẻ | `GET`/`POST /api/v1/content/terms` | `content.read`, `content.manage` |
| Menu của website | `GET`/`POST /api/v1/menus`, `PUT /api/v1/menus/{id}`; công khai `GET /api/v1/menus/public/{key}` | `menus.read`, `menus.manage` |
| Website: danh sách, trang theo địa chỉ, khung trang | `GET /api/v1/content/public/posts`, `/public/resolve`, `/public/site` | — |
| Sản phẩm của cửa hàng | `GET /api/v1/catalog/public/products` | — |
| Form liên hệ | `POST /api/v1/forms/public/{key}/submissions` | — |
| Cập nhật trực tiếp | `GET /api/v1/kernel/events/stream` ([Server-Sent Events](realtime-events.vi.md)) | đã đăng nhập |

Cửa hàng có hướng dẫn riêng: [Mở một cửa hàng](shop.vi.md) và [Dựng trang chủ của cửa hàng](storefront.vi.md).
Việc khác thì tìm danh từ bạn cần trong `docs/openapi.json` (hay `/api/docs`); mỗi module có một hướng dẫn trong
[`docs/`](../MODULES.vi.md).

## 11. Cả file `api.ts`

Một file cho ứng dụng: client, đăng nhập (kèm bước hai), refresh an toàn giữa các tab, đăng xuất, permission,
lưu với `If-Match`, gọi lại, tải ảnh và trang công khai. Đổi `API_URL` và `LOCALES`.
```ts
// api.ts — the Autoxide API for a web app.
import { AutoxideError, createClient } from "@autoxide/sdk";

export const API_URL = "https://api.example.com";
/** The server's AUTOXIDE_CONTENT_LOCALES: no endpoint lists them. */
export const LOCALES = ["en", "vi"] as const;
const REFRESH_KEY = "autoxide.refresh_token";

let access: string | undefined;
let signedOutListener: () => void = () => {};

/** Called when the session ends (signed out elsewhere, refresh refused): show the sign-in page. */
export function onSignedOut(listener: () => void) {
  signedOutListener = listener;
}

function storedRefresh(): string | undefined {
  try {
    return localStorage.getItem(REFRESH_KEY) ?? undefined;
  } catch {
    return undefined; // storage blocked (private mode): the session lasts as long as the page
  }
}

function keep(tokens: { access_token: string; refresh_token: string } | undefined) {
  access = tokens?.access_token;
  try {
    if (tokens) localStorage.setItem(REFRESH_KEY, tokens.refresh_token);
    else localStorage.removeItem(REFRESH_KEY);
  } catch {
    // storage blocked: keep the access token in memory only
  }
}

const language = navigator.language.toLowerCase().startsWith("vi") ? "vi" : "en";

/** For sign-in, refresh, sign-out and public pages: no token. */
export const publicApi = createClient({ baseUrl: API_URL, language });

/** Runs `refresh` in one tab at a time (Web Locks), so two tabs never spend the same refresh token. */
async function oneTabAtATime(work: () => Promise<void>) {
  if (typeof navigator !== "undefined" && "locks" in navigator) {
    await navigator.locks.request("autoxide-refresh", work);
  } else {
    await work();
  }
}

async function refresh() {
  await oneTabAtATime(async () => {
    // Read inside the lock: a tab that refreshed just before saved the newest refresh token here.
    const current = storedRefresh();
    if (!current) throw new Error("signed out");
    try {
      keep(await publicApi.post("/api/v1/auth/refresh", { body: { refresh_token: current } }));
    } catch (error) {
      keep(undefined);
      signedOutListener();
      throw error;
    }
  });
}

/** For everything that needs a signed-in user. */
export const api = createClient({ baseUrl: API_URL, language, token: () => access, refresh });

export type SignIn = { signedIn: true } | { signedIn: false; mfaToken: string; methods: string[] };

export async function signIn(email: string, password: string): Promise<SignIn> {
  const answer = await publicApi.post("/api/v1/auth/login", { body: { email, password } });
  if ("mfa_token" in answer) return { signedIn: false, mfaToken: answer.mfa_token, methods: answer.methods };
  keep(answer);
  return { signedIn: true };
}

/** The second step: the 6-digit code of the authenticator app, or a recovery code. */
export async function signInWithCode(mfaToken: string, code: string) {
  keep(await publicApi.post("/api/v1/auth/login/mfa", { body: { mfa_token: mfaToken, code } }));
}

/** After a reload: true when a stored refresh token still works. */
export async function resume(): Promise<boolean> {
  if (!storedRefresh()) return false;
  try {
    await refresh();
    return true;
  } catch {
    return false;
  }
}

export async function signOut() {
  const refreshToken = storedRefresh();
  keep(undefined);
  if (refreshToken) {
    await publicApi.post("/api/v1/auth/logout", { body: { refresh_token: refreshToken } }).catch(() => undefined);
  }
}

/** `can("content.publish")`: for showing or hiding buttons. The server still checks every request. */
export async function permissions() {
  const me = await api.get("/api/v1/auth/me");
  return {
    me,
    can: (permission: string) => me.all_permissions || me.permissions.includes(permission),
    waitingForAccess: !me.all_permissions && me.permissions.length === 0,
  };
}

/** Retries a request on 429, 503 and network errors. Give writes an `idempotencyKey`. */
export async function withRetry<T>(send: () => Promise<T>, tries = 3): Promise<T> {
  for (let attempt = 1; ; attempt += 1) {
    try {
      return await send();
    } catch (error) {
      const busy =
        error instanceof AutoxideError &&
        (error.status === 503 || (error.status === 429 && error.code !== "auth.login_throttled"));
      if (attempt >= tries || !(busy || error instanceof TypeError)) throw error;
      const seconds = error instanceof AutoxideError ? (error.retryAfter ?? 1) : attempt;
      await new Promise((resolve) => setTimeout(resolve, seconds * 1000));
    }
  }
}

/** The fields a form shows next to its inputs, from a 422. */
export function fieldErrors(error: unknown): Record<string, string> {
  const out: Record<string, string> = {};
  if (error instanceof AutoxideError) for (const field of error.errors) out[field.field] = field.detail;
  return out;
}

/** Changes a post's title with the version just read; `false` when someone saved first (412). */
export async function renameEntry(id: string, title: string): Promise<boolean> {
  const { etag } = await api.requestWithHeaders("get", "/api/v1/content/entries/{id}", { path: { id } });
  try {
    await api.patch("/api/v1/content/entries/{id}", { path: { id }, ifMatch: etag, body: { title } });
    return true;
  } catch (error) {
    if (error instanceof AutoxideError && error.code === "precondition_failed") return false;
    throw error;
  }
}

/** Uploads a picture, makes it public and returns its full URL. */
export async function uploadPicture(file: File, alt: string) {
  const key = crypto.randomUUID();
  const uploaded = await withRetry(() =>
    api.post("/api/v1/media", { query: { name: file.name }, body: file, idempotencyKey: key }),
  );
  const { etag } = await api.requestWithHeaders("get", "/api/v1/media/{id}", { path: { id: uploaded.id } });
  const media = await api.patch("/api/v1/media/{id}", {
    path: { id: uploaded.id },
    ifMatch: etag,
    body: { visibility: "public", alt },
  });
  return { id: media.id, src: mediaUrl(media.url) };
}

/** Media URLs are relative to the server. */
export const mediaUrl = (url: string) => new URL(url, API_URL).href;

/** What to show at an address of the website. */
export async function page(path: string, locale: (typeof LOCALES)[number]) {
  const answer = await publicApi.get("/api/v1/content/public/resolve", { query: { locale, path } });
  switch (answer.type) {
    case "entry":
      return { kind: "entry" as const, entry: answer.entry };
    case "redirect":
      return { kind: "redirect" as const, to: answer.location, status: answer.status };
    default:
      return { kind: "gone" as const };
  }
}
```
