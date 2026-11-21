## D-20261121-buyer-keys · Khoá người mua cho hạn mức khuyến mãi và cờ đơn đầu — 2026-11-21 — agent quyết

Chủ đề: Thương mại

Quyết định: làm đúng phần "Chống lạm dụng" của đặc tả 43 và "Giỏ: coupon, voucher" của đặc tả 45 (D-222):

1. `orders` điền `Cart.buyer_keys` cho mọi người mua, kể cả vãng lai: `user:<id>` (đăng nhập), `phone:<khoá>` (số nhận
   hàng của giỏ, không có thì số của địa chỉ trong sổ; khoá = `SecretKey::phone_key`, cùng khoá với cột
   `orders_orders.buyer_key`) và `address:<HMAC>` (địa chỉ giao chuẩn hoá: mã nước, tỉnh, xã + dòng địa chỉ chữ thường,
   bỏ dấu câu và khoảng trắng thừa; HMAC-SHA256 bằng khoá dẫn `orders.address-key` của khách). Lúc đặt hàng, số liên hệ
   của lượt đặt được thêm vào trước khi giữ lượt. Khoá không đổi kết quả tính giảm (không vào dấu vân tay của báo giá)
   — chỉ thêm bộ đếm.
2. `promotions` từ chối giữ lượt (`promotions.buyer_unidentified`, 409) cho chương trình có hạn mức mỗi người
   (`per_customer_limit` hay `per_customer_quantity_limit` của flash sale) khi giỏ không có khoá người mua nào. Trước
   đây không có khoá thì không đếm, tức không giới hạn.
3. `first_order` (đặc tả 45) = người mua chưa có đơn nào **không bị huỷ** trong khách, xét theo tài khoản
   (`buyer_user_id`) và theo khoá số điện thoại (`buyer_key`) của giỏ. Đọc "đơn hoàn tất" là đơn đã đặt thành công:
   chờ tới lúc đơn chuyển `completed` thì người mua đặt liền nhiều đơn vẫn được tính là đơn đầu. Không biết người mua
   (vãng lai chưa nhập số) thì **không** phải đơn đầu. Cờ chỉ tính từ dữ liệu của giỏ (không dùng số liên hệ nhập lúc
   đặt), để báo giá ở giỏ và ở bước đặt cho cùng kết quả. Bán tại quầy (POS) vẫn để `false` như trước.

Lý do: hạn mức mỗi người chỉ có nghĩa khi mọi lượt đều có khoá để đếm; một khách đổi tài khoản, bỏ đăng nhập hay mua
vãng lai vẫn trùng số điện thoại hoặc địa chỉ nhận. Cờ đơn đầu phải ổn định giữa báo giá và đặt hàng, nếu không bước
đặt báo "giá đổi" mãi. Truy vấn đơn cũ đi theo index sẵn có (`orders_orders_buyer_idx`, `orders_orders_buyer_key_idx`),
chạy được trên cả SQLite và Postgres.

Đã cân nhắc:
- Bỏ qua chương trình có hạn mức khi không có khoá (ở bước tính giảm): báo giá ở giỏ (chưa có số) và ở bước đặt (có
  số) sẽ khác nhau.
- Tính `first_order` theo cả số liên hệ lúc đặt: số đó không có ở giỏ, báo giá hai bước lệch nhau.
- Chỉ tính đơn `completed`: đơn đầu dùng lại được trong suốt thời gian giao hàng.
