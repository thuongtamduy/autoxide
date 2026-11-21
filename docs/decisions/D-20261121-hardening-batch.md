## D-20261121-hardening-batch · Cứng hoá kernel và hạ tầng: step-up cho tool, quyền event của webhook, mẫu, SVG, ảnh, trang tài liệu — 2026-11-21 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định:

1. **MCP / tự động hoá:** `Tool::authorize` (gọi trong `call_tool` trước khi chạy) kiểm như `Principal::require`, kể cả
   xác nhận lại (step-up) cho permission `destructive` / `money`; `Tool::allows` chỉ còn để lọc danh sách tool.
2. **Webhook:** tạo / sửa endpoint đòi `stream_permission` của từng event đăng ký (`*`: mọi event có permission đó),
   như luồng event SSE / WebSocket.
3. **Mẫu của khách:** khối `{% autoescape … %}` bị từ chối lúc kiểm và lúc dựng (`template.autoescape_forbidden`) —
   minijinja không cho bỏ thẻ này khỏi cú pháp.
4. **SVG tải lên:** ngoài các mẫu chữ cũ, chỉ nhận thẻ trong danh sách cho phép (hình, chữ, tô màu, bộ lọc, siêu dữ liệu
   Inkscape / RDF; không `a`, `set`, `animate*`), chặn `<!DOCTYPE` / `<!ENTITY` / `<![CDATA[`, chỉ thị xử lý khác
   `<?xml …?>` và namespace XHTML.
5. **Ảnh:** `AUTOXIDE_MEDIA_IMAGE_JOBS` (mặc định 2) giới hạn số ảnh giải mã cùng lúc (ảnh thu nhỏ, AVIF, xem trước
   cho AI).
6. **Trang tài liệu API:** script Scalar ghim phiên bản với Subresource Integrity (`sha384`); bản chạy nhanh tắt trang
   này mặc định.
7. **Cấu hình:** `AUTOXIDE_SECRET_KEY` mang giá trị mẫu (`change-me…`) bị từ chối lúc khởi động; `compose.yml` (bản chạy
   thật) bắt buộc có biến này. `AUTOXIDE_HTTP_CLIENT_IP_HEADER` đặt mà thiếu `AUTOXIDE_HTTP_TRUSTED_PROXIES` thì cảnh báo
   lúc khởi động.
8. **`LIKE`:** mẫu dựng từ dữ liệu người dùng đi qua `autoxide_kernel::search::{starts_with, contains, escape_like}`
   (thoát `%`, `_`, `\`); không dùng `ColumnTrait::starts_with` / `contains` của SeaORM với chuỗi người dùng.

Lý do: các điểm do rà soát bảo mật nội bộ tìm ra; mỗi điểm sửa ở một chỗ dùng chung để module sau không lặp lỗi.

Đã cân nhắc: phục vụ script Scalar từ binary (thêm ~5 MB vào binary); cấm SVG hẳn (nhiều website cần logo SVG); một
lint xtask cấm `starts_with` của SeaORM (khó phân biệt hằng số với chuỗi người dùng; để sau).
