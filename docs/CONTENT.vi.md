# Nội dung

*English: [CONTENT.md](CONTENT.md)*

Module `content` giữ bài viết và trang của một website, nhiều ngôn ngữ, có bản nháp, lịch sử,
hẹn giờ, danh mục, thẻ, tác giả, menu và chuyển hướng. Module chỉ có API: trang quản trị và
chính website là các dự án riêng gọi vào nó. Module nằm dưới `/api/v1/content`. Hợp đồng đầy
đủ, với mọi tham số và mã lỗi, ở [`openapi.json`](openapi.json) (và ở `/api/docs` khi
`AUTOXIDE_HTTP_API_DOCS=true`).

Ảnh là item của thư viện media ([`MEDIA.vi.md`](MEDIA.vi.md)): một mục chỉ giữ ID của ảnh, và
ảnh phải công khai thì mục mới đăng được.

## Permission

| Permission        | Cho phép                                                                          |
| ----------------- | --------------------------------------------------------------------------------- |
| `content.read`    | Đọc bản nháp, revision và lịch                                                    |
| `content.write`   | Tạo và sửa bản nháp, bản dịch, khôi phục revision, tạo link xem trước             |
| `content.publish` | Đăng, gỡ, lưu trữ, xoá, chuyển trang, hẹn giờ; danh mục và tác giả của mục đang công khai |
| `content.manage`  | Danh mục, thẻ, tác giả, menu, chuyển hướng                                        |

Gán cho một role bằng `PUT /api/v1/auth/roles/{id}`. Role `admin` có đủ. Người đọc website
không cần permission nào: mọi thứ dưới `/api/v1/content/public/` đều mở.

## Mô hình

Một **mục** (entry) là bài viết (`kind: post`) hoặc trang (`kind: page`, có thể có trang cha).
Mỗi mục có một **bản nháp** — luôn có, là bản người biên tập sửa — và, khi đang công khai, một
**bản công khai** riêng mà người đọc thấy. Sửa bản nháp không đổi thứ người đọc thấy cho tới khi
đăng lại (`has_unpublished_changes` cho biết).

- `status` là `draft`, `published` hoặc `archived`.
- `slug` là phần cuối của địa chỉ: tự tạo từ tiêu đề khi không gửi (`"Đường đến Đà Lạt"` →
  `duong-den-da-lat`; thêm `-2`, `-3`… khi trùng), chữ thường và số nối bằng `-`, không trùng
  giữa các mục cùng loại, cùng ngôn ngữ, cùng trang cha.
