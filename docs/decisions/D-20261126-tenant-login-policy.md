## D-20261126-tenant-login-policy · Tự đăng ký và chính sách đăng nhập theo khách: cài đặt ở nút gốc, chỉ siết hơn máy chủ — 2026-11-26 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Mục 2 và 10 của đợt cấu hình theo khách đặt bằng **cài đặt theo cây** của `auth` (đọc ở nút gốc của
khách qua `organizations`, cùng chỗ với `auth.customer_registration` và `auth.support_access` đã có):

- `auth.staff_registration` (true/false): người lạ tự tạo tài khoản nhân sự ở khách này (`POST /register` với
  `realm: staff`, và đăng nhập OIDC lần đầu ở nhà cung cấp cho đăng ký). Chưa đặt → biến chung
  `AUTOXIDE_AUTH_SELF_REGISTRATION` (hành vi cũ). Khách tự mở hay tự đóng cho chính mình; tài khoản mới không có role
  nên mở ở khách A không cho ai quyền gì ở khách A, càng không ở khách khác.
- `auth.password_min_length` (12–128): độ dài tối thiểu khi đặt / đổi / đặt lại mật khẩu, nhận lời mời, tạo tài
  khoản. Hiệu lực = lớn hơn giữa 12 (của máy chủ) và giá trị của khách. Mật khẩu đang có vẫn dùng được. Đặt lại mật
  khẩu: kiểm mức của khách sau khi biết khách từ token nhưng TRƯỚC khi dùng token (mật khẩu ngắn không đốt token).
- `auth.session_days` (1–365): thời hạn mỗi refresh token. Hiệu lực = ngắn hơn giữa
  `AUTOXIDE_AUTH_REFRESH_TOKEN_DAYS` và giá trị của khách — khách đặt dài hơn bị bỏ qua, không lỗi. Áp khi đăng nhập
  và mỗi lần làm mới; token đã cấp giữ hạn cũ tới lần làm mới.
- `auth.totp_issuer` (1–100 ký tự, không `:`): tên trong app xác thực khi bật TOTP, thay `AUTOXIDE_AUTH_TOTP_ISSUER`.
  Không ảnh hưởng mã; app đã thêm tài khoản giữ tên cũ.

Đọc qua `TenantPolicy` (một chỗ, `modules/auth/src/policy.rs`); không có `organizations` thì theo máy chủ. Mỗi lần
đọc là một truy vấn nhỏ, chỉ ở đăng nhập, làm mới phiên, đặt mật khẩu, bật TOTP.

Lý do: chủ dự án yêu cầu phục vụ SaaS nhiều khách bằng cấu hình theo khách; chính sách bảo mật mỗi khách tự chọn
nhưng không được thấp hơn mức an toàn của người vận hành.

Đã cân nhắc: cài đặt khách của kernel (`kernel.settings`, nơi `auth` đặt `auth.step_up` và link mẫu) — chính sách
đăng ký sẵn có (`auth.customer_registration`) ở cài đặt theo cây, để đăng ký nhân sự cạnh nó; chính sách đăng nhập đặt
cùng chỗ để người quản trị tìm ở một nơi. Cho khách nới (phiên dài hơn, mật khẩu ngắn hơn) — trái
yêu cầu "chỉ siết hơn". Từ chối lúc lưu khi khách đặt lỏng hơn — cài đặt theo cây kiểm một giá trị đơn lẻ, không biết
cấu hình máy chủ; bỏ qua lúc dùng và ghi rõ trong mô tả.
