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

| Permission            | Cho phép                                                                     |
| --------------------- | ---------------------------------------------------------------------------- |
| `content.read`        | Đọc bản nháp, revision và lịch                                               |
| `content.write`       | Tạo và sửa bản nháp, bản dịch, khôi phục revision, tạo link xem trước, xin duyệt |
| `content.write_own`   | Như `content.write`, nhưng chỉ trên mục do chính bên gọi tạo (cùng người dùng **và** cùng API key) |
| `content.publish`     | Đăng, gỡ, lưu trữ, chuyển trang, hẹn giờ; danh mục và tác giả của mục đang công khai |
| `content.publish_own` | Chỉ đăng và gỡ mục do chính bên gọi tạo; không lưu trữ, hẹn giờ hay chuyển trang đang công khai |
| `content.delete`      | Xoá hẳn mục                                                                  |
| `content.manage`      | Danh mục, thẻ, tác giả, menu, chuyển hướng                                   |

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
  Lịch thất bại giữ mã lỗi (`status: failed`) và phát event `content.schedule_failed`. Gỡ mục
  thì huỷ lịch đăng đang chờ của nó, lưu trữ thì huỷ mọi lịch đang chờ của mục (mỗi lịch phát
  `content.schedule_canceled`): lịch đặt từ trước không bao giờ đăng lại mục đã bị gỡ.

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
  (`content.redirect_loop`). Địa chỉ trong site bắt đầu bằng đúng một `/`: `//host` (trình duyệt
  hiểu là site khác) bị từ chối, ở đây và trong menu.

## Ngôn ngữ

Ngôn ngữ đặt bằng `AUTOXIDE_CONTENT_LOCALES` (mặc định `en,vi`, mã BCP 47) và
`AUTOXIDE_CONTENT_DEFAULT_LOCALE` (mặc định `en`).

Khi có nhiều khách hàng (tenant), hai biến này là mặc định của mọi khách: mỗi khách có thể đặt
`content.locales` và `content.default_locale` của mình (quản trị của khách gọi
`PUT /api/v1/kernel/settings/{name}`, permission `kernel.settings.manage`; nhân sự nền tảng gọi
`PUT /api/v1/kernel/tenants/{id}/settings/{name}`). Ngôn ngữ của khách áp cho mọi thứ của khách đó:
mục, term, menu, chuyển hướng nhận ngôn ngữ nào, `locale` của route công khai, và phần `content` của
`GET /api/v1/kernel/client-config` cho request nêu khách. Ngôn ngữ mặc định của khách không nằm
trong danh sách ngôn ngữ của khách thì dùng mặc định của máy chủ nếu nó nằm trong danh sách, không
thì ngôn ngữ đầu tiên. Mục đã viết bằng ngôn ngữ mà khách bỏ đi vẫn giữ nguyên. Mã sai bị từ chối
ngay lúc đặt; gọi `DELETE` cấu hình đó để về giá trị của máy chủ.

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
  thể có nhiều tác giả, theo thứ tự: `PUT /api/v1/content/entries/{id}/authors`. Gắn term hay
  tác giả cho mục cần quyền sửa mục đó, không cần `content.read`.
- **Menu** (`/api/v1/content/menus/{key}?locale=`): một cây mục (tối đa 4 cấp, 200 mục), mỗi
  mục trỏ tới một bài / trang, một term, hoặc một địa chỉ. `PUT` thay cả cây (`If-Match: *` để
  tạo mới). **Đã deprecated** (sunset 14/05/2027): menu của website chuyển sang module `menus`, mục
  trỏ được tới đối tượng của module bất kỳ; chép menu sang bằng
  `autoxide task menus.import_content_menus` ([`MENUS.vi.md`](MENUS.vi.md)).

## Cửa hàng và website

Ở khách có cây tổ chức (module `organizations`), mỗi mục thuộc một nút: `org_id` khi tạo (bỏ
trống: nút duy nhất của bạn, hay nút gốc khi role của bạn phủ cả khách). Các permission trên áp
theo độ sâu của role: người có `content.write` ở một cửa hàng (`subtree`) chỉ viết mục của cửa hàng
đó và các nút bên dưới; mục ngoài phạm vi trả 404. Lọc danh sách quản trị bằng `owner_org` (một
nút) hay `site` (những gì một website hiện).

