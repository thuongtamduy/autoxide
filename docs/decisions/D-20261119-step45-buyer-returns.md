## D-20261119-step45-buyer-returns · Trả hàng phía người mua: ảnh kèm qua `Media::uploaders`, xem trước không ghi, thời gian hoàn là dữ liệu có ngày hiệu lực — 2026-11-19 — agent quyết

Chủ đề: Thương mại

Bối cảnh: mục 7 của bước 45 (D-201, D-202), nối tiếp D-20261119-step45-returns. Đặc tả 45 liệt kê mã lỗi riêng cho ảnh
(`orders.return_photos_required`, `orders.return_photo_not_owned`, `orders.return_too_many_photos`) nhưng cũng đòi lỗi ảnh
"gom chung một lần với lỗi khác của đầu vào" (D-083).

Quyết định:

1. **Lỗi ảnh là lỗi trường trong cùng 422 `validation_failed`** (D-083 thắng): `media_ids` mã `length` (quá 6),
   `photos_required` (lý do thuộc cài đặt kế thừa mới `orders.return_photo_reasons`, mặc định `damaged`, `wrong_item`),
   `media_ids[i]` mã `not_owned` (không do chính người mua tải lên, hay đang trong thùng rác). Không thêm ba mã lỗi cấp
   trên của đặc tả — client đọc `errors[].code`.
2. **Kiểm người tải lên qua `Media::uploaders`** (trait của `media`, có sẵn; không đọc bảng của `media`). `media` chưa
   cài: không ảnh nào hợp lệ. Endpoint tải ảnh của người mua là việc của `media`, không thuộc mục này.
3. **Cách gửi trả** (`pickup`, `drop_off` mặc định, `in_store`): `pickup` cần địa chỉ giao và phương thức giao của đơn;
   `in_store` chỉ khi đơn có cửa hàng nhận; chọn cách không dùng được → lỗi trường `return_method` mã `unavailable`.
   Phí lấy hàng = phương thức rẻ nhất của `ShippingRates::quote` (từ kho nhận về địa chỉ người mua — ước lượng chiều về
   bằng chiều đi), người mua trả khi không phải lỗi người bán, lỗi người bán thì 0. `drop_off`, `in_store`: 0 (cửa hàng
   không thu; cước bưu cục người mua trả trực tiếp). Duyệt yêu cầu `pickup` → `orders.return_pickup_requested` với
   `after.shipment` là `ShipmentRequest` chiều `return` (khoá `return:<id>`) cho `shipping`; kho nhận ghi vào yêu cầu và
   là kho mặc định khi nhận hàng.
4. **Xem trước** (`POST /me/orders/{id}/returns/preview`): cùng hàm kiểm với `POST …/returns` (cùng lỗi), cùng hàm tính
   tiền với lúc nhận hàng (`return_refund::refund_for`), không ghi, không event, không giữ chỗ. Không có tool MCP (việc
   của người mua).
5. **Thời gian hoàn dự kiến** là bảng dữ liệu có ngày hiệu lực theo nhóm cách trả (`gateway` 5–15 ngày về thẻ / ví đã
   trả; `transfer`, `cash` 1–3 ngày chuyển khoản) — ước lượng cho người mua, không phải cam kết của đối tác; đổi số thì
   thêm dòng có `effective_from` mới.

Hệ quả: `orders` phụ thuộc `autoxide-media` (chỉ `api`); cài đặt `orders.return_photo_reasons`.
