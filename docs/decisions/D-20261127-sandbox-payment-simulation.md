## D-20261127-sandbox-payment-simulation · Hoàn tất thanh toán giả trong sandbox qua đường webhook — 2026-11-27 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định: Thêm `POST /api/v1/ledger/payments/{id}/simulate` (tool MCP `ledger.simulate_payment`, permission
`ledger.reconcile`) để người thử hoàn tất một thanh toán giả trong sandbox mà không cần tài khoản đối tác thật (báo
cáo dùng thử RC vòng 1, UT-4). Lệnh dựng một `GatewayNotification` như bộ nối dựng từ webhook rồi đi đúng đường của
webhook: `LedgerService::record_notification` (tách từ `receive_webhook`, dùng chung) → job `ProcessGatewayEvent` →
ghi thu / hỏng / `needs_review`, bút toán, event `ledger.payment_succeeded`. Chỉ bỏ bước kiểm chữ ký. Bốn điều kiện,
thiếu một là từ chối: (1) biến `AUTOXIDE_LEDGER_SANDBOX_SIMULATION=true`, mặc định `false` (404
`ledger.simulation_disabled`); (2) khách hiện tại là sandbox (409 `tenancy.sandbox_only`), nên chế độ một khách không
bao giờ dùng được; (3) lượt thu có cờ `sandbox` (cột mới `ledger_payments.sandbox`, m0013, ghi lúc tạo theo
`TenantContext::is_sandbox`) VÀ `provider_ref` có tiền tố `SANDBOX-` của bộ giả kernel (409
`ledger.payment_not_simulated`); (4) lượt thu còn `pending` (409 `ledger.payment_state`). Lệnh đi qua lớp idempotency
chuẩn của kernel (route có xác thực).

Lý do: Trong sandbox kernel bọc mọi `PaymentGateway` (D-232, D-20261119-step38-sandbox-isolation): lượt thu được tạo ở
bộ giả, không đối tác nào gửi webhook về, nên khoản đứng `pending` mãi; người thử từng phải tự ký thông báo MoMo bằng
khoá bịa. Đi qua đúng đường của webhook giữ một cách xử lý cho cả thật lẫn giả (bên nhận như `orders` không rẽ nhánh)
và cho người thử tập cả đường `needs_review`. D-187 (core không giữ tiền) không bị chạm: không có tiền thật, lệnh chỉ
trả lời thay đối tác cho lượt thu chưa từng tới đối tác thật. Cờ lúc tạo cộng tiền tố để một mã giả không lọt qua chỉ
nhờ tiền tố, và một lượt thu tạo trước khi có cột (cờ `false`) không giả lập được.

Đã cân nhắc: Cho sandbox tự thành công ngay khi tạo lượt thu (người thử không tập được đường hỏng hay lệch số, và
module nguồn nhận event trước khi có câu trả lời tạo lượt thu — khác đường thật). Gọi `payment_status` của bộ giả
bằng job định kỳ (chậm, không chọn được kết quả). Ký thông báo bằng khoá riêng của sandbox rồi gửi vào URL webhook
(thêm một secret phải quản lý, mà không thêm an toàn gì so với kiểm quyền và sandbox). Một khái niệm "môi trường
không production" cho cả máy chủ (chưa có; chế độ một khách không có đối tác giả, gọi giả lập ở đó là gọi tên đối tác
thật).