Mỗi nút có thể có website: tên miền gắn với nút (`PATCH /api/v1/kernel/domains/{id}` với
`org_id`), hay `?site=<id của nút>` ở các endpoint công khai. Website hiện mục của nút đó và của mọi
nút cấp trên — bài của trụ sở hiện trên website của mọi cửa hàng, bài của một cửa hàng không bao
giờ hiện trên website của cửa hàng khác. Đường dẫn vẫn duy nhất trong cả khách.

## Website: endpoint công khai

Không cần đăng nhập. Chỉ trả bản đã đăng — không bao giờ trả bản nháp.

| Endpoint                                              | Trả                                                          |
| ----------------------------------------------------- | ------------------------------------------------------------ |
| `GET /public/resolve?locale=&path=`                   | Thứ ở một địa chỉ: `entry`, `redirect` hoặc `gone`           |
| `GET /public/posts?locale=&term_id=&author_id=&q=`    | Bài đã đăng, mới nhất trước, có pagination (`q`: tối đa 200 ký tự) |
| `GET /public/menus/{key}?locale=`                     | Menu kèm `href` đã tính; đã deprecated, dùng `GET /api/v1/menus/public/{key}` |
| `GET /public/terms/{taxonomy}/{slug}?locale=`         | Trang của một danh mục hay thẻ                               |
| `GET /public/authors/{slug}`                          | Trang của một tác giả                                        |
| `GET /public/sitemap?locale=`                         | Mọi địa chỉ đang công khai, kèm bản dịch cho `hreflang`, có pagination |
| `GET /public/feed?locale=&limit=`                     | Các bài mới nhất kèm thân bài, cho RSS, Atom hay JSON Feed   |

(Tất cả dưới `/api/v1/content`.)

**Nhiều site trên một máy chủ.** Máy chủ có nhiều khách thì mỗi khách là một site riêng: endpoint
công khai trả lời cho khách của request — tên miền riêng của khách, hay header `Autoxide-Tenant`
(tên rút gọn của khách). Hai khách được dùng cùng slug, cùng địa chỉ, cùng slug tác giả và term,
cùng menu và chuyển hướng; khách này không bao giờ thấy bài của khách kia. `ETag` của khách này
không bao giờ khớp ở khách khác.

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
   nháp. Câu trả lời có `Cache-Control: no-store`, luôn là bản nháp mới nhất, và chỉ có nội dung
   — không có ai tạo hay ai xin duyệt. Token sai hay hết hạn trả `404`
   (`content.preview_token_invalid`). Đổi secret là thu hồi mọi token.

Máy chủ có nhiều khách thì token chỉ mở bản nháp của khách đã tạo ra nó: token ghi khách của nó,
nên trang xem trước chạy được trên tên miền chính không cần header khách, còn cùng token gửi tới
khách khác (qua tên miền hay header) trả `404` như token sai.

## AI agent

**Công cụ AI** (hướng dẫn: [`AI.vi.md`](AI.vi.md)): `GET /entries/{id}/seo-check` (không AI),
`POST /entries/{id}/seo-suggestions`, `POST /entries/{id}/translate` (vào bản nháp của bản dịch,
không bao giờ tự đăng; theo dõi bằng `GET /translation-requests/{id}`), và `GET /search` /
`GET /public/search` (theo ý nghĩa và theo chữ). Mục do máy chủ dịch mang
`translated_from_revision_id`, và `translation_outdated` khi bản gốc đổi.

**Tool MCP.** `content.list`, `content.get`, `content.create_draft`, `content.update`,
`content.publish`, `content.unpublish`, `content.list_revisions`, `content.restore_revision`,
`content.request_review`, `content.seo_check`, `content.suggest_seo`, `content.translate`,
`content.get_translation_request`, `content.search` và `content.resolve`, cùng permission với
endpoint REST. Kết quả có
mục thì kèm `version` của mục; tool thay đổi mục bắt buộc gửi lại giá trị đó trong tham số
`version`, y như `If-Match`.

