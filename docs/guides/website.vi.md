# Hiện bài viết và ảnh trên website

Tiếng Anh: [`website.md`](website.md). Ví dụ dùng `curl` với biến `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`), và SDK TypeScript (`sdk/typescript`). Chúng chạy như test với mỗi bản build, nên khớp máy chủ của phiên bản này.

Website đọc endpoint **công khai**: không cần đăng nhập, chỉ trả bản đã đăng. Các bước quản trị dưới đây (tải ảnh, viết, đăng) dựng sẵn thứ để đọc.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

## Tải ảnh lên và cho công khai

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

Lệnh sửa nhận `ETag` bạn đã đọc trong `If-Match`, để hai người không ghi đè nhau (`412` nếu người khác vừa sửa).

<!-- example: make-public -->
```bash
curl -sS -X PATCH "$API/api/v1/media/$MEDIA" -H "Authorization: Bearer $TOKEN" -H "If-Match: $MEDIA_ETAG" \
  --json '{"visibility":"public"}' 
```

## Viết và đăng bài

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

## Đọc như website

Bài mới nhất, từng trang:

<!-- example: public-posts -->
```bash
curl -sS "$API/api/v1/content/public/posts?locale=en"
```

Với địa chỉ khách vừa mở, `resolve` cho biết phải hiện gì: `entry` (hiện bài), `redirect` (chuyển khách đi) hay `gone` (trả 410). Thiếu bản dịch thì 404.

`featured_media` và `body_media` cho mỗi ảnh `url`, `alt` (theo ngôn ngữ của bài), kích thước, và `variants`: các bản nhỏ hơn, mỗi bản có bản WebP trong `sources`. Dùng chúng cho `<picture>` và `srcset`:

```html
<picture>
  <source type="image/webp" srcset="…/content?v=…&w=480&format=webp 480w, …">
  <img src="…/content?v=…" srcset="…&w=480 480w, …" sizes="(max-width: 600px) 100vw, 600px" alt="…" width="…" height="…">
</picture>
```

Bản nhỏ được làm nền một lúc sau khi tải lên; trước đó `variants` rỗng và ảnh gốc được trả.

<!-- example: resolve -->
```bash
curl -sS "$API/api/v1/content/public/resolve?locale=en&path=/blog/hello-world"
# save: PAGE_ETAG=header:etag
```

## Cache: ETag và 304

Mọi câu trả lời công khai có `ETag`. Gửi lại trong `If-None-Match`: không có gì đổi thì trả `304` không kèm thân, và bạn dùng tiếp bản đang có. URL ảnh mang phiên bản (`?v=…`) và được cache một năm (`Cache-Control: public, max-age=31536000, immutable`): ảnh đổi thì URL đổi.

<!-- example: not-modified -->
```bash
curl -sS "$API/api/v1/content/public/resolve?locale=en&path=/blog/hello-world" -H "If-None-Match: $PAGE_ETAG"
# expect: 304
```

## Với SDK

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
