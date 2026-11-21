## D-20261121-admin-password-file · Mật khẩu tự sinh của quản trị viên đầu tiên ghi ra file, không vào log — 2026-11-21 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: khi `AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL` đặt mà không có mật khẩu, `serve` ghi email và mật khẩu tự sinh vào
`data/initial-admin-password` (theo thư mục làm việc; trong image Docker là volume dữ liệu), quyền `0600` trên Unix;
log chỉ ghi email và đường dẫn. Không ghi được file thì khởi động báo lỗi. Hướng dẫn (README EN + VI, MCP guide) đọc
file bằng `docker compose … exec autoxide cat data/initial-admin-password` và xoá nó sau khi đổi mật khẩu. Bản chạy
nhanh (`compose.quickstart.yml`) tắt trang tài liệu API mặc định (`AUTOXIDE_HTTP_API_DOCS` theo `.env`).

Lý do: log thường được gom về hệ thống khác (nhiều người đọc, giữ lâu); ngoại lệ "secret trong log" không còn cần khi
có chỗ ghi riêng tư sẵn trong volume dữ liệu.

Đã cân nhắc: bắt đổi mật khẩu ở lần đăng nhập đầu (cần thay đổi trong module `auth`); in ra stdout ngoài hệ thống log
(vẫn bị gom cùng log của container).

Thay: phần "Không đặt mật khẩu thì sinh ngẫu nhiên và ghi vào log một lần" của D-20261121-migrate-on-start
