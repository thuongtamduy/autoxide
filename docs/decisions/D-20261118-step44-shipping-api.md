## D-20261118-step44-shipping-api · Module `shipping`: hợp đồng `api` cho bước 45, sổ hãng `CarrierRegistry`, sổ địa chỉ không cần permission, địa chỉ nước ngoài trong `Address` — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả bước 44 (`docs/steps/44-shipping.md`) phác thảo trait; bước 45 (`orders`) cần báo giá, sổ địa chỉ và
yêu cầu giao qua event. Khi dựng khung còn vài chỗ đặc tả chưa nói hay lệch với code thật.

Quyết định:

1. **`shipping::api` công bố ba trait:** `ShippingRates` (`quote`, `confirm_option` — kiểm lại một cách giao theo
   `option_key` lúc đặt hàng, lệch → `shipping.option_changed`), `AddressBook` (`get`, `default_for` — bước 45 đọc địa chỉ
   người mua theo ID) và `CarrierRegistry`. Đặc tả 45 gọi tên `ShippingQuotes`; tên thật là `ShippingRates` (đặc tả bên
   cung cấp thắng). Kiểu trong `api` đều `#[non_exhaustive]` kèm hàm dựng (`QuoteRequest::new`, `ShipmentRequest::new`,
   `QuoteItem::new(…).weight(…)`…) để thêm trường sau không phá bước 45. `ShippingOption` có sẵn `free_over` (cho
   `free_shipping_remaining`), `tax_category` (dòng thuế của phí giao), `eta`, `ready_at`, `hold_days`.
2. **`ShipmentRequest` thêm `buyer_id` và `quoted_fee`:** trả lời tự động về hành trình chỉ cho chủ đơn (D-230) cần biết
   người mua; phí đã báo cần để đối chiếu với phí thật của hãng.
3. **Sổ hãng là trait `CarrierRegistry` do `shipping` công bố,** không dùng `StartContext::contribute`: bộ nối hãng là
   module đăng ký SAU `shipping`, mà `contributions` chỉ thấy module đăng ký trước. Trùng mã hãng → lỗi lúc khởi động.
   `ShippingCarrier` thêm `check` (nút "Kiểm tra kết nối", cập nhật `check_status`). Thông tin đăng nhập là
   `CarrierCredentials` (bản đồ chuỗi, `Debug` che giá trị, không `Serialize`).
4. **Sổ địa chỉ của chính người mua không cần permission** (`/me/addresses…`): tài khoản người mua không có permission
   (bước 34), như `/me/tickets` của `support`. Bỏ permission `shipping.me.addresses` của đặc tả; nhân viên đọc sổ của
   người mua bằng `shipping.addresses.read`.
5. **Địa chỉ nước ngoài dùng chung kiểu `Address` của module `address`** (không thêm kiểu riêng): `province_code` /
   `province_name` là vùng (bang, tỉnh), `commune_name` là thành phố, `commune_code` rỗng; mã bưu chính là cột riêng
   `postal_code` của sổ và vận đơn. Địa chỉ Việt Nam bắt buộc có module `address` để kiểm xã thuộc tỉnh, còn hiệu lực.
   `Address` là `#[non_exhaustive]` nên `shipping` dựng lại từ cột bằng `serde`.
6. **Đúng một địa chỉ mặc định:** cột `default_key` (= `buyer_id` khi mặc định, NULL khi không) có unique
   (`tenant_id`, `default_key`); đặt mặc định bỏ cờ mọi địa chỉ khác trong cùng transaction. NULL không trùng nhau trên
   cả SQLite và Postgres. Xoá địa chỉ mặc định thì địa chỉ sửa gần nhất còn lại thành mặc định.
7. **Mọi bảng của bước 44 tạo trong migration đầu** (`m0001_create_shipping`), theo bảng của đặc tả, để các mục sau không
   phải thêm migration; cột bổ sung so với đặc tả: `method_kind`, `buyer_id`, `delivered_at`, `last_error_code`, `note`
   trên vận đơn, `auto_book` trên phương thức (tự đặt với hãng, mặc định tắt), `content_hash` trên bảng kê,
   `discrepancy_key` (chống trùng) trên chênh lệch COD. Thông tin đăng nhập hãng lưu mã hoá trong cột
   `credentials_sealed` (thay `secret_ref`).

Hệ quả: bước 45 dựng `QuoteRequest` / `ShipmentRequest` bằng hàm dựng và lấy trait bằng
`context.require::<dyn autoxide_shipping::api::ShippingRates>()` (đăng ký sau `shipping`).
