## D-20261125-sms-templates · Mẫu câu SMS mang mã một lần là cấu hình theo khách, nhóm `sms_template` — 2026-11-25 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định:

1. Mỗi câu SMS mang mã một lần là một cấu hình theo khách sẵn có của kernel (`Module::tenant_settings`, D-258),
   tên `<module>.sms_template.<tin>`, nhóm `sms_template` (`autoxide_kernel::sms::SMS_TEMPLATE_GROUP`); khách đặt
   qua `PUT /api/v1/kernel/settings/{name}` (`kernel.settings.manage`), nhân sự nền tảng qua
   `/kernel/tenants/{id}/settings…`, xem cả nhóm bằng `GET /api/v1/kernel/settings?group=sms_template`. Năm cấu hình:
   `compliance.sms_template.phone_verification`, `compliance.sms_template.sign_in`,
   `orders.sms_template.order_lookup`, `orders.sms_template.address_change`, `bookings.sms_template.guest_code`.
2. Kiểm lúc đặt bằng `check_sms_template(raw, allowed)` của kernel: 1–480 ký tự, chỉ chỗ điền `{tên}` module cho
   phép (`code`, `minutes`, thêm `number` ở `orders`), `{code}` đúng một lần — sai thì 422, lỗi ở ô `value`
   (`length` / `placeholder_unknown` / `placeholder_code`). Đọc bằng `tenant_sms_text` (qua `parsed_setting`: mẫu
   đã lưu mà nay sai thì cảnh báo và dùng câu mặc định), điền một lượt như i18n (D-150).
3. Chưa đặt thì câu mặc định như trước: `compliance` và `bookings` một câu cố định, `orders` dịch theo ngôn ngữ của
   đơn. Mẫu khách đặt gửi **nguyên văn** với mọi ngôn ngữ — template đăng ký với nhà mạng là một câu cố định.

Lý do: khách có brandname đăng ký template với nhà mạng thì tin lệch template bị chặn, nên câu viết cứng làm khách
không gửi được mã. Cấu hình theo khách đã có sẵn kiểm giá trị, cache, quyền, audit (event `kernel.setting_changed`)
và màn hình theo nhóm — không cần bảng hay endpoint mới. Hàm kiểm / điền đặt ở kernel cạnh `SmsMessage` để ba module
(và module ngoài) dùng một cách.

Đã cân nhắc: bảng mẫu riêng trong `notifications` (template theo kênh) — loại: mã một lần không đi qua
`notifications` (gửi đồng bộ, ngoài job, mã không nằm trong hàng đợi) và chỉ cần một chuỗi mỗi tin. Một cấu hình
chung cho mọi tin — loại: mỗi tin có template đăng ký riêng và chỗ điền khác nhau. Mẫu theo từng ngôn ngữ — để sau:
template nhà mạng là một câu, chưa có khách cần nhiều bản.
