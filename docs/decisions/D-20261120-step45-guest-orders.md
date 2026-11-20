## D-20261120-step45-guest-orders · Mua không cần tài khoản của `orders`: mã một lần, header `Order-Token`, khoá liên hệ theo khách, gắn đơn bằng token — 2026-11-20 — agent quyết

Chủ đề: Thương mại

Bối cảnh: mục "Mua không cần tài khoản" của bước 45 (D-201, D-202, D-204). Đặc tả để agent quyết khoá / index của
`orders_lookup_tokens`, cách người mua chứng minh số điện thoại khi gắn đơn, và giả định token đi bằng
`Authorization: Order <token>`.

Quyết định:

1. **Token xem đơn đi bằng header riêng `Order-Token`** (như `Cart-Token`), không bằng `Authorization: Order …`:
   kernel xác thực mọi giá trị của `Authorization` (chọn khách theo người đăng nhập, lớp idempotency) và trả 401 cho
   giá trị không phải token đăng nhập — dùng `Authorization` cho token xem đơn sẽ làm hỏng chọn khách ở chế độ nhiều
   khách. Lệch đặc tả 45 mục "Mua không cần tài khoản".
2. **Ba bảng, chỉ lưu băm:** `orders_lookup_codes` (mã 6 số, băm bằng pepper riêng của khách dẫn từ khoá gốc, sống 10
   phút, sai 5 lần thì hỏng, dùng một lần bằng `UPDATE … WHERE consumed_at IS NULL`; mã đổi địa chỉ gắn với token đã
   xin nó), `orders_lookup_tokens` (SHA-256 của token 32 byte, unique (`tenant_id`, `token_hash`), sống
   `AUTOXIDE_ORDERS_LOOKUP_TOKEN_MINUTES`), `orders_guest_requests` (chống trùng thao tác vãng lai theo
   `Idempotency-Key`: lớp idempotency HTTP của kernel bỏ qua request chưa đăng nhập). Cả ba là dữ liệu tạm, không đi
   theo khách khi tách / gộp. `Idempotency-Key` của thao tác vãng lai không bắt buộc: huỷ vốn không làm hai lần, đổi
   địa chỉ cần mã dùng một lần, mở hỗ trợ không khoá thì mỗi lần một phiếu.
3. **Giới hạn bằng `Throttle` của kernel** (bảng `orders_throttle`): theo cặp (số đơn, số điện thoại E.164)
   `AUTOXIDE_ORDERS_LOOKUP_PER_HOUR` (5) và theo IP `AUTOXIDE_ORDERS_LOOKUP_IP_PER_HOUR` (20 — thêm biến thay cho số
   viết cứng), đếm TRƯỚC khi tìm đơn nên dò số đơn cũng bị chặn; gửi lại mã sớm hơn
   `AUTOXIDE_ORDERS_LOOKUP_RESEND_SECONDS` → 429 `orders.lookup_resend_too_soon`. "Không thấy đơn" và "sai số" cùng
   404 `orders.lookup_failed`, không gửi SMS.
4. **Khoá liên hệ ở kernel:** `SecretKey::phone_key(tenant, &PhoneNumber)` / `email_key` — HMAC-SHA256 của giá trị
   chuẩn hoá bằng khoá riêng của khách (dẫn từ khoá gốc theo khách, D-113). Đặt ở `SecretKey` thay vì hàm tự do
   `tenancy::contact_key` của đặc tả: chuẩn hoá số đã có ở `sms::PhoneNumber`, khoá đã có ở `SecretKey`. Đặt hàng ghi
   `orders_orders.buyer_key` = khoá của số liên hệ (cột có sẵn, trước đây để trống); chống bom hàng (D-194) và
   `customers` (bước 47) dùng cùng khoá.
5. **Gắn đơn bằng token:** `POST /me/orders/claim {lookup_token}` — người mua đăng nhập đưa token xem đơn (đã chứng
   minh số bằng OTP); mọi đơn chưa có chủ cùng `buyer_key` trong khách (và đơn của token) về tài khoản, một câu
   `UPDATE … WHERE buyer_user_id IS NULL` nên gọi lại hay hai người cùng gắn không chiếm đơn đã có chủ. Không cần
   `auth` có cờ "số đã xác thực" (bước 37 chưa có). Event `orders.orders_claimed`.
6. **Đổi địa chỉ:** chỉ khi đơn trước `shipping`, có địa chỉ giao (không nhận tại cửa hàng) và chưa yêu cầu vận đơn
   nào; phí giao tính lại cho cùng `option_key` — khác phí hay cách giao không tới được → 409
   `orders.address_change_needs_payment` (kiểm phí TRƯỚC khi dùng mã, để người mua sửa rồi gửi lại). Event
   `orders.delivery_address_changed` không mang địa chỉ.
7. **Che dữ liệu** (Luật 91/2025/QH15, NĐ 13/2023/NĐ-CP, nguyên tắc tối thiểu hoá — **cần đối chiếu văn bản gốc**):
   response bằng token che số (3 số đầu, 3 số cuối), tên chỉ còn chữ cái đầu của họ và đệm, địa chỉ tới cấp phường /
   xã. Câu SMS là mẫu i18n theo ngôn ngữ của đơn.
8. **Hỗ trợ và trả lời tự động:** `support` đăng ký trước `orders`, nên `orders` lấy `dyn SupportIntake` (tuỳ chọn;
   chưa cài → 404 `orders.support_unavailable`) và gắn `FactSource` ý định `orders.order_status` vào `SupportFacts`
   (D-230) — chỉ trả lời tài khoản chủ đơn hay số đã xác thực trùng số liên hệ. Có thêm `POST
   /me/orders/{id}/support` cho người mua có tài khoản.
9. **`OrderLookups` công bố bằng `context.provide`**: `verified_phone(token)` trả số E.164 khi token còn hạn và số
   trong đơn đúng là số đã qua OTP. `auth` đăng ký TRƯỚC `orders` nên không `require` được lúc khởi động — phần
   `POST /api/v1/auth/phone-signups` (tạo tài khoản từ số đã xác thực) cần thêm chỗ nối muộn ở kernel và tài khoản
   không email ở `auth`; làm ở mục riêng của bước 45.

Căn cứ: NĐ 13/2023/NĐ-CP và Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 (tối thiểu hoá dữ liệu) — cần đối chiếu văn bản
gốc; Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15 (quyền huỷ theo điều kiện giao dịch chung đã công bố).

Hệ quả: chống bom hàng COD đọc thống kê theo `buyer_key`; đơn đặt trước khi có cột này (không có `buyer_key`) vẫn tra
được (so số trong `contact`) và vẫn gắn được qua chính token của nó.
