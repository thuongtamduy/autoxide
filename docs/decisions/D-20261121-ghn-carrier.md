## D-20261121-ghn-carrier · Bộ nối Giao Hàng Nhanh: dùng tài khoản hãng của `shipping`, địa chỉ hai cấp theo tên, webhook không chữ ký — 2026-11-21 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. Module `ghn` (hạng thử nghiệm, phụ thuộc `shipping`) cài `ShippingCarrier` và ghi tên vào `CarrierRegistry`. Viết
   theo tài liệu công khai của GHN ở https://developer.ghn.vn (đọc ngày 2026-11-21; danh sách trang ở đầu
   `modules/ghn/src/lib.rs`), chưa chạy với dịch vụ thật.
2. **Tài khoản GHN của khách nằm trong tài khoản hãng sẵn có của `shipping`** (`shipping_carrier_accounts`:
   `credentials.token` mã hoá, `settings.shop_id` và tuỳ chọn `required_note`, `payment_type_id`, `label_size`,
   `content`; cờ `sandbox` chọn môi trường staging của GHN). Bộ nối vận chuyển KHÔNG khai `TenantSetting` nhóm
   `connection` của D-20261121-provider-accounts; nó chỉ dùng phần gọi mạng của `autoxide_kernel::provider`
   (`http_client`, `send`). Thử kết nối đi qua `ShippingCarrier::check`, không qua `ConnectionProbe`.
3. **Địa chỉ gửi GHN theo mô hình hai cấp sau 01/07/2025, bằng tên** (`to_province_name`, `to_ward_name`,
   `is_new_to_address: true`), lấy thẳng từ địa chỉ của module `address`. Không có bảng ánh xạ mã tỉnh / quận / phường
   của GHN. Báo giá dùng `shipping-order/preview` (chạy thử của tạo đơn) vì các endpoint tính phí, thời gian giao và
   dịch vụ khả dụng chỉ nhận mã quận của mô hình cũ.
4. **Webhook của hãng có địa chỉ mang cả khách lẫn tài khoản**: route mới
   `POST /api/v1/shipping/webhooks/{carrier_code}/{hook}/{token}` — `{hook}` tìm khách bằng
   `ProviderAccounts::resolve_webhook`, `{token}` tìm tài khoản hãng (lưu băm). `webhook_path` của tài khoản hãng mới
   trả địa chỉ dạng này; route cũ `…/{carrier_code}/{token}` (khách theo header / tên miền) giữ nguyên.
5. **GHN không ký webhook.** Bộ nối coi một callback là hợp lệ khi nó tới đúng địa chỉ mang token ngẫu nhiên của tài
   khoản hãng VÀ `ShopID` trong thân là cửa hàng của tài khoản đó; sai → `shipping.webhook_signature` (401), không
   lưu. Đây là ngoại lệ có chủ đích cho câu "nhà cung cấp không có chữ ký hay khoá thì không nối" của hướng dẫn
   provider-adapters: token của tài khoản trong địa chỉ chính là khoá (192 bit, mỗi tài khoản một cái, lưu băm).
6. Chỉ callback loại `create` và `switch_status` sinh cập nhật trạng thái. `damage` và `scrap` thành `lost`;
   `exception` và trạng thái chưa có trong tài liệu bị bỏ qua (không đoán). Tiền thu hộ không đi qua hệ thống
   (D-187): GHN không có API bảng kê COD trong tài liệu, đối soát bằng nhập bảng kê sẵn có.

Lý do: `shipping` đã có tài khoản hãng theo khách từ bước 44 (nhiều tài khoản mỗi khách, theo nút tổ chức, mã hoá,
có thử kết nối) và trait `ShippingCarrier` nhận sẵn tài khoản đó — khai thêm cấu hình `connection` là hai cách cho
một việc và mất khả năng một khách có nhiều cửa hàng GHN. Module `address` đã theo mô hình hai cấp nên gửi tên là
đủ, và bảng ánh xạ mã sẽ là dữ liệu phải bảo trì theo từng đợt sáp nhập. GHN chỉ cho đặt "custom header" tĩnh cho
webhook; một header tĩnh không mạnh hơn token trong địa chỉ, mà `shipping_webhook_inbox` lại lưu header nguyên văn
nên secret sẽ nằm dạng rõ trong database.

Điểm tài liệu của GHN chưa rõ, cần kiểm bằng tài khoản thật: `preview` có nhận người nhận giữ chỗ không (bộ nối gửi
tên `Quote`, số `0900000000`); GHN khớp tên tỉnh / phường viết đầy đủ ("Thành phố Hồ Chí Minh") hay chỉ tên chuẩn
của danh mục mới; kiểu nội dung của trang in vận đơn (bộ nối nhận ra PDF theo đầu file, còn lại coi là HTML);
`detail` có trả lịch sử hành trình không (bộ nối chỉ lấy trạng thái hiện tại); giới hạn tần suất gọi (tài liệu
không nêu). Giá trị khai báo trên 5 000 000 đ được khai ở mức trần của GHN; kiện không có kích thước gửi 10 cm mỗi
cạnh. Vận đơn lấy hàng trả từ người mua (`Direction::Return`) chưa hỗ trợ: `ghn.direction_unsupported`.

Đã cân nhắc: khai tài khoản GHN bằng `TenantSetting` nhóm `connection` như bộ nối thanh toán — loại, lý do ở trên.
Dùng `calculate-fee` / `leadtime` với bảng ánh xạ sang mã quận cũ của GHN — loại: mô hình cũ, phải bảo trì bảng mã.
Bắt khách đặt một header bí mật ở trang webhook của GHN — loại cho tới khi hộp thư webhook của `shipping` che được
header do bộ nối khai. Sửa route webhook cũ thay vì thêm route — loại: thay đổi phá vỡ (D-059).
