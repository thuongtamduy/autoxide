## D-20261127-initial-password-command · Lệnh `autoxide initial-password` thay `cat` và `rm` của quickstart — 2026-11-27 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: Image Docker là distroless (không shell, không `cat`, không `rm`), nên hướng dẫn quickstart lấy và xoá file mật khẩu
quản trị viên đầu tiên bằng lệnh con của binary: `autoxide initial-password` (in file ra stdout: email rồi mật khẩu) và
`autoxide initial-password --remove` (xoá file). Đường dẫn lấy từ `AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_OUT`, đúng cài đặt mà `serve`
dùng để ghi. Mã thoát: 0 thành công, 1 khi file không tồn tại hay không đọc được (kèm lời giải thích), 2 khi cấu hình sai. Mật khẩu chỉ
ra stdout khi người dùng tự gọi lệnh; không bao giờ vào log. Lệnh là một phần của hợp đồng CLI (G14).

Lý do: không thêm shell vào image chỉ để đọc một file; lệnh chạy được cả ngoài Docker và nhất quán giữa mọi cách cài. Gợi ý của phiên điều
phối trong lượt rà #587.

Đã cân nhắc: để `busybox` trong image (thêm bề mặt tấn công, trái mục đích distroless); đọc mật khẩu qua endpoint API (mở thêm đường lộ mật khẩu).
