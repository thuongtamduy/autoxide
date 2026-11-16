# Thư viện media

*English: [MEDIA.md](MEDIA.md)*

Module `media` giữ ảnh, video, âm thanh và tài liệu của một website hay ứng dụng trong
thư mục lồng nhau, và phục vụ chúng — công khai, hoặc cho người đã đăng nhập. Module nằm
dưới `/api/v1/media`. Hợp đồng đầy đủ, với mọi tham số và mã lỗi, ở
[`openapi.json`](openapi.json) (và ở `/api/docs` khi `AUTOXIDE_HTTP_API_DOCS=true`).

Nội dung lưu ở nơi module `files` lưu file (`AUTOXIDE_FILES_DIR` trên ổ đĩa, hoặc một
bucket S3), dưới các khoá bắt đầu bằng `media/`. Thông tin (tên, loại, kích thước, kích
thước ảnh, chữ mô tả, thư mục) nằm trong database. `autoxide backup` và `autoxide restore`
mang theo cả hai.

## Permission

| Permission    | Cho phép                                                                  |
| ------------- | ------------------------------------------------------------------------- |
| `media.read`  | Liệt kê và đọc item (kể cả item riêng tư), thư mục và thùng rác           |
| `media.write` | Tải lên, sửa, thay nội dung, chuyển, công khai, cho vào thùng rác; thư mục |
| `media.purge` | Xoá hẳn item trong thùng rác                                              |

Gán cho một role bằng `PUT /api/v1/auth/roles/{id}`. Role `admin` có đủ.

## Cửa hàng và người bán

Ở khách có cây tổ chức (module `organizations`), mỗi item thuộc một nút: `org_id` khi tải lên (bỏ
trống: nút duy nhất của bạn, hay nút gốc khi role của bạn phủ cả khách). Permission áp theo độ sâu
của role — người bán có `media.write` ở gian hàng của mình (`subtree`) chỉ thấy và sửa item của
gian hàng đó; `own` là item bạn tải lên. Item ngoài phạm vi trả 404, kể cả link tải riêng tư. Lọc
danh sách bằng `owner_org`. Thư mục và nhãn dùng chung cả khách.

## Tải lên

Gửi file thô làm thân request (không phải form multipart), loại của nó trong
`Content-Type` và tên trong `name`:

```bash
curl -X POST "http://localhost:8080/api/v1/media?name=team.jpg" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: image/jpeg" \
  -H "Idempotency-Key: $(uuidgen)" \
  --data-binary @team.jpg
```

- Máy chủ kiểm nội dung có thật đúng loại đã khai không, theo các byte đầu file. Trang
  HTML gửi dưới tên `image/png` bị từ chối (`415`, mã `media.content_mismatch`). Không gửi
  `Content-Type` (hay gửi `application/octet-stream`) thì máy chủ tự nhận loại.
- Loại được nhận đặt bằng `AUTOXIDE_MEDIA_ALLOWED_TYPES` (mặc định: JPEG, PNG, GIF, WebP,
  AVIF, SVG, MP4, WebM, mọi loại âm thanh, PDF); kích thước tối đa bằng
  `AUTOXIDE_MEDIA_MAX_BYTES` (mặc định 100 MiB).
- Ảnh raster (JPEG, PNG, GIF, WebP, AVIF) có `width` và `height`. Ảnh JPEG, PNG, GIF và
  WebP còn có ảnh thu nhỏ (xem bên dưới).
- Kết quả (`201`, kèm `Location`) có `duplicates`: các item đã có trong thư viện với nội
  dung y hệt (cùng SHA-256). Bản vừa tải lên vẫn được giữ.
- `folder_id=<id>` đặt item vào một thư mục, `tags=brand,logo` gắn nhãn. Item mới là
  `private`.

## Đọc, liệt kê, tìm

- `GET /api/v1/media/{id}` — một item, kèm `ETag`.
- `GET /api/v1/media` — một trang item. Bộ lọc: `folder_id` (`root` hay một ID; chỉ thư
  mục đó, không gồm thư mục con), `type` (`image/png` hay `image/*`), `visibility`
  (`public` / `private`), `trashed=true` (thùng rác), `q` (tên chứa chuỗi, không phân biệt
  hoa thường), `tag` (mang nhãn này, không phân biệt hoa thường), `alt_missing=true` (ảnh
  chưa có alt và không đánh dấu `decorative`). Thứ tự:
  `sort=created_at|name|size` với `order=asc|desc`. Lật trang theo
  `pagination.next_cursor`, giữ nguyên bộ lọc. `include=uploader` thêm `uploaded_by_name`, tên
  người tải lên từng item.
