## D-20261125-mail-bounces · Sổ thư bị dội và danh sách chặn cho mọi thư, trong module `mail` — 2026-11-25 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định: Module `mail` (nơi gửi) giữ bảng `mail_bounces`: mỗi (khách, địa chỉ chuẩn hoá) một dòng, có `tenant_id`,
ghi trong CÙNG transaction với event `mail.delivery_failed` khi máy chủ nhận từ chối hẳn địa chỉ lúc gửi (SMTP 5xx mã
mở rộng 5.1.x / 5.2.x — nguồn chắc chắn có với SMTP thuần). Đủ `AUTOXIDE_MAIL_HARD_BOUNCE_LIMIT` (mặc định 3) lần dội
cứng thì địa chỉ bị chặn ở riêng khách đó (event `mail.address_suppressed`); `Mailer::send` tới địa chỉ đang chặn không
xếp hàng thư, trả `Ok` như thường (không lộ địa chỉ nào bị chặn qua API quên mật khẩu) và ghi dấu vết trên dòng: số
thư bị bỏ, lần cuối, nguồn (phần đầu `dedupe_key`). Thư bảo mật (đặt lại mật khẩu) cũng bị bỏ — gửi tới hộp thư không
tồn tại vô ích và làm xấu uy tín gửi của khách — nhưng tra được. API `/api/v1/mail/bounces` (Page, lọc `email`,
`suppressed`), `/stats`, `/{id}/release` (đặt lại số lần tính chặn), `DELETE /{id}`; permission `mail.bounces.read`,
`mail.bounces.manage` (dữ liệu nhạy cảm). Giữ `AUTOXIDE_MAIL_BOUNCE_RETENTION_DAYS` (mặc định 365) kể từ lần dội /
lần bỏ thư cuối, job `mail.bounces_cleanup` hằng ngày. Số đo không có nhãn theo khách.

Lý do: trước đây chỉ newsletter ghi nhận thư dội (danh sách chặn riêng của nó, nghe chính event này); thư khác (đặt
lại mật khẩu, thông báo) không tra được, không chặn, gửi lại mãi tới hộp thư không tồn tại. Đặt ở `mail` vì mọi thư
đi qua đó và không dựng module mới (D-20261121-community-first). Chèn `ON CONFLICT DO NOTHING` rồi tăng đếm bằng một
câu `UPDATE` chạy đúng trên cả SQLite và Postgres khi hai job dội cùng địa chỉ cùng lúc (có test đồng thời). Kiểm chặn
thêm một câu `UPDATE` cho mỗi thư xếp hàng — gửi thư không phải đường nóng.

Dữ liệu cá nhân: địa chỉ email là dữ liệu cá nhân nhưng không gắn với tài khoản (thư gửi theo địa chỉ; event
`auth.user_deleted` chỉ mang ID, địa chỉ đã mất khi event tới). Nên yêu cầu của chủ dữ liệu đi theo địa chỉ như
newsletter (`GET ?email=`, `DELETE /{id}`), có thời hạn giữ, và không đăng ký phần xuất theo tài khoản. Mang email trong
`auth.user_deleted` để xoá tự động là việc `[ ]` ở lộ trình (đụng module `auth`).

Đã cân nhắc: đặt ở `notifications` — không thấy thư `auth` gửi thẳng qua `Mailer`. Trả lỗi cho bên gọi khi địa chỉ bị
chặn — lộ địa chỉ qua API công khai, và bên gọi không làm gì hơn được. Chặn ngay lần dội đầu — một lần cấu hình sai
của máy chủ nhận làm mất người dùng thật; ngưỡng đặt được. Gộp danh sách chặn của newsletter vào đây — newsletter còn
nhận dội mềm và khiếu nại qua webhook, đổi nguồn cần migration dữ liệu: để `[ ]`. DSN báo về sau (thư dội không đồng
bộ) và webhook nhà cung cấp chung: `[ ]`.
