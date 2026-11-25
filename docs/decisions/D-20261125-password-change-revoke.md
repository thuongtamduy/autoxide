## D-20261125-password-change-revoke · Đổi mật khẩu có lựa chọn `revoke_other_credentials` gỡ API key, passkey, liên kết đăng nhập ngoài — 2026-11-25 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: `POST /api/v1/auth/me/password` nhận thêm `revoke_other_credentials` (mặc định `false`). `true` thì trong
CÙNG transaction đổi mật khẩu còn gỡ mọi API key, passkey, liên kết đăng nhập ngoài (OIDC) và token đặt lại mật khẩu
đang chờ — y như khôi phục mật khẩu bằng mail (`PasswordWrite::ChangeRevokingAll` dùng chung nhánh với `Recovery`).
Mô tả endpoint nêu rõ những thứ còn giữ khi không chọn. Sửa L-7 của `docs/security/review-2026-11.md`.

Lý do: người nghi bị lộ mật khẩu thường đổi mật khẩu chứ không đi đường "quên mật khẩu"; trước đây đổi xong thì API key
hay liên kết đăng nhập ngoài kẻ gian đã tạo vẫn dùng được. Mặc định `false` giữ nguyên hành vi cũ của endpoint (luật
kiến trúc 6): đổi mật khẩu định kỳ không được làm gãy các tích hợp đang chạy bằng API key của người dùng.

Đã cân nhắc: mặc định gỡ hết — thay đổi phá vỡ cho client đang đổi mật khẩu thường lệ, mất API key không báo trước;
chỉ liệt kê các thông tin đăng nhập còn lại trong câu trả lời — người dùng vẫn phải gỡ từng cái, và câu trả lời
`TokenResponse` dùng chung với đăng nhập.
