## D-20261119-step54-step-up · Bước 54: xác nhận lại cho việc nhạy cảm ("sudo mode") — 2026-11-19 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Permission có `risk` `destructive`, `money` hay `sensitive_data` chỉ dùng được khi lần xác thực gần nhất của
phiên đăng nhập còn trong `AUTOXIDE_AUTH_STEP_UP_MINUTES` phút (mặc định 15, từ 1 tới 120). Lần xác thực là lúc đăng
nhập, hay lúc `POST /api/v1/auth/me/reauth` thành công. Nếu không còn trong cửa sổ, API trả 401 `step_up_required`
kèm `WWW-Authenticate: Bearer error="insufficient_user_authentication", …, max_age=…` (RFC 9470).

Cách làm:
- Access token mang thêm hai claim: `auth_time` (như OpenID Connect) và `sid` (phiên).
- Dòng refresh token giữ `authenticated_at` (migration `m0042`), nên refresh không làm mất lần xác nhận.
- `/me/reauth` (mật khẩu, mã TOTP, passkey) ghi `authenticated_at = now` cho phiên của chính token gọi, và trả thêm
  `access_token` mới. Cách `email_link` không trả access token.
- Kernel: `Principal::with_step_up(StepUp)`. Kiểm ở `require`, `check` / `require_at`, `scope_condition`, nên REST và
  MCP như nhau. Người gọi không gắn `StepUp` (API key, job, bộ xác thực khác) không bị đòi.
- **Bật mặc định cho mọi khách.** Khách tắt bằng cấu hình theo khách `auth.step_up = false`; đặt cấu hình này
  cần `kernel.settings.manage` và có audit như mọi cấu hình. Chủ dự án giao phiên điều phối quyết, phiên đó chọn bật
  mặc định.
- Tài khoản chưa có MFA xác nhận lại bằng mật khẩu, không bị chặn hẳn; lỗi nêu cả passkey và mã ứng dụng xác thực.

Lý do: Đặc tả 54 P5, theo "sudo mode" của GitHub. Coi lúc đăng nhập là một lần xác thực, nên người vừa đăng nhập không
bị hỏi lại; chỉ phiên đã lâu mới bị hỏi. Nhờ vậy giao diện và mọi test sẵn có không đổi. `sid` trong token giúp
`/me/reauth` làm mới đúng phiên đang gọi, không làm mới mọi phiên của người dùng (phiên bị đánh cắp ở máy khác không
được hưởng). API key không bị đòi, theo đặc tả: việc nhạy cảm của agent đi đường duyệt (P12 bước 51).

Chưa làm, để sau: rào quyền của khách cha buộc bật `auth.step_up` cho khách con; buộc dùng passkey / MFA (không nhận
mật khẩu) khi tài khoản đã có. Phần "điều kiện theo IP và khung giờ trên gán role" của mục 3 làm ở PR kế.

Đã cân nhắc: (1) Lưu lần xác nhận theo người dùng. Bị loại vì phiên ở máy khác cũng được hưởng. (2) Token xác nhận
dùng một lần (`reauth_token`) cho mọi việc nhạy cảm. Bị loại vì mọi endpoint nhạy cảm phải nhận thêm một trường,
và việc đọc (danh sách người dùng) không có thân request.