- `GET /api/v1/media/summary` — số liệu cả thư viện: `items`, `bytes`, `images_missing_alt`,
  `trashed_items`.

## Sửa

Lệnh sửa cần `ETag` đã đọc, gửi trong `If-Match` (`*` để sửa bất kể phiên bản). Không gửi
thì nhận `428` (mã `precondition_required`); ai đó vừa sửa trước thì `412` (mã
`precondition_failed`) — đọc lại rồi thử lại.

- `PATCH /api/v1/media/{id}` — `name`, `alt`, `title`, `caption`, `decorative` (`true` cho ảnh
  chỉ để trang trí: alt để trống có chủ ý), `folder_id` (`null` là
  thư mục gốc), `visibility`, `tags` (thay cả bộ nhãn; `[]` gỡ hết).
- `PUT /api/v1/media/{id}/content` — thay nội dung, GIỮ ID (mọi chỗ đang dùng item vẫn
  đúng). Kiểm như khi tải lên. `url`, `version_id` và `ETag` đổi; nội dung cũ bị xoá.
- `PUT /api/v1/media/{id}/alts/{locale}` với `{"alt": "…"}` — alt bằng một ngôn ngữ (vd `vi` trên
  website hai ngôn ngữ); `DELETE /api/v1/media/{id}/alts/{locale}` gỡ nó. `alt` của item vẫn là bản
  mặc định cho mọi ngôn ngữ chưa có bản dịch; item trả mọi bản dịch trong `alts`. Ngôn ngữ được
  phép: `AUTOXIDE_MEDIA_ALT_LOCALES` (mặc định `en,vi`), hoặc cấu hình `media.alt_locales` riêng
  của khách khi khách đã đặt (quản trị của khách gọi `PUT /api/v1/kernel/settings/media.alt_locales`,
  nhân sự nền tảng gọi `PUT /api/v1/kernel/tenants/{id}/settings/media.alt_locales`; `alt_locales`
  trong `GET /api/v1/kernel/client-config` cho thấy danh sách của khách); ngôn ngữ khác bị từ chối
  với `media.unsupported_locale`. Xoá hẳn item thì xoá luôn bản dịch.

## Phục vụ nội dung

Mỗi item có `url`: `/api/v1/media/{id}/content?v=<version_id>`.

- **Ai xem được**: item `visibility: public` (và không ở thùng rác) tải được không cần
  đăng nhập — dùng thẳng `url` trong `<img src>` hay `<video src>`. Item khác cần
  `media.read`; không có thì nhận `404`, như thể item không tồn tại.
- **Cache**: kèm đúng `v` hiện tại thì được cache một năm (`immutable`): thay nội dung là
  `v` đổi, tức địa chỉ đổi. Không kèm `v`, hay `v` cũ, thì cache một phút (công khai) hoặc
  hỏi lại máy chủ mỗi lần (riêng tư). Chuyển item sang riêng tư không xoá được bản đã nằm
  trong cache (trình duyệt, CDN); cần vậy thì thay nội dung của nó.
- **An toàn**: chỉ ảnh raster được mở ngay trong trình duyệt (`Content-Disposition:
  inline`); mọi loại khác, kể cả SVG và PDF, là tải về (`attachment`). Mọi response có
  `X-Content-Type-Options: nosniff` và `Content-Security-Policy: default-src 'none';
  sandbox`, nên nội dung không chạy được script trên tên miền của bạn. Thêm nữa, SVG chứa
  bất cứ thứ gì chạy được (`<script>`, thuộc tính `on…=`, liên kết `javascript:`, HTML nhúng),
  ở bất cứ đâu trong file, bị từ chối khi tải lên (`415 media.svg_active_content`).
- **Metadata**: file gốc được trả đúng như lúc tải lên, kể cả metadata — EXIF của ảnh chụp
  có thể chứa nơi chụp và số máy. Cần thì xoá trước khi tải lên; các bản thu nhỏ bên dưới
  không mang metadata nào.
