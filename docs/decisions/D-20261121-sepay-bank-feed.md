## D-20261121-sepay-bank-feed · Bộ nối SePay là nguồn biến động số dư (`BankFeed`), không phải cổng giữ tiền — 2026-11-21 — agent quyết

Chủ đề: Thương mại

Quyết định (bước 70 P7, "Thanh toán: SePay"; theo D-20261121-vn-providers và D-20261121-provider-accounts):

1. Module `sepay` (crate `autoxide-sepay`, hạng thử nghiệm) cài **`BankFeed`**: tiền người mua chuyển thẳng vào tài khoản
   ngân hàng của khách, SePay chỉ báo biến động, `ledger` tự khớp thanh toán VietQR theo mã chuyển khoản + đúng số tiền
   như luồng sẵn có (D-187: core không giữ tiền). Không cài `PaymentGateway`: trang thanh toán của cổng SePay (thẻ,
   NAPAS) khởi tạo bằng form POST có chữ ký từ trình duyệt, không khớp `checkout_url` của hợp đồng — để sau, không đổi
   hợp đồng vì một nhà cung cấp.
2. **Webhook** (`POST /api/v1/ledger/bank-feeds/sepay/{hook}`): khách đặt `sepay.connection.bank.webhook_secret` thì bắt
   buộc chữ ký `X-SePay-Signature: sha256=hex(HMAC-SHA256(secret, "{X-SePay-Timestamp}.{thân thô}"))`, lệch quá 300 giây
   bị từ chối; chỉ đặt `…webhook_api_key` thì kiểm `Authorization: Apikey …`; chưa đặt gì thì từ chối
   (`provider.not_configured`) — không nhận webhook không xác thực. Trả đúng `{"success": true}` (SePay chỉ tính thế là
   đã nhận). Chỉ nhận thân JSON.
3. **Kéo** (`fetch`, lịch `ledger.poll_bank_feed`): `GET /v2/transactions` của SePay API v2 bằng API token của khách, lọc
   theo tài khoản, sắp `transaction_date` tăng dần, đọc hết các trang của một mốc `since_id` rồi mới lấy giao dịch cuối
   làm mốc kế (đúng lời dặn của tài liệu); lần đầu đọc lùi 3 ngày. Khách chưa đặt API token (chỉ dùng webhook) thì trả
   trang rỗng, không lỗi.
4. **Mã chống trùng** của một biến động phải giống nhau dù tới qua webhook hay API (webhook mang `id` số nguyên, API
   mang UUID): có mã tham chiếu ngân hàng thì dùng `ref:<tài khoản>:<ngày>:<mã tham chiếu>:<số tiền>`; không có thì dùng
   `id` của nguồn — bản thứ hai (nếu có) không khớp lại vào thanh toán mà vào hàng chờ của `ledger`.
5. Địa chỉ API là cấu hình của người vận hành `AUTOXIDE_SEPAY_API_URL` (mặc định `https://userapi.sepay.vn/v2`).

Nguồn (đọc ngày 2026-11-21): https://developer.sepay.vn/vi/sepay-webhooks/tich-hop-webhook,
https://developer.sepay.vn/vi/sepay-webhooks/xac-thuc, https://developer.sepay.vn/vi/sepay-webhooks/bao-mat,
https://developer.sepay.vn/vi/sepay-webhooks/xu-ly-loi, https://developer.sepay.vn/vi/sepay-webhooks/doi-soat-giao-dich,
https://developer.sepay.vn/vi/sepay-api/v2/xac-thuc, https://developer.sepay.vn/vi/sepay-api/v2/giao-dich/danh-sach,
https://developer.sepay.vn/vi/sepay-api/v2/tai-khoan-ngan-hang/danh-sach. Chưa kiểm với dịch vụ thật.

Căn cứ pháp lý: nền tảng không làm trung gian thanh toán — tiền không qua tài khoản của nền tảng (Nghị định
52/2024/NĐ-CP về thanh toán không dùng tiền mặt; D-187); cần đối chiếu văn bản gốc / người có chuyên môn pháp lý khi mở
thêm hình thức thu hộ.

Lý do: dịch vụ lõi của SePay là báo biến động số dư của chính tài khoản người bán — đúng nghĩa `BankFeed` đã có và luồng
VietQR tự khớp đã có test; không cần hợp đồng mới.

Đã cân nhắc: cài `PaymentGateway` bằng cổng thanh toán SePay (cần form POST từ trình duyệt và IPN riêng — hợp đồng hiện
tại không diễn đạt được, để sau); dùng `id` của từng nguồn làm mã chống trùng (cùng một giao dịch tới qua hai đường sẽ
thành hai dòng); chỉ dùng mã tham chiếu không kèm ngày và số tiền (mã ngắn của vài ngân hàng có thể lặp lại).
