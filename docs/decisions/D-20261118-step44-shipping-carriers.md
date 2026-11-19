## D-20261118-step44-shipping-carriers · `shipping`: tài khoản hãng mã hoá trong bảng, bảng chuyển trạng thái chung, đặt hãng giữa hai transaction, webhook ghi nguyên văn rồi xử lý bằng job — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả bước 44 mục 3–4 (`docs/steps/44-shipping.md`) và tiêu chí xong yêu cầu ghi lại: sổ hãng cho nhiều hãng,
bảng trạng thái chung và chuyển hợp lệ, webhook ghi nguyên văn rồi xử lý bằng job; cùng các lựa chọn để agent tự quyết.

Quyết định:

1. **Thông tin đăng nhập hãng** lưu mã hoá AEAD trong cột `credentials_sealed` (`Sealer` của kernel, khoá theo khách, gắn
   với ID tài khoản — D-190) thay vì `kernel_tenant_settings`: một khách có nhiều tài khoản của nhiều hãng, tên cấu hình
   của kernel là cố định. API chỉ trả tên các trường đã đặt (`credential_fields`); `CarrierCredentials` che giá trị khi in.
   `check_status` (`never` / `ok` / `failed`) cập nhật bởi `POST /carrier-accounts/{id}/test` và khi gọi hãng thật nhận
   `shipping.carrier_unauthorized`. Tạo tài khoản cần tính năng `shipping.carriers` của gói.
2. **Bảng chuyển trạng thái chung** (`status.rs`): trạng thái chỉ đi tới; cuối là `delivered`, `returned`,
   `picked_up_by_buyer`, `cancelled`, `lost`. Cập nhật của hãng trái bảng hay cũ hơn trạng thái hiện tại (so
   `occurred_at`) chỉ ghi vào hành trình, không đổi trạng thái — `delivered` tới trước `in_transit` thì vẫn là
   `delivered`; ngoại lệ: `delivery_failed → out_for_delivery` (giao lại). Nhân viên đổi trái bảng →
   `shipping.invalid_transition`. Hành trình chỉ thêm, chống trùng bằng unique (`shipment_id`, `event_key`).
3. **Đặt hãng giữa hai transaction:** `pending → booking` (commit) → gọi `create_shipment` ngoài transaction với
   `client_reference` = ID vận đơn → ghi mã vận đơn, `ready_to_ship` (commit). Vận đơn kẹt `booking` (lần chạy lại của job,
   lịch `shipping.reconcile_bookings` mỗi 5 phút cho vận đơn kẹt quá 10 phút) hỏi `find_by_reference` trước — không gọi
   `create_shipment` lần hai mà chưa hỏi. Lỗi tạm (hãng không trả lời) → job chạy lại; lỗi hẳn → `pending` +
   `shipping.booking_failed`.
4. **Webhook:** đường dẫn `/webhooks/{carrier_code}/{token}` (token ngẫu nhiên lưu băm SHA-256, trả một lần khi tạo tài
   khoản; khách xác định như mọi request công khai, token của khách A không khớp tài khoản nào ở khách B → 404). Kiểm chữ
   ký trên thân thô trước khi lưu (sai → 401, không lưu); ghi nguyên văn (bỏ header `authorization`, `cookie`…) vào
   `shipping_webhook_inbox` rồi mới trả 202; job `shipping.process_webhook` đọc lại, kiểm chữ ký lần nữa và áp từng cập
   nhật. Thân lớn hơn `AUTOXIDE_SHIPPING_WEBHOOK_MAX_BYTES` → 413.
5. **Tự quyết theo đặc tả:** vận đơn nhận tại cửa hàng cũng là vận đơn (`ready_for_pickup` → `picked_up_by_buyer` do nhân
   viên đặt) — một chỗ theo dõi mọi cách giao; tự đặt với hãng khi nhận yêu cầu là cờ `auto_book` của phương thức, mặc định
   tắt (nhân viên kiểm hàng trước).
6. **Nhận yêu cầu giao hai lần:** unique (`order_id`, `request_key`) + `request_fingerprint` (SHA-256 của yêu cầu): cùng
   nội dung → vận đơn cũ; khác nội dung → `shipping.request_conflict`, ghi lỗi, không ghi đè. Tách vận đơn (giao một phần)
   tạo vận đơn mới `request_key` = `<khoá cũ>/split-<n>`; tiền thu hộ ở lại vận đơn gốc.
7. **Nhãn của hãng** tải mỗi lần từ hãng, không lưu đệm vào `Storage` (giảm một chỗ giữ dữ liệu cá nhân); thêm đệm khi đo
   được là cần.
8. **Lịch** `shipping.poll_tracking` mỗi 30 phút chỉ hỏi vận đơn của hãng không gửi webhook
   (`CarrierCapabilities::webhooks = false`).

Hệ quả: bước 45 nghe `shipping.shipment_*` (payload `after` là `ShipmentEvent`, không dữ liệu cá nhân); bộ nối hãng thật
theo `docs/SHIPPING.md`.