- **Video và file lớn**: một `Range: bytes=…` được trả `206` (tua video được); khoảng nằm
  ngoài nội dung thì `416`. `HEAD` chỉ trả header. `If-None-Match` với `ETag` thì `304`.

## Ảnh thu nhỏ

Vài giây sau khi tải lên (hay thay nội dung), một job nền làm các bản nhỏ hơn của mọi ảnh
JPEG, PNG, GIF và WebP, theo các chiều rộng trong `AUTOXIDE_MEDIA_IMAGE_WIDTHS` (mặc định
`320,640,1280,1920`; chỉ những cỡ nhỏ hơn ảnh; `off` để tắt). Ảnh được xoay đúng chiều (theo
EXIF), giữ tỉ lệ, lưu dạng JPEG — hoặc PNG khi ảnh có phần trong suốt. Item liệt kê chúng
trong `variants`, mỗi bản có `url` riêng (`url` của item thêm `&w=<rộng>`), dùng ngay cho
`srcset`:

```html
<img src="/api/v1/media/…/content?v=…&w=640"
     srcset="…&w=320 320w, …&w=640 640w, …&w=1280 1280w" sizes="100vw" alt="…">
```

Khi bản thu nhỏ chưa làm xong, xin nó thì nhận bản gốc, cache ngắn. Ảnh hơn 40 triệu pixel,
ảnh AVIF và SVG không có bản thu nhỏ. Thay nội dung thì làm bản mới và xoá bản cũ. Bản thu
nhỏ theo chế độ công khai và thùng rác của item.

Đổi `AUTOXIDE_MEDIA_IMAGE_WIDTHS` chỉ áp cho ảnh tải lên sau đó. Để ảnh đã tải lên trước cũng
có đúng các cỡ mới, chạy lệnh này một lần sau khi khởi động lại với cấu hình mới; worker làm
bản thu nhỏ ở nền (`dry_run=true` thì chỉ đếm):

```bash
autoxide task media.refresh_variants
```

### Bản WebP

Bên cạnh mỗi bản JPEG hay PNG, job còn làm một bản WebP cùng cỡ, thường nhẹ hơn nhiều: với
một ảnh chụp thử, nhẹ hơn bản JPEG khoảng 55–60%, và với ảnh có phần trong suốt thì nhẹ hơn
bản PNG trên 90% (3,3 MB xuống 80 KB ở 1920 px). Mỗi mục của `variants` cho địa chỉ của nó ở
`webp_url` (`url` của bản đó thêm `&format=webp`) và dung lượng ở `webp_size`. Để trình duyệt
tự chọn: trình duyệt đọc được WebP lấy `<source>`, trình duyệt khác lấy `<img>`:

```html
<picture>
  <source type="image/webp"
          srcset="…&w=320&format=webp 320w, …&w=640&format=webp 640w" sizes="100vw">
  <img src="…&w=640" srcset="…&w=320 320w, …&w=640 640w" sizes="100vw" alt="…">
</picture>
```

Bản công khai của module nội dung (`featured_media`, `body_media`) mang cùng `webp_url`.
`webp_url` là `null` khi bản WebP chưa làm xong; xin địa chỉ có `&format=webp` trước lúc đó thì
nhận bản JPEG hay PNG, cache ngắn. Ảnh có bản thu nhỏ làm từ trước khi có bản WebP nhận chúng
bằng đúng lệnh trên: `autoxide task media.refresh_variants` (lệnh cũng xếp những ảnh chỉ thiếu
bản WebP). Làm bản WebP tốn thời gian của job nền gấp khoảng ba lần bản JPEG (thêm chừng một
phần tư giây cho mỗi ảnh chụp với các cỡ mặc định); request không bị chậm đi.

## Nhãn

Ngoài một thư mục, item mang được tối đa 20 nhãn (mỗi nhãn 1 tới 50 ký tự, không có `,`).
Nhãn được tạo khi lần đầu gắn cho một item. `/api/v1/media/tags` liệt kê nhãn, kèm số item
(ngoài thùng rác) mang mỗi nhãn, đổi tên (`PATCH`, kèm `If-Match`) hay xoá nhãn (`DELETE`,
gỡ khỏi mọi item). Tên nhãn không trùng, không phân biệt hoa thường.

## Thư mục

