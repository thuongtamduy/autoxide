## D-20261119-step45-pickup-preorder · Nhận tại cửa hàng và hàng đặt trước của `orders` — 2026-11-19 — agent quyết

Chủ đề: Thương mại

Bối cảnh: mục 5 của bước 45 (D-201, D-204, B-c7). Đặc tả 45 thêm cột nhận tại cửa hàng vào `orders_orders`, chính sách
`pickup_expired_action` vào `orders_policies` (đã thay bằng cài đặt kế thừa — D-20261118-step45-lifecycle).

Quyết định — nhận tại cửa hàng:

1. **Bảng riêng `orders_pickups`** (một dòng cho mỗi đơn đã sẵn sàng: cửa hàng giữ, trạng thái `ready` / `picked_up` /
   `expired`, mã nhận, `hold_until`, đã nhắc chưa, ai nhận), không thêm cột vào `orders_orders`: migration mới chỉ tạo
   bảng (không `ALTER` bảng lớn), danh sách đơn lọc `pickup_status` bằng truy vấn con trên bảng của chính module.
2. **Người nhận hộ** khai lúc đặt trong `contact` (`proxy_name`, `proxy_phone`, cả hai hoặc không). Giao ở quầy
   (`POST /{id}/pickup/handover`, `orders.manage`) bằng mã của người mua hoặc tên + số điện thoại của người nhận hộ
   (so tên bỏ hoa thường / khoảng trắng thừa, so số bỏ định dạng và coi `+84…` = `0…`) → đơn `delivered` qua
   `Lifecycle::apply`, ghi ai đã nhận.
3. **Mã nhận 6 số** sinh từ phần ngẫu nhiên của UUIDv7 và **lưu nguyên** (không băm): người mua phải đọc lại được mã
   trong đơn của mình (`GET /me/orders/{id}`, chỉ người mua thấy) và `notifications` gửi mã qua event
   `orders.pickup_ready` (`after.pickup_code`). Đây là mã đối chiếu ở quầy do nhân viên nhập, không phải secret đăng nhập
   — ngoại lệ có chủ đích của luật "event không mang secret / token".
4. **Sẵn sàng** (`POST /{id}/pickup/ready`, `orders.manage`): nhận đơn nếu còn `confirmed`, giữ hàng
   `hold_days` của phương thức nhận tại cửa hàng (chụp vào `shipping_option` lúc đặt; không khai thì 3 ngày). Gọi lại:
   kết quả cũ.
5. **Job `orders.expire_pickups`** (mỗi giờ): còn dưới 24 giờ → `orders.pickup_reminder` một lần; quá hạn →
   `orders.pickup_expired` và theo cài đặt kế thừa `orders.pickup_expired_action`: `cancel_refund` (mặc định — huỷ
   `pickup_expired`, trả kho qua `orders.order_cancelled`, hoàn tiền) hay `keep_waiting` (đánh dấu hết hạn, vẫn giao được).

Hệ quả: lịch định kỳ thứ bảy của `orders`; ba event mới; không tool MCP (việc ở quầy).
