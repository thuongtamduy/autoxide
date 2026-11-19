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

Quyết định — hàng đặt trước (D-204):

6. **Đồng ý điều kiện trước khi đặt:** báo giá giỏ đã trả `preorder` (ngày giao dự kiến) cho từng dòng; `POST /checkout`
   bắt buộc `accept_preorder_terms = true` khi giỏ có hàng đặt trước (lỗi trường, gom cùng lỗi khác — D-083), lúc đồng ý
   lưu ở `orders_orders.preorder_accepted_at`. Chứng cứ đồng ý của `compliance` (`ConsentLedger`) dành cho xử lý dữ liệu
   cá nhân, không dùng cho điều kiện bán hàng.
7. **Giao nhiều lần** (`orders_fulfillments`, khoá gửi `shipping` là `fulfillment:<id>`, thay khoá `order:<id>:<version>`):
   mỗi `POST /{id}/shipments` gom các đơn vị đã có hàng chưa nằm trong lần giao nào chưa hỏng — hàng sẵn đi ngay, hàng đặt
   trước khi `inventory.preorder_allocated` báo đã chia đủ. Không có gì mới mà còn lần giao đang chờ → gửi lại đúng yêu cầu
   đó; không có gì cả → 409 `orders.nothing_to_ship`. Vận đơn tới tay người mua chỉ đưa đơn tới `delivered` khi mọi đơn vị
   cần giao (trừ phần đã huỷ) nằm trong lần giao đã giao; không thì đơn dừng ở `shipping`. Lần giao hỏng thì đơn vị của nó
   được giao lại ở lần sau.
8. **COD giao nhiều lần:** lần trước thu tiền hàng của lần đó (theo tỉ lệ đã trả), lần cuối thu phần còn lại của đơn (trừ
   tiền hàng của lần khác chưa thu và phần đã huỷ). `shipping.cod_collected` cộng theo từng lần giao (ghi
   `cod_collected_minor`, event tới lại không cộng hai lần); chưa đủ → `payment_status = partially_paid`, đủ → `paid`.
9. **Dời ngày:** nghe `catalog.preorder_date_changed`; ngày mới muộn hơn ngày đã hứa (chụp lúc đặt) → ghi
   `preorder_delayed_to`, phát `orders.preorder_delayed` (một lần cho mỗi ngày mới) cho thông báo. Người mua được huỷ
   **miễn phí, bỏ qua `buyer_cancel_until`**: cả đơn khi đơn chưa giao đi; đơn đã giao một phần thì huỷ phần đặt trước
   chưa có hàng.
10. **Ngừng bán đặt trước** = biến thể đặt trước bị tắt (`catalog.variant_updated`, `after.active = false`) hay bị xoá.
    Chuyển chế độ bán sang có sẵn **không** coi là ngừng (thường là hàng đã về). Đơn chỉ còn chờ hàng đó và chưa giao gì →
    huỷ cả đơn (`preorder_stopped`) và hoàn tiền; đơn có phần khác → ghi `cancelled_quantity` cho phần chưa có hàng, phát
    `orders.preorder_cancelled`, hoàn đúng tiền của phần đó (không vượt số đã thu) qua đường hoàn tiền chung.
11. **Chỗ giữ của phần huỷ ở `inventory`:** `Inventory::release` chỉ trả theo cả đơn; phần đặt trước bị huỷ khi đơn vẫn
    còn hiệu lực cần `inventory` nghe `orders.preorder_cancelled` (`after.lines[{line_key, variant_id, quantity}]`) — đã
    nhắn phiên của `inventory`. Thanh toán `on_arrival` / trả trước một phần thuộc mục "Trả trước một phần" (D-232).

Hệ quả: lịch định kỳ thứ bảy của `orders`; năm event mới (`orders.pickup_*`, `orders.preorder_delayed`,
`orders.preorder_cancelled`); bảng `orders_pickups`, `orders_fulfillments`; không tool MCP mới (việc ở quầy).
