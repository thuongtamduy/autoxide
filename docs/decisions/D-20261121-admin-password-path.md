## D-20261121-admin-password-path · Nơi ghi mật khẩu tự sinh là một Setting; gate chặn test ghi file vào cây mã nguồn — 2026-11-21 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. Đường dẫn file mật khẩu tự sinh của quản trị viên đầu tiên (D-20261121-admin-password-file) là Setting
   `AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_OUT`, mặc định `data/initial-admin-password` (theo thư mục làm việc, như
   `AUTOXIDE_FILES_DIR` / `AUTOXIDE_MAIL_FILE_DIR`; trong image Docker là volume dữ liệu). Test đặt nó vào thư mục tạm.
2. `gate.sh` chụp danh sách file chưa theo dõi (không tính file trong `.gitignore`) lúc bắt đầu; trước khi báo xanh,
   có file mới thì đỏ và liệt kê chúng. Bỏ dòng `.gitignore` riêng cho file mật khẩu: file đó không còn được sinh trong
   cây mã nguồn.

Lý do: hằng số tương đối khiến test `e2e::startup` (chạy trong thư mục của crate) ghi file vào `crates/app/data/` và
file bị commit nhầm — cùng loại lỗi với #318. Ignore file đó chỉ che lỗi; kiểm ở gate bắt mọi trường hợp sau này.

Đã cân nhắc: ghi cạnh `AUTOXIDE_FILES_DIR` (module `files` có thể không được đăng ký, và lưu trữ có thể là S3); kiểm
bằng một test Rust (không thấy file do test của crate khác ghi).
