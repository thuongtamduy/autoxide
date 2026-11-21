## D-20261121-migrate-on-start · Tuỳ chọn tự migrate, tạo quản trị viên đầu tiên và chạy task khi `serve` khởi động — 2026-11-21 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: thêm ba việc `autoxide serve` làm lúc khởi động, trước khi nghe port, **đều tắt mặc
định** (`crates/app/src/startup.rs`):

- `AUTOXIDE_MIGRATE_ON_START=true`: áp dụng migration còn chờ của database chính (placement của
  chế độ `multi` vẫn cần `autoxide migrate up --all`).
- `AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL` (+ `AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD` hoặc `…_PASSWORD_FILE`):
  khách mặc định chưa có quản trị viên thì tạo một tài khoản role `admin` qua trait công bố
  `Accounts` / `TenantAdmins` của module `auth` (như `autoxide user create`). Đã có quản trị viên thì
  không làm gì. Không đặt mật khẩu thì sinh ngẫu nhiên và ghi vào log **một lần** — ngoại lệ có chủ
  ý cho luật "không ghi secret vào log": người vận hành chưa có đường nào khác để biết mật khẩu, và
  đổi được ngay sau lần đăng nhập đầu.
- `AUTOXIDE_STARTUP_TASKS`: chạy các task của module (không tham số) theo tên, vd
  `address.import,tax.import_pack,promotions.load_legal_data` — các task này chạy lại không đổi gì.

Cả ba chạy dưới **khoá migration** `autoxide_kernel::migration::MigrationLock`; `autoxide migrate up`
cũng lấy khoá này. Postgres: `pg_try_advisory_xact_lock` trong một transaction giữ mở trên một kết
nối riêng (tiến trình chết thì kết nối đứt, Postgres tự nhả). SQLite: khoá file của hệ điều hành
(`File::try_lock`) trên `<file database>.migrate-lock` cạnh file database (tiến trình chết thì hệ
điều hành tự nhả); SQLite trong bộ nhớ không cần khoá. Bên đến sau chờ, thử lại mỗi 250 ms, tối đa
15 phút, rồi báo lỗi nói rõ.

Khuyến nghị production **không đổi**: chạy `autoxide migrate up` một lần mỗi lần deploy, trước khi
khởi động máy chủ (`docs/DEPLOY.md`). Tuỳ chọn mới dành cho một máy chủ và lần chạy thử đầu tiên
(`compose.quickstart.yml`).

Lý do: góp ý của người chạy thử image lần đầu — muốn chạy được Autoxide bằng một lệnh, không phải
gõ `migrate up` rồi `user create` trong container. Lý do cũ của D-036 ("nhiều bản máy chủ khởi động
cùng lúc sẽ tranh nhau chạy migration") được giải bằng khoá; lý do còn lại ("người vận hành thấy rõ
điều gì sắp đổi") vẫn đúng cho production, nên tuỳ chọn tắt mặc định và tài liệu deploy giữ cách cũ.
Khoá rẽ nhánh theo database vì không có cơ chế chung: advisory lock là cách chuẩn của Postgres; với
SQLite, database là một file trên máy nên khoá file là tương đương (cùng máy, cùng volume). Test
`migration_lock` (kernel) và `e2e::startup` (hai bản `serve` khởi động cùng lúc trên database trống)
chạy trên cả hai database.

Đã cân nhắc: bảng khoá có hạn thuê (lease) trong database — một cách cho cả hai database, nhưng
bảng đó phải có trước mọi migration (ngoài cơ chế migration và luật sở hữu bảng), và tiến trình
chết để lại khoá tới khi hết hạn; advisory lock theo phiên (`pg_advisory_lock`) trên pool — pool có
thể đóng kết nối rảnh giữa chừng và mất khoá, nên giữ transaction để kết nối luôn bị chiếm; tạo
quản trị viên khi "chưa có người dùng nào" — module `auth` chỉ công bố `TenantAdmins`, và thêm
hàm vào `api` của `auth` phải sửa module đó; "chưa có quản trị viên" cũng là điều `autoxide doctor`
kiểm (`auth.administrator`).

Thay: phần "Migration không tự chạy khi khởi động máy chủ" của D-036
