## D-20261118-step45-checkout · Đặt hàng của `orders`: saga giữ chỗ, chống trùng, số đơn, thanh toán qua `ledger`, máy trạng thái — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Bối cảnh: mục 3 của bước 45 (đặt hàng). Đặc tả 45 mục "Luồng đặt hàng" đã chốt luồng; còn vài chi tiết lệch code
thật của bước 36, 42, 43 hay để agent quyết.

Quyết định:

1. **Chống trùng ở `orders_checkouts`**, khoá unique (`tenant_id`, `owner_key`, `reference`) — `owner_key` là
   `user:<id>` hay `cart:<id>` (vãng lai), `reference` là `Idempotency-Key`. Dấu vân tay trên nội dung nghiệp vụ (giỏ,
   dấu vân tay của `quote_token`, phương thức, liên hệ, ghi chú). Gửi lại: `placed` → 200 đơn cũ (dựng lại từ bảng đơn, không
   lưu thân trả lời); đang `reserving` → 409 `orders.checkout_in_progress` (có `Retry-After`); `failed` → 409
   `orders.checkout_failed`; khác nội dung → 422 `orders.idempotency_key_reused`. Không dựa vào lớp idempotency HTTP
   (bỏ qua request chưa xác thực).
2. **ID lượt đặt = ID đơn** = mã giữ chỗ của `inventory` / `promotions` (`order_id`). Giữ chỗ gọi trực tiếp ngoài
   transaction; hỏng thì trả trực tiếp. Xác nhận / trả sau khi đơn tồn tại đi qua event: `inventory` tự nghe; `promotions`
   không nghe `orders` nên `orders` tự nghe `orders.order_confirmed` / `orders.order_cancelled` của mình rồi gọi
   `Promotions::confirm` / `release`.
3. **Hạn giữ chỗ là cấu hình** `AUTOXIDE_ORDERS_HOLD_MINUTES` (60): `ledger` không công bố hạn thanh toán trước khi tạo
   lượt thu, nên chỗ giữ lấy hạn cố định đủ dài; `payment_due_at` tạm = giữ − `HOLD_GRACE_MINUTES`, rồi thay bằng
   `expires_at` thật của `ledger` khi lượt thu được tạo (D-218). Phải đặt `HOLD_MINUTES` > hạn của `ledger` + grace
   (ghi trong mô tả biến).
4. **Số đơn** `<năm>-<6 chữ số>` từ dãy `orders.order:<năm>` của cả khách (unique theo khách, không theo nút), cấp trong
   transaction tạo đơn — lượt đặt hỏng ở giữ chỗ không đốt số.
5. **Phương thức:** `cod` (chỉ khi có hàng giao, đơn vào thẳng `confirmed`, `payment_status = cod_pending`);
   `bank_transfer` → `Ledger::create_transfer_payment` (VietQR về tài khoản người bán); còn lại là phương thức của cổng →
   `Ledger::create_payment`. `ledger` chưa cài thì chỉ COD. Cổng lỗi lúc đặt: đơn vẫn chờ thanh toán, hết hạn thì huỷ.
6. **Token trạng thái thanh toán công khai** = `<payment_id>.<SHA-256(khoá riêng của khách, payment_id)>`: không lưu,
   tính lại được khi gửi lại kết quả đặt hàng; sống tới hạn thanh toán (và sau khi đã thu).
7. **Máy trạng thái** `lifecycle.rs`: bảng chuyển là hằng; `apply` (khoá dòng đơn, lịch sử, event) là chỗ duy nhất đổi
   `status`, `create` là chỗ duy nhất tạo đơn có `status`; test đọc mã nguồn kiểm. Chuyển tới trạng thái đang có = không
   làm gì. Thanh toán tới khi đơn đã huỷ → ghi đã thu, hoàn qua `Ledger::refund` (`late_payment`), không mở lại đơn.
8. **Một loại tiền mỗi đơn:** cột `<tên>_minor` cùng một cột `currency` (thay cặp `_minor` / `_currency` cho từng khoản).
9. **`quote_token` so số tiền người mua thấy:** dấu vân tay gồm dòng, tiền, cách giao và (chương trình, số tiền giảm) —
   không gồm dấu vân tay nội bộ của `promotions` (đổi theo cả dữ liệu không làm đổi tiền). Dòng hết hàng / ngừng bán
   báo lỗi của dòng trước khi so báo giá.
10. **Thuế:** nút chưa có hồ sơ thuế thì chưa tính thuế (như `catalog`, bước 41); có thì gọi `TaxEngine::quote` theo
    `jurisdiction` của hồ sơ.

Hệ quả: mục vòng đời (4) thêm chuyển do nhân viên / vận chuyển / kênh và bút toán sổ cái vào `lifecycle`; bước 46 tách
đơn theo người bán bằng `SellerSplit` ở bước tạo đơn.
