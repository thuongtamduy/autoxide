## D-20261121-provider-accounts · Một cách cho mọi bộ nối nhà cung cấp: tài khoản theo khách, webhook theo khách — 2026-11-21 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định (bước 70 P7; chủ dự án yêu cầu: tài khoản nhà cung cấp là của từng khách, viết theo tài liệu công khai):

1. **Lưu tài khoản bằng cơ chế sẵn có, không bảng mới.** Mỗi trường của tài khoản là một cấu hình theo khách nhóm
   `connection` (`<module>.connection.<loại>.<trường>`, D-258), trường bí mật `.secret()` (mã hoá bằng khoá gốc, không
   bao giờ trả ra API, log, event). Đặt / bỏ / thử bằng API sẵn có `PUT` / `DELETE /kernel/settings/{name}`,
   `POST …/test`, permission `kernel.settings.manage` (loại nhạy cảm). Không có biến môi trường nào cho thứ thuộc về
   khách; địa chỉ API của chính nhà cung cấp là cấu hình của người vận hành (`AUTOXIDE_<TÊN>_API_URL`).
2. **`autoxide_kernel::provider`** là chỗ dùng chung: `ProviderAccounts` (kernel công bố) đọc tài khoản — trường bắt
   buộc chưa đặt → `provider.not_configured` (422), không bao giờ rơi về tài khoản khác; `http_client` / `send` cho lời
   gọi ra ngoài (hạn giờ, không theo chuyển hướng, log không chứa secret, địa chỉ do khách chọn thì chỉ IP public theo
   D-20261121-ssrf-targets); `hmac_sha256`, `hex`, `constant_time_eq` cho chữ ký.
3. **Webhook theo khách.** Bộ nối khai `WebhookEndpoint`; URL của mỗi khách kết thúc bằng một định danh =
   `link_token(khách, HMAC-SHA256(khoá dẫn xuất riêng của khách, đường dẫn))` (192 bit, không lưu ở đâu). Module nhận
   tìm khách từ định danh (`resolve_webhook`) TRƯỚC, rồi bộ nối kiểm chữ ký bằng secret của khách đó; định danh chỉ để
   định tuyến, không thay chữ ký. `ledger` thêm `POST /webhooks/{provider}/{hook}` và `/bank-feeds/{provider}/{hook}`;
   hai route cũ (khách theo tên miền / header) giữ nguyên. Khách xem URL ở `GET /kernel/connections/webhooks`.
4. **Đổi trait — vì mục 2 của D-20261121-vn-providers (secret theo khách), không vì một nhà cung cấp nào:**
   `PaymentGateway::verify_webhook` và `BankFeed::verify_webhook` thành `async` (bộ nối phải đọc secret của khách từ
   kho cấu hình; hàm đồng bộ không đọc được); thêm `webhook_ack` có mặc định `None` — câu trả lời nhà cung cấp chờ
   (SePay cần `{"success": true}`, có nhà cung cấp cần 204). Bộ giả, lớp sandbox và test sẵn có giữ nguyên hành vi.
   Đây là ngoại lệ có chủ đích của luật "response chỉ có ba hình dạng": chỉ cho route webhook của đối tác, và chỉ câu
   trả lời thành công.
5. **Tool MCP:** thêm `kernel.set_setting` (từ chối cấu hình bí mật: `kernel.setting_secret_rest_only`),
   `kernel.delete_setting`, `kernel.test_connection`, `kernel.list_connection_webhooks`. Giữ ý của D-258 mục 3: giá
   trị bí mật không đi qua ngữ cảnh của agent, người đặt qua REST.
6. **Quy ước bộ nối:** mỗi bộ nối một crate `modules/<tên>`, hạng thử nghiệm, comment đầu module ghi URL tài liệu và
   ngày đọc, mô tả ghi "written from the provider's public documentation; not yet verified against the live service";
   test bằng `FakeProviderServer` của testkit trên cả hai database, có test hai khách hai tài khoản và webhook gửi lại.
   Hướng dẫn: `docs/guides/provider-adapters.md`.

Lý do: hợp đồng (`PaymentGateway`…) đã nhận `TenantId` và kho cấu hình theo khách đã có mã hoá, nhóm `connection`, thử
kết nối, trạng thái lần dùng cuối — thiếu đúng ba thứ: cách đọc "bắt buộc phải có", định tuyến webhook khi nhà cung cấp
không gửi được header chọn khách, và bộ dựng máy chủ giả. Một bảng tài khoản riêng sẽ lặp lại kho cấu hình.

Đã cân nhắc: bảng `kernel_provider_accounts` riêng (lặp D-258, thêm migration và API thứ hai cho cùng một việc); lưu
định danh webhook ngẫu nhiên trong database điều khiển (thêm bảng và một truy vấn mỗi webhook; định danh dẫn xuất không
cần lưu, đổi được bằng cách xoay khoá gốc); route webhook chung ở kernel rồi chuyển cho module (kernel phải biết module
nhận; mỗi module nhận tự có route thì đơn giản hơn); cho tool MCP đặt cả secret (secret sẽ nằm trong ngữ cảnh và lịch sử
của agent).

Thay: phần "Một tool MCP chỉ đọc, không tool ghi" của D-258 (nay có tool ghi cho cấu hình không bí mật)