- `body` là một tài liệu JSON của [ProseMirror](https://prosemirror.net) /
  [Tiptap](https://tiptap.dev). Máy chủ kiểm theo danh sách node, mark và thuộc tính được phép
  (đoạn văn, tiêu đề cấp 2–4, danh sách, trích dẫn, code, bảng, ảnh, nhúng video từ
  `AUTOXIDE_CONTENT_EMBED_HOSTS`; đậm, nghiêng, gạch dưới, gạch ngang, code, link), từ chối link
  `javascript:` và link không an toàn khác, ảnh chỉ giữ ID media, và báo mọi lỗi một lần kèm vị
  trí (`body.content[2].attrs.href`). Máy chủ không lưu HTML: website tự dựng từ JSON.
- Máy chủ tính `word_count`, `reading_time_minutes` và một bản chữ thuần để tìm kiếm (không phân
  biệt hoa thường và dấu: `da lat` tìm ra `Đà Lạt`).
- Trường SEO: `meta_title`, `meta_description`, `og_media_id` (ảnh chia sẻ), `canonical_url`,
  `noindex`.

## Viết và đăng

```bash
# Tạo bản nháp
curl -X POST http://localhost:8080/api/v1/content/entries \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -H "Idempotency-Key: $(uuidgen)" \
  -d '{"kind": "post", "locale": "en", "title": "Hello",
       "body": {"type": "doc", "content": [{"type": "paragraph",
                "content": [{"type": "text", "text": "First post."}]}]}}'

# Đọc lại: để ý header ETag
curl -i http://localhost:8080/api/v1/content/entries/$ID -H "Authorization: Bearer $TOKEN"

# Sửa bản nháp: bắt buộc If-Match
curl -X PATCH http://localhost:8080/api/v1/content/entries/$ID \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -H 'If-Match: "1731400000000000"' -d '{"excerpt": "A short summary."}'

# Đăng
curl -X POST http://localhost:8080/api/v1/content/entries/$ID/publish \
  -H "Authorization: Bearer $TOKEN" -H 'If-Match: "1731400000000001"'
```

- **Bắt buộc `If-Match`** khi sửa hay xoá mục, khi đổi trạng thái mục (đăng, gỡ, lưu trữ,
  chuyển, khôi phục, hẹn giờ), khi sửa danh mục / thẻ, tác giả, chuyển hướng, và khi thay menu:
  gửi `ETag` vừa đọc (`*` để sửa bất kể phiên bản). Thiếu: `428 precondition_required`; có người
  sửa trước: `412 precondition_failed` — đọc lại rồi sửa lại. Nhờ vậy hai người biên tập (hay hai
  AI agent) không ghi đè lên nhau. Xoá danh mục / thẻ, tác giả, chuyển hướng và gắn danh mục / tác
  giả cho mục thì không cần: không có gì bị ghi đè.
- `POST …/unpublish` gỡ mục (về lại bản nháp, địa chỉ trả 404); `POST …/archive` gỡ hẳn (địa
  chỉ trả 410); `DELETE` xoá mục (mục đang công khai được gỡ trước, địa chỉ trả 410).
- Không đăng được trang trước trang cha của nó; không gỡ hay xoá được trang còn trang con đang
  công khai.

## Lịch sử và hẹn giờ

- Một **revision** là bản chép đầy đủ các trường sửa được. Máy chủ giữ một bản khi tạo, mỗi lần
  đăng và gỡ, khi khôi phục, khi được yêu cầu (`"save_revision": true`), và tự động khi bản gần
  nhất đã cũ hơn 10 phút. `GET …/revisions` liệt kê,
  `POST …/revisions/{revision_id}/restore` chép một bản về bản nháp (không tự đăng). Mỗi mục giữ
  `AUTOXIDE_CONTENT_MAX_REVISIONS` (mặc định 50) bản mới nhất, cộng bản đang công khai và bản
  của các lịch đang chờ; một job chạy mỗi ngày dọn phần còn lại.
- **Hẹn giờ**: `POST …/schedules` với `{"action": "publish" | "unpublish", "run_at": …}`. Lịch
  đăng ghim bản nháp lúc đặt thành một revision: sửa sau đó không bị lịch này đăng. Nếu mục đã
  được đăng tay sau khi đặt lịch, lịch thất bại (`content.schedule_outdated`) thay vì ghi đè.
  Lịch thất bại giữ mã lỗi (`status: failed`) và phát event `content.schedule_failed`.

## Trang và địa chỉ

Máy chủ tính địa chỉ (`path`) của mọi mục đang công khai:

- bài viết: `/<AUTOXIDE_CONTENT_POST_PATH_PREFIX>/<slug>` (mặc định `/blog/hello`);
- trang: slug của các trang cha, rồi slug của chính nó (`/about/team`); tối đa 5 cấp.

Website chỉ ghép tên miền và, nếu dùng, tiền tố ngôn ngữ (`/vi/…`).

- `POST …/move` chuyển một trang sang trang cha khác (hoặc lên gốc). Có hiệu lực ngay, cho bản
  nháp lẫn bản công khai, và cho mọi trang con. Không chuyển được một trang vào dưới chính nó
  hay trang con của nó (`content.tree_cycle`).
- **Khi địa chỉ công khai đổi** (đổi slug, chuyển trang), máy chủ ghi chuyển hướng `301` từ địa
  chỉ cũ tới mục — cho mục đó và mọi trang con — nên link cũ vẫn chạy. Lưu trữ hay xoá thì ghi
  `410 Gone`.
- Chuyển hướng cũng làm tay được (`/api/v1/content/redirects`, permission `content.manage`):
  `301`, `302`, `307`, `308` tới một mục hay một địa chỉ, hoặc `410`. Vòng lặp bị từ chối
  (`content.redirect_loop`).

## Ngôn ngữ

Ngôn ngữ đặt bằng `AUTOXIDE_CONTENT_LOCALES` (mặc định `en,vi`, mã BCP 47) và
`AUTOXIDE_CONTENT_DEFAULT_LOCALE` (mặc định `en`).

- Mỗi bản dịch là một mục riêng, có slug, trạng thái và lịch sử riêng, nối với nhau bằng
  `translation_group_id`. Tạo bằng `POST …/entries/{id}/translations`
  (`{"locale": "vi", "title": …}`). Trang con chỉ dịch được khi trang cha đã có bản dịch.
- **Bài và trang không bao giờ lấy ngôn ngữ khác thay thế**: thiếu bản dịch là `404`, để
  canonical và `hreflang` luôn đúng. Mục công khai liệt kê các bản dịch của nó (`translations`,
  dùng cho `hreflang`).
- Tên danh mục, thẻ và menu thì lấy ngôn ngữ mặc định khi thiếu; câu trả lời ghi ngôn ngữ thật
  đã dùng (`locale`).

## Danh mục, thẻ, tác giả, menu

Permission `content.manage`.

- **Term** (`/api/v1/content/terms`): `category` (có thể có cha) hoặc `tag`, có tên, slug, mô tả
  và chữ SEO theo từng ngôn ngữ. Gắn vào mục bằng `PUT /api/v1/content/entries/{id}/terms`
  (`{"term_ids": [...]}`, danh mục đầu tiên là danh mục chính). Có hiệu lực ngay, cả với bản
  công khai — nên với mục đang công khai cần thêm `content.publish` ngoài `content.write` (gắn
  tác giả cũng vậy).
- **Tác giả** (`/api/v1/content/authors`): hồ sơ của module content — tên hiển thị, slug, giới
  thiệu, ảnh đại diện (ID media), website — có thể gắn với một tài khoản (`user_id`). Một mục có
  thể có nhiều tác giả, theo thứ tự: `PUT /api/v1/content/entries/{id}/authors`.
- **Menu** (`/api/v1/content/menus/{key}?locale=`): một cây mục (tối đa 4 cấp, 200 mục), mỗi
  mục trỏ tới một bài / trang, một term, hoặc một địa chỉ. `PUT` thay cả cây (`If-Match: *` để
  tạo mới).

## Website: endpoint công khai

Không cần đăng nhập. Chỉ trả bản đã đăng — không bao giờ trả bản nháp.

| Endpoint                                              | Trả                                                          |
| ----------------------------------------------------- | ------------------------------------------------------------ |
| `GET /public/resolve?locale=&path=`                   | Thứ ở một địa chỉ: `entry`, `redirect` hoặc `gone`           |
| `GET /public/posts?locale=&term_id=&author_id=&q=`    | Bài đã đăng, mới nhất trước, có pagination                    |
| `GET /public/menus/{key}?locale=`                     | Menu kèm `href` đã tính; mục trỏ tới bài chưa đăng bị ẩn     |
| `GET /public/terms/{taxonomy}/{slug}?locale=`         | Trang của một danh mục hay thẻ                               |
| `GET /public/authors/{slug}`                          | Trang của một tác giả                                        |
| `GET /public/sitemap?locale=`                         | Mọi địa chỉ đang công khai, kèm bản dịch cho `hreflang`, có pagination |
| `GET /public/feed?locale=&limit=`                     | Các bài mới nhất kèm thân bài, cho RSS, Atom hay JSON Feed   |

(Tất cả dưới `/api/v1/content`.)

**Hiển thị một địa chỉ.** Với mỗi request, website gọi `resolve` với ngôn ngữ và đường dẫn, rồi
làm theo `type`:

- `entry` → hiển thị `data.entry` (tiêu đề, JSON thân bài, ảnh kèm URL, SEO, term, tác giả, bản
  dịch);
- `redirect` → trả `data.status` với `Location: data.location`;
- `gone` → trả `410`;
- nhận `404` → trả `404`.

**Cache.** Câu trả lời công khai có `Cache-Control: public, no-cache` và, khi hợp lý, `ETag`:
cache được nhưng phải hỏi lại; gửi lại `ETag` trong `If-None-Match` nhận về `304 Not Modified`
rất rẻ.

**Làm mới cache khi nội dung đổi.** Đăng ký webhook (`POST /api/v1/webhooks`, xem phần webhook
trong [README](../README.vi.md)) cho `content.entry_published`, `content.entry_unpublished`,
`content.entry_archived`, `content.entry_deleted` và `content.entry_moved`. Trường
`data.payload.published` cho biết cần xoá cache gì:

```json
{
  "entry_id": "…", "kind": "page", "locale": "en", "translation_group_id": "…",
  "path": "/company/team",
  "stale_paths": ["/about/team", "/about/team/jobs"],
  "moved_subpages": [{"entry_id": "…", "old_path": "/about/team/jobs", "path": "/company/team/jobs"}]
}
```

`path` là địa chỉ mới (`null` khi không còn công khai), `stale_paths` là mọi địa chỉ đã đổi, kể
cả của trang con. Menu, term và tác giả phát `content.menu_changed`, `content.term_changed` và
`content.author_changed`; các event này không kèm danh sách trang bị ảnh hưởng (một thẻ có thể
nằm trên hàng nghìn bài). Hãy gắn cho mỗi trang đã cache tag theo ID term và tác giả của nó (ví dụ
`term:<id>`, `author:<id>`) rồi làm mới các tag đó khi nhận event — đổi tên hay xoá thẻ, tác
giả (kể cả khi xoá tài khoản) sẽ làm mới mọi trang đang hiện nó.

**Sitemap và feed.** Dựng `sitemap.xml` từ `/public/sitemap` (đi theo `next_cursor`; bỏ mục có
`noindex: true`; `content_modified_at` là `lastmod`; `translations` cho các `hreflang`). Dựng
RSS, Atom hay JSON Feed từ `/public/feed`, dùng `id` của bài (không bao giờ đổi) làm ID của mục
trong feed.

## Xem trước bản nháp

Người biên tập có thể cho một người không có tài khoản xem bản nháp:

1. Đặt `AUTOXIDE_CONTENT_PREVIEW_SECRET` (chuỗi ngẫu nhiên ít nhất 32 ký tự). Không đặt thì tính
   năng tắt (`503`, mã `content.preview_disabled`).
2. `POST /api/v1/content/entries/{id}/preview-token` (permission `content.write`) trả một
   `token` đã ký và `expires_at` (`AUTOXIDE_CONTENT_PREVIEW_TTL_SECONDS`, mặc định 10 phút).
3. Trang xem trước của website gọi `GET /api/v1/content/public/preview?token=…` rồi hiển thị bản
   nháp. Câu trả lời có `Cache-Control: no-store`, luôn là bản nháp mới nhất. Token sai hay hết
   hạn trả `404` (`content.preview_token_invalid`). Đổi secret là thu hồi mọi token.

## AI agent (MCP)

Tool `content.list`, `content.get`, `content.create_draft`, `content.update`,
`content.publish`, `content.unpublish`, `content.list_revisions`, `content.restore_revision` và
`content.resolve`, cùng permission với endpoint REST. Kết quả có mục thì kèm `version` của mục;
tool thay đổi mục bắt buộc gửi lại giá trị đó trong tham số `version`, y như `If-Match`.

## Khi một tài khoản bị xoá

Bài và trang thuộc về site, không thuộc người viết: chúng được giữ. Hồ sơ tác giả gắn với tài
khoản bị xoá (và gỡ khỏi các mục), lịch người đó đặt bị huỷ, `created_by` thành `null`. Bản xuất
dữ liệu cá nhân liệt kê hồ sơ tác giả của người đó và các mục họ tạo.

## Cấu hình

| Biến                                   | Mặc định                                         |
| -------------------------------------- | ------------------------------------------------ |
| `AUTOXIDE_CONTENT_LOCALES`             | `en,vi`                                          |
| `AUTOXIDE_CONTENT_DEFAULT_LOCALE`      | `en`                                             |
| `AUTOXIDE_CONTENT_POST_PATH_PREFIX`    | `blog`                                           |
| `AUTOXIDE_CONTENT_MAX_REVISIONS`       | `50`                                             |
| `AUTOXIDE_CONTENT_MAX_BODY_BYTES`      | `524288`                                         |
| `AUTOXIDE_CONTENT_WORDS_PER_MINUTE`    | `238`                                            |
| `AUTOXIDE_CONTENT_EMBED_HOSTS`         | `youtube.com,youtu.be,youtube-nocookie.com,vimeo.com` |
| `AUTOXIDE_CONTENT_PREVIEW_SECRET`      | không có (xem trước tắt)                         |
| `AUTOXIDE_CONTENT_PREVIEW_TTL_SECONDS` | `600`                                            |

Mọi biến được mô tả trong [`.env.example`](../.env.example).
