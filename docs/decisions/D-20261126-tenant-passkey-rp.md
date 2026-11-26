## D-20261126-tenant-passkey-rp · Passkey theo khách: RP ID là tên miền đang xác minh của chính khách, kiểm lúc đặt và lúc dùng — 2026-11-26 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định:

- Kernel thêm loại cài đặt khách **tên miền của chính khách**: `TenantSetting::own_domain(…)`, nhóm `domain`
  (`DOMAIN_GROUP`). Giá trị là tên miền trần viết thường (`shop.example`). Chế độ nhiều khách: phải là tên miền khách
  này ĐANG xác minh — kiểm lúc đặt (422 `kernel.domain_not_verified` / `kernel.domain_name_invalid`) và **mỗi lần
  dùng** (`SiteLinks::own_domain`, đọc ảnh chụp tên miền đã xác minh như CORS và link mẫu, không truy vấn mỗi lần);
  không còn thoả thì bỏ qua, ghi cảnh báo, và `GET /kernel/settings` báo `problem: kernel.domain_unverified`. Một
  khách: chỉ kiểm dạng (không có bảng tên miền để đối chiếu) — như luật P16 của D-20261125-tenant-site-verified.
- `auth.passkey_rp_id` (loại trên): RP ID của passkey ở khách, origin `https://<tên miền>`. Không đặt hay không còn
  hợp lệ → cấu hình máy chủ `AUTOXIDE_AUTH_PASSKEY_RP_ID` / `_ORIGINS` (máy chủ không cấu hình → passkey tắt ở khách
  đó, 403 `auth.passkeys_disabled`). Động cơ WebAuthn dựng theo khách mỗi nghi thức (chỉ đọc tên miền, rẻ).
- `GET /kernel/client-config` phần `auth` theo khách: `passkeys`, `passkey_rp_id` (mới), `self_registration`.

Lý do: WebAuthn gắn passkey với RP ID; trình duyệt chỉ cho trang có origin thuộc RP ID dùng passkey đó. Nếu khách A
giữ RP `shop.vn` sau khi `shop.vn` về tay khách B, trang của B lấy được chữ ký passkey của người dùng A và gửi tới
endpoint đăng nhập của A. Kiểm "đang xác minh, của chính khách" ngay lúc dùng chặn đúng chỗ đó; test chứng minh
passkey không đăng nhập được vào A sau khi tên miền chuyển sang khách khác.

Đã cân nhắc: cho khách nhập RP ID tự do — mở đường cho trường hợp trên. Lưu RP ID vào từng passkey để passkey cũ (RP
máy chủ) vẫn dùng song song — thêm cột, migration và chọn RP theo credential trước khi biết người dùng; để sau nếu cần.
Đổi RP ID làm passkey đã đăng ký không dùng được (đặc tính của WebAuthn) — ghi trong mô tả setting.