**Cấp cho agent một API key riêng** (`POST /api/v1/auth/api-keys`), `expires_at` ngắn và chỉ
những permission nó cần:

| Agent được…                            | Permission                                             |
| -------------------------------------- | ------------------------------------------------------ |
| viết nháp, người đăng                  | `content.read`, `content.write_own`                    |
| viết và tự đăng bài của mình           | `content.read`, `content.write_own`, `content.publish_own` |

Với các permission `_own`, agent chỉ đụng tới mục nó tạo bằng chính key đó: key khác — kể cả
của cùng người dùng — nhận `403 content.not_owner`. Agent không bao giờ lưu trữ, hẹn giờ hay xoá
được (xoá cần `content.delete`), cũng không chuyển được trang đang công khai; nó chuyển được trang
chưa đăng của mình, vào dưới trang do nó tạo. Thêm vài giới hạn để agent không đi vòng qua người
biên tập:

- `content.write_own` mà không có `content.write`: trang mới, trang được chuyển hay bản dịch của
  trang con chỉ nằm dưới trang cha do chính agent tạo (`403 content.not_owner`), để nó không
  chặn được việc gỡ hay xoá trang của người khác; mục luôn mang `ai_generated: true`, dù gửi gì;
  ở `PUT …/authors` chỉ thêm được hồ sơ tác giả gắn với chính tài khoản của nó (`user_id`) — hồ
  sơ đã có trên mục thì giữ được.
- `content.publish_own` mà không có `content.publish`: không đăng lại được mục người biên tập đã
  lưu trữ (`403 content.archived`); không tự đặt `published_at` (422, `not_allowed`; máy chủ
  dùng giờ hiện tại, hoặc giữ ngày của lần đăng trước); không đăng vào địa chỉ đang có chuyển
  hướng do người biên tập đặt (`409 content.path_has_redirect`), vì bản công khai sẽ che mất nó.

Mọi thay đổi nằm trong audit log kèm API key đã làm, và request bằng API key bị rate limiting
theo từng key (`AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE`, mặc định 120).

**Xin duyệt.** Viết xong, agent gọi `POST /entries/{id}/request-review` (tool
`content.request_review`): bản nháp có `review_requested_at` và event
`content.review_requested` được phát — đăng ký webhook để báo người biên tập.
`DELETE /entries/{id}/request-review` rút lại; đăng mục thì cờ tự xoá.

**Nội dung do AI tạo.** Gửi `"ai_generated": true` khi tạo hay sửa nội dung do AI viết (agent
nên luôn gửi; chỉ có `content.write_own` thì máy chủ tự đặt). Khi **một người** — đăng nhập, không phải API key — đăng mục đó sau khi đã duyệt,
họ gửi `{"ai_reviewed": true}` tới `POST /entries/{id}/publish`; bản công khai ghi ai duyệt và
lúc nào. Endpoint công khai trả `ai_generated` và `ai_reviewed`, để website hiện thông báo cho
người đọc khi luật yêu cầu (ví dụ Điều 50 của EU AI Act, áp dụng từ 2/8/2026; ngoại lệ "đã qua
người duyệt" được hiểu hẹp). Autoxide chỉ ghi lại sự thật, không đưa ra lời khuyên pháp lý. API
quản trị cũng cho biết mục được tạo thế nào: `created_via` là `session` hoặc `api_key`.

## Khi một tài khoản bị xoá

Bài và trang thuộc về site, không thuộc người viết: chúng được giữ. Hồ sơ tác giả gắn với tài
khoản bị xoá (và gỡ khỏi các mục), lịch người đó đặt bị huỷ, `created_by` thành `null`, cùng
`review_requested_by` (ai xin duyệt) và ghi nhận ai đã xác nhận duyệt nội dung AI — thời điểm xác
nhận vẫn giữ, nên người đọc vẫn thấy `ai_reviewed`. Bản xuất dữ liệu cá nhân liệt kê hồ sơ tác giả
của người đó và các mục họ tạo.

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
