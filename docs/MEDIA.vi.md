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
- Ảnh raster (JPEG, PNG, GIF, WebP, AVIF) có `width` và `height`. Máy chủ chỉ giữ bản
  gốc, không tự tạo ảnh thu nhỏ hay cỡ khác. Đổi cỡ ở website (vd component ảnh của
  framework) hoặc ở lớp đứng trước máy chủ.
- Kết quả (`201`, kèm `Location`) có `duplicates`: các item đã có trong thư viện với nội
  dung y hệt (cùng SHA-256). Bản vừa tải lên vẫn được giữ.
- `folder_id=<id>` đặt item vào một thư mục. Item mới là `private`.

## Đọc, liệt kê, tìm

- `GET /api/v1/media/{id}` — một item, kèm `ETag`.
- `GET /api/v1/media` — một trang item. Bộ lọc: `folder_id` (`root` hay một ID; chỉ thư
  mục đó, không gồm thư mục con), `type` (`image/png` hay `image/*`), `visibility`
  (`public` / `private`), `trashed=true` (thùng rác), `q` (tên chứa chuỗi, không phân biệt
  hoa thường). Thứ tự: `sort=created_at|name|size` với `order=asc|desc`. Lật trang theo
  `pagination.next_cursor`, giữ nguyên bộ lọc.

## Sửa

Lệnh sửa cần `ETag` đã đọc, gửi trong `If-Match` (`*` để sửa bất kể phiên bản). Không gửi
thì nhận `428` (mã `precondition_required`); ai đó vừa sửa trước thì `412` (mã
`precondition_failed`) — đọc lại rồi thử lại.

- `PATCH /api/v1/media/{id}` — `name`, `alt`, `title`, `caption`, `folder_id` (`null` là
  thư mục gốc), `visibility`.
- `PUT /api/v1/media/{id}/content` — thay nội dung, GIỮ ID (mọi chỗ đang dùng item vẫn
  đúng). Kiểm như khi tải lên. `url`, `version_id` và `ETag` đổi; nội dung cũ bị xoá.

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
  sandbox`, nên nội dung không chạy được script trên tên miền của bạn.
- **Video và file lớn**: một `Range: bytes=…` được trả `206` (tua video được); khoảng nằm
  ngoài nội dung thì `416`. `HEAD` chỉ trả header. `If-None-Match` với `ETag` thì `304`.

## Thư mục

`/api/v1/media/folders`: tạo (`POST`, `name` và `parent_id`), liệt kê (`GET`, lọc
`parent_id=root|<id>`, `sort=created_at|name`), đọc, đổi tên hay chuyển (`PATCH`, kèm
`If-Match`), xoá (`DELETE`). Thư mục lồng nhau được. Tên không trùng trong cùng thư mục
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

## AI agent (MCP)

Tool `media.list`, `media.get`, `media.update` và `media.trash`, cùng permission với
endpoint REST. `media.update` nhận `updated_at` đã đọc thay cho `If-Match`. Tải lên đi
qua REST: nội dung nhị phân không hợp với lời gọi tool.

## Cho người viết module

Module khác đọc media qua trait `autoxide_media::api::Media` (đăng ký sau `media`):
`find_many(&ids)` trả về, trong một câu query, ID, visibility, loại, kích thước ảnh, `alt`
và `url` của mỗi item không ở thùng rác. Bảng của module mình chỉ giữ ID của item.

## Khi một tài khoản bị xoá

Item thuộc về thư viện, không thuộc người tải lên: item vẫn còn, `uploaded_by` thành
`null`. Bản xuất dữ liệu cá nhân liệt kê các item người đó đã tải lên.