`/api/v1/media/folders`: tạo (`POST`, `name` và `parent_id`), liệt kê (`GET`, lọc
`parent_id=root|<id>`, `sort=created_at|name`), đọc, đổi tên hay chuyển (`PATCH`, kèm
`If-Match`), xoá (`DELETE`). Mỗi thư mục có `item_count`: số item nằm thẳng trong nó, không
tính thùng rác. Thư mục lồng nhau được. Tên không trùng trong cùng thư mục
cha, không phân biệt hoa thường. Không chuyển được thư mục vào chính nó hay thư mục con
của nó (`409`, `media.folder_cycle`). Chỉ xoá được thư mục trống — item trong thùng rác
cũng tính (`409`, `media.folder_not_empty`).

## Thùng rác

- `DELETE /api/v1/media/{id}` cho item vào thùng rác: link công khai thôi chạy, người có
  `media.read` vẫn thấy (`GET /api/v1/media?trashed=true`).
- `POST /api/v1/media/{id}/restore` lấy ra.
- `POST /api/v1/media/{id}/purge` (permission `media.purge`) xoá hẳn.
- Item nằm trong thùng rác `AUTOXIDE_MEDIA_TRASH_DAYS` ngày (mặc định 30), rồi một job
  hằng ngày xoá hẳn. Một job hằng ngày khác xoá nội dung không còn item nào trỏ tới (vd do
  tải lên bị ngắt), chỉ khi nó cũ hơn một ngày.

## Gợi ý alt text

`POST /api/v1/media/{id}/alt-suggestions` với `{"locale": "vi", "context": "…"}` (permission
`media.write`) nhờ mô hình AI của máy chủ gợi ý alt text cho một ảnh JPEG, PNG, GIF hay WebP
bằng ngôn ngữ đó. Gửi một bản nhỏ (tối đa 1280 px), làm lại không kèm metadata kể cả khi ảnh
gốc đã nhỏ, không bao giờ gửi file gốc; ảnh riêng tư chỉ
gửi khi `AUTOXIDE_MEDIA_AI_SEND_PRIVATE=true`. Không lưu gì: người xem rồi lưu bằng `PATCH` (ngôn
ngữ mặc định) hay `PUT /api/v1/media/{id}/alts/{locale}` (ngôn ngữ khác). `decorative: true` nghĩa là
ảnh không thêm thông tin — để alt trống. Thân bài cũng có thể đặt alt riêng trên node ảnh của nó, và
alt đó được ưu tiên hơn alt của item. Bật AI: [`AI.vi.md`](AI.vi.md).

## AI agent (MCP)

Tool `media.list`, `media.get`, `media.update`, `media.set_alt`, `media.trash`,
`media.list_tags`, `media.upload` và `media.suggest_alt`, cùng permission với endpoint REST.
`media.update` và `media.set_alt` nhận `updated_at` đã đọc thay cho `If-Match`. `media.upload` nhận nội dung dạng base64, tối đa 1 MiB sau khi giải mã —
nhưng cả lời gọi tool còn phải lọt `AUTOXIDE_HTTP_BODY_LIMIT_BYTES` (mặc định 1 MiB), mà
base64 dài hơn một phần ba, nên với giới hạn mặc định chỉ file tới khoảng 750 KiB là vừa; file
lớn hơn thì tải lên qua REST.

`media.write` cho người giữ nó thấy mọi item nó thay đổi — ví dụ bằng cách đổi item riêng tư
thành công khai — nên chỉ cấp cho người cũng được xem thư viện.

## Cho người viết module

Module khác đọc media qua trait `autoxide_media::api::Media` (đăng ký sau `media`):
`find_many(&ids)` trả về, trong hai câu query cho bao nhiêu ID cũng được, ID, visibility,
loại, kích thước ảnh, `alt`, `url` và ảnh thu nhỏ của mỗi item không ở thùng rác.
`find_many_in(&ids, locale)` làm như vậy, với `alt` bằng ngôn ngữ đó khi item có bản dịch (thêm một
câu query), không thì bản mặc định. Bảng của module mình chỉ giữ ID của item.

## Khi một tài khoản bị xoá

Item thuộc về thư viện, không thuộc người tải lên: item vẫn còn, `uploaded_by` thành
`null`. Bản xuất dữ liệu cá nhân liệt kê các item người đó đã tải lên.
