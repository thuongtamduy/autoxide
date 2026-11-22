## D-20261122-provider-environments · Bộ nối: URL webhook đầy đủ theo khách, môi trường thử theo khách, xác thực lại khi đổi tài khoản — 2026-11-22 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định (bổ sung D-20261121-provider-accounts, bước 70 P7):

1. `ProviderAccounts::webhook_url(&tenant, path)` trả URL webhook đầy đủ của một khách cho bộ nối phải gửi URL trong
   từng request (`ipnUrl` của MoMo, `CallbackUrl` của eSMS): ghép từ `AUTOXIDE_HTTP_PUBLIC_URL`; chưa đặt thì lỗi
   `public_url_missing` — không bao giờ đoán từ header `Host`.
2. **Môi trường thử theo khách**: bộ nối mà nhà cung cấp có host sandbox khai cấu hình khách
   `<module>.connection.<loại>.environment` (`production` mặc định | `sandbox`, kiểm bằng
   `autoxide_kernel::provider::Environment`) và chọn giữa HAI địa chỉ cố định của người vận hành
   (`AUTOXIDE_<TÊN>_API_URL`, `AUTOXIDE_<TÊN>_SANDBOX_API_URL`, `Target::Fixed`). Khách không bao giờ tự nhập địa chỉ
   của nhà cung cấp. Khách sandbox của kernel (bước 38) không tới bộ nối thật — `PartnerGuard` đưa bộ giả. `sepay`
   không khai `environment`: tài liệu SePay không có host sandbox riêng cho API giao dịch (chế độ thử nằm trong cùng
   tài khoản); `einvoice_sepay` áp quy ước này.
3. **Đặt / bỏ cấu hình nhóm `connection` đòi lần xác thực gần đây** (step-up, `step_up_required`), qua
   `Principal::require_fresh`, dù `kernel.settings.manage` thuộc loại `sensitive_data` chứ không phải `money`: đổi tài
   khoản nhà cung cấp là đổi nơi tiền về. Người gọi không gắn step-up (API key, job) không bị đòi, như mọi việc nhạy
   cảm khác (D-20261119-step54-step-up). Audit: event `kernel.setting_changed` (chỉ mang tên) đã được module `audit`
   ghi như mọi event — không cần thêm.
4. `WebhookEndpoint` không gắn với nhóm cấu hình nào (nhãn `connection` chỉ để hiển thị); module có tài khoản riêng
   (`shipping` với tài khoản hãng) đăng ký đường của mình và dùng `webhook_token` / `webhook_url` / `resolve_webhook`.
   Bộ nối vận chuyển dùng tài khoản hãng của `shipping`, không khai cấu hình `connection`.

Lý do: ba bộ nối (MoMo, eSMS, GHN) cần cùng lúc; chốt một quy ước ở kernel để không mỗi nơi một kiểu.

Đã cân nhắc: cho khách tự nhập địa chỉ sandbox (mở đường gọi tới địa chỉ tuỳ ý — phải lọc như D-20261121-ssrf-targets
mà không được gì thêm); nâng `kernel.settings.manage` lên loại `money` (mọi cài đặt thường cũng đòi xác thực lại, và
đổi loại của permission là thay đổi người dùng thấy); permission riêng cho nhóm `connection` (giao diện và role sẵn có
phải cấp thêm quyền mới).
