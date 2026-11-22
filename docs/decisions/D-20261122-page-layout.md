## D-20261122-page-layout · Thân `layout` của trang: cùng cột `body`, sửa từng phần bằng JSON Patch tự viết, khoá sửa tách mục riêng — 2026-11-22 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định (bước 67 mục 2, chi tiết hoá D-229):

1. Thân `layout` (`{"type": "layout", "schema": 1, "sections": [...]}`) nằm **trong cột `body` sẵn có**, phân biệt với
   thân `rich` (`{"type": "doc", …}`) bằng `type` ở gốc — không thêm cột `body_kind` như bảng dữ liệu phác thảo ở
   `docs/steps/67-page-builder.md`. Nháp, revision, khôi phục, đăng, hẹn giờ, link xem trước, bản dịch (mỗi bản dịch
   một bố cục) chép `body` nên dùng nguyên cơ chế sẵn có, không migration. Chỉ mục `kind = page` nhận thân `layout`.
2. Kiểm từ chối mặc định ở `content` (`entries/layout.rs`): khối phải nằm trong `Blocks::usable` của khách; props
   do kernel kiểm theo schema của khối (D-20261122-block-registry); `id` 1–64 ký tự `[A-Za-z0-9_-]` duy nhất trong
   trang; `visibility` chỉ nhận `base` / `md` / `lg` → bool; `children` chỉ ở khối khai `with_children`; tối đa 500
   khối, sâu 6, 256 KB. Lỗi gom một lần, tối đa 50, đường dẫn bắt đầu `body.` (D-083). Chữ trong props dạng chuỗi
   thường (không `format`, `enum`, `x-link`, `x-media`) vào `plain_text` để tìm kiếm và đếm chữ; ID ở `x-media`
   được kiểm tồn tại như ảnh của thân `rich`.
3. `PUT /entries/{id}/layout` thay cả; `PATCH /entries/{id}/layout` nhận JSON Patch RFC 6902 (mảng trần,
   `application/json-patch+json` hay `application/json`), tối đa 200 thao tác, nguyên tử, rồi kiểm lại cả trang. Cả
   hai buộc `If-Match` và đi qua đúng `update` của `content` (permission, revision nền, event
   `content.entry_updated`). Trang có thân `rich` trống được PATCH như bố cục trống; thân `rich` có nội dung → 409
   `content.not_layout`. JSON Patch tự viết (~150 dòng, sáu thao tác, JSON Pointer RFC 6901) thay vì thêm thư viện.
4. **Khoá sửa** (`POST/DELETE /entries/{id}/edit-lock`) tách thành mục riêng của bước 67 trong `docs/ROADMAP.md`: cần
   bảng mới (`content_edit_locks`) với phân loại khách, chính sách dời / sandbox và test đồng thời trên hai database —
   lớn hơn phần còn lại của mục; `If-Match` vẫn là chốt chống ghi đè, khoá chỉ để báo.
5. Hai khối nền tối thiểu `content.section` (chứa khối) và `content.heading` có từ mục này để dựng và kiểm bố cục;
   các khối nền còn lại vẫn thuộc mục "Khối nền" của bước 67.

Lý do: dùng lại cơ chế của `content` là yêu cầu của đặc tả; một cột `body` giữ một nguồn sự thật, không phải đồng
bộ hai cột trên bản nháp, revision và bản công khai.

Đã cân nhắc: cột `body_kind` (thêm migration cho hai bảng và mọi chỗ chép thân bài mà không thêm thông tin — `type`
ở gốc đã nói đủ); thư viện `json-patch` (thêm phụ thuộc cho ~150 dòng); làm khoá sửa ngay (kéo dài PR, mục chính
không cần nó để dùng được).
