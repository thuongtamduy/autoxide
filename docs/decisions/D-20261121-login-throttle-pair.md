## D-20261121-login-throttle-pair · Khoá đăng nhập theo cặp email + địa chỉ IP, trần rộng hơn cho cả email; lần sai của tài khoản đã biết đếm theo ID — 2026-11-21 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: `POST /login` đếm lần sai theo cặp email + địa chỉ client (`AUTOXIDE_AUTH_LOGIN_MAX_FAILURES` lần trong
một khoảng), cộng một trần gấp 10 lần cho cả email (mọi địa chỉ cộng lại). Không biết địa chỉ thì cặp là "email +
không rõ" (như trước). Kiểm mật khẩu hiện tại và mã MFA khi đăng nhập đếm theo ID tài khoản (`account:<id>`), không
theo email — và kiểm mật khẩu hiện tại trong đúng loại tài khoản (nhân sự / người mua) của người dùng.

Lý do: khoá cứng theo email cho phép bất kỳ ai khoá tài khoản người khác (kể cả quản trị viên) bằng vài request mỗi
khoảng. Đếm theo cặp là cách chuẩn (OWASP Authentication Cheat Sheet): người thử sai chỉ khoá chính mình; trần của cả
email vẫn chặn đoán phân tán từ nhiều địa chỉ, và muốn khoá chủ tài khoản phải dùng nhiều địa chỉ hơn hẳn. Người mua
chỉ có số điện thoại có email rỗng: đếm theo email làm mọi người như vậy dùng chung một bộ đếm.

Đã cân nhắc: chỉ đếm theo IP — kẻ đoán đổi email liên tục từ một địa chỉ; backoff tăng dần mỗi lần sai — vẫn là
khoá theo email, chỉ ngắn hơn; CAPTCHA — core chỉ có API, giao diện tự thêm nếu muốn.

Thay: phần "khoá theo email" của D-079
