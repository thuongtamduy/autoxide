## D-20261121-login-link-claims · Link đăng nhập trên tài khoản chưa xác minh email: xác minh và gỡ mọi cửa đặt trước — 2026-11-21 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Khi link đăng nhập (gửi tới email của tài khoản) được dùng trên tài khoản mà email chưa xác minh, trong
cùng transaction: đánh dấu email đã xác minh, xoá mật khẩu, thu hồi mọi phiên, passkey, API key, TOTP và liên kết OIDC
của tài khoản. Tài khoản đã xác minh: không đổi gì.

Lý do: ai cũng đăng ký trước được bằng email của người khác rồi giữ mật khẩu / phiên chờ chủ email dùng tài khoản
("account pre-hijacking"). Người mở link mới chứng minh giữ email; mọi thứ đặt trước đó không ai bảo đảm. Chủ tài
khoản thật mất mật khẩu tự đặt khi chưa xác minh email thì đặt lại bằng email.

Đã cân nhắc: chỉ đánh dấu đã xác minh — kẻ đăng ký trước vẫn giữ mật khẩu và phiên.
