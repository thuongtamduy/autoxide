# Lộ trình

Dựng theo thứ tự. Mỗi bước xong khi `./gate.sh` xanh trên **cả SQLite và
Postgres**; agent đánh dấu `[x]` rồi commit. Chi tiết thư viện: [`STACK.md`](../STACK.md).

Trạng thái: **bước 0–12 xong**; đang ở bước 13 (củng cố core), mục "CI chạy xanh trên GitHub".

## 0. Khung dự án

- [x] Workspace Cargo, cấu hình lint chung (cấm `unsafe`, cấm `unwrap` ngoài test)
- [x] Khai `license = "MIT OR Apache-2.0"` cho mọi crate
- [x] `README.md` (tiếng Anh) + `README.vi.md`: dự án là gì, cách chạy, license
- [x] Ghim phiên bản thư viện nền; xác nhận chúng tương thích với nhau
- [x] `gate.sh`: fmt, clippy, test SQLite, test Postgres (Docker)
- [x] Kiểm ranh giới module tự động trong gate

## 1. Nền

- [x] Cấu hình từ env, kiểm lúc khởi động
- [x] Kết nối database: tách đọc / ghi, WAL trên SQLite
- [x] Migration theo module (up, down, status)
- [x] Kiểm trong gate: tên bảng mang tiền tố module, không foreign key xuyên module
- [x] Hợp đồng module + đăng ký ở một chỗ (khi có module đầu tiên: thêm `"modules/*"`
      vào `members` của `Cargo.toml` gốc)
- [x] Kiểu lỗi chung, envelope trả về, pagination theo UUIDv7
- [x] HTTP: `/api/v1`, request id, timeout, giới hạn body, CORS, graceful shutdown
- [x] OpenAPI sinh từ code + kiểm không lệch trong gate
- [x] Health check (`/health/live`, `/health/ready`)
- [x] Log, metrics (`/metrics`), tracing
- [x] CLI: `serve`, `migrate` (`serve` từ chối chạy khi còn migration chưa áp dụng)

## 2. Đăng nhập, role và permission

- [x] Người dùng, hash mật khẩu Argon2id
- [x] Đăng nhập: access token sống ngắn + refresh token rotation, phát hiện reuse
- [x] Role và permission; module tự khai báo permission của mình
- [x] Rate limiting; login throttling
- [x] CLI tạo người dùng đầu tiên
- [x] Tạo người dùng: quản trị viên tạo; tự đăng ký bật tắt bằng cấu hình (mặc định tắt)
- [x] API key cho máy và AI agent: tạo, thu hồi, giới hạn theo permission

## 3. MFA

- [x] TOTP: bật, xác nhận, tắt
- [x] Recovery code dùng một lần
- [x] Đăng nhập hai bước khi đã bật MFA

## 4. Event và audit log

- [x] Event ghi cùng transaction rồi mới phát
- [x] Audit log chỉ thêm: ai, làm gì, bản ghi nào, lúc nào, trước / sau
- [x] API xem audit log (có permission riêng)

## 5. Job queue

- [x] Job lưu trong database; claim job an toàn khi nhiều worker (test đồng thời)
- [x] Retry có backoff, dead-letter queue
- [x] Lịch chạy định kỳ (job đầu tiên: dọn dòng hết hạn của `auth_login_throttle`,
      `auth_refresh_token`, `auth_mfa_challenge`, và event đã giao xong trong các bảng
      `<module>_events`)
- [x] Process role: `api`, `worker`, hoặc cả hai

## 6. i18n và idempotency

- [x] Thông báo lỗi và hệ thống theo i18n key (vi, en), chọn theo header; mặc định tiếng Anh
      (`code` của lỗi là i18n key, câu tiếng Anh mặc định đã nằm ở chỗ tạo lỗi — D-039;
      việc còn lại: bản dịch vi tra theo `code`, chọn ngôn ngữ theo header, và gate kiểm mọi
      `code` đều có bản dịch)
- [x] Idempotency key cho lệnh ghi (test gửi trùng đồng thời)
- [x] Đánh dấu endpoint deprecated

## 7. Email

Đặt sau job queue vì mail gửi qua job để có retry.

- [x] Gửi mail qua SMTP sau một trait; khi chưa cấu hình SMTP thì ghi mail ra file (cho dev và test)
- [x] Mẫu mail theo i18n key (vi, en)
- [x] Đặt lại mật khẩu: token dùng một lần, có hạn, không lộ việc email có tồn tại hay không
- [x] Người dùng tự đổi mật khẩu của mình (cần nhập mật khẩu cũ); quản trị viên đặt mật khẩu
      cho tài khoản tạo không kèm mật khẩu (service `set_password` đã có)
- [x] Xác minh email khi tạo tài khoản và khi đổi email

## 8. Lưu file

- [x] Trait chung; cài đặt ổ đĩa cục bộ và S3
- [x] Tải lên, tải xuống, xoá; metadata trong database
- [x] Giới hạn kích thước và loại file

## 9. MCP endpoint

Cho AI agent gọi chức năng của hệ thống như công cụ (Model Context Protocol).

- [x] Endpoint MCP qua HTTP, xác thực bằng API key hoặc access token
- [x] Module tự khai báo tool của mình qua hợp đồng module (như route và permission)
- [x] Mỗi tool gắn permission; chỉ liệt kê và cho gọi tool mà người gọi có permission
- [x] Tool gọi vào service layer (cùng logic với REST); lệnh ghi đi qua audit log
- [x] Tool của core: ít nhất đọc thông tin người gọi và danh mục permission
- [x] Test bằng một MCP client thật trên cả hai database

## 10. Module mẫu

- [x] Một module nghiệp vụ nhỏ dùng đủ: migration, permission, service, REST, OpenAPI,
      event, audit, MCP tool — làm khuôn cho module về sau
- [x] Hướng dẫn viết module (tiếng Anh + tiếng Việt)

## 11. Công cụ và deploy

- [x] Lệnh CLI chép dữ liệu giữa SQLite và Postgres
- [x] Bộ đo hiệu năng (đọc, list, ghi, ghi đồng thời) trên cả hai database, có số gốc để so về sau
- [x] Dockerfile multi-stage + `compose.yml` (ứng dụng + Postgres)
- [x] Backup và restore
- [x] CI: gate đầy đủ (đã gồm `cargo-deny`) trên Postgres tối thiểu và mới nhất; quét
      secret bằng gitleaks
- [x] Thử gỡ ngoại lệ RUSTSEC-2026-0459 trong `deny.toml` (D-032) — chưa gỡ được, xem D-075
- [x] Hướng dẫn cài đặt và deploy (tiếng Anh + tiếng Việt), đủ để người lạ tự làm theo

## 12. Passkey (mở sau core, chủ dự án chọn ngày 11/11/2026)

Đăng nhập bằng passkey (WebAuthn: vân tay, Face ID, khoá bảo mật) bên cạnh mật khẩu + TOTP.

- [x] Người dùng đã đăng nhập thêm, xem, xoá passkey của mình
- [x] Đăng nhập bằng passkey không cần email (discoverable credential), cấp token như đăng nhập thường
- [x] Trạng thái nghi thức lưu trong database (nhiều máy chủ chạy song song được), dùng một lần, có hạn
- [x] Cấu hình RP ID và origin; không cấu hình thì tính năng tắt, báo lỗi rõ
- [x] Test bằng authenticator phần mềm trên cả hai database, kể cả dùng đồng thời một nghi thức

## 13. Củng cố core (chủ dự án giao ngày 11/11/2026, agent quyết cách làm)

Kiểm lại phần đã có:

- [ ] CI chạy xanh trên GitHub
- [x] Rà soát bảo mật toàn repo bằng một lượt đọc độc lập; sửa mọi lỗi tìm thấy (D-079)
- [x] Thử tải dài hạn: không rò bộ nhớ, không rò kết nối database (docs/BENCHMARKS.md)

Hệ thống tài khoản:

- [x] Quản lý phiên đăng nhập: người dùng xem và đăng xuất từng thiết bị; quản trị viên thu hồi mọi phiên của một người (D-080)
- [x] Xoá tài khoản (event cho mọi module dọn dữ liệu của người đó) và xuất dữ liệu cá nhân (D-080)
- [x] Tìm người dùng theo email hoặc tên (D-080)

Phát hành và nâng cấp:

- [ ] Số phiên bản, `CHANGELOG`, bản phát hành trên GitHub
- [ ] CI build và đăng image Docker lên GitHub Container Registry
- [x] Dependabot đề xuất nâng cấp; kiểm lỗ hổng thư viện hằng ngày

Tích hợp:

- [x] Webhook: gửi event ra hệ thống bên ngoài, có chữ ký, thử lại, theo dõi được (D-081)

## 14. Hoàn thiện hợp đồng API (chủ dự án giao ngày 12/11/2026)

Sau lượt kiểm hợp đồng API của chủ dự án. Đã sửa ngay (D-082): `docs/openapi.json` không
hợp lệ (`$ref` tới `Order` không tồn tại), mô tả tool MCP lọt tiếng Việt; gate kiểm cả hai.

- [x] Kết quả tool MCP dạng danh sách không lồng `data` hai lần — cùng hình dạng với REST (D-083)
- [x] Tool MCP cho phần còn thiếu: file, audit log, role, API key (cùng service, cùng permission với REST) (D-083)
- [x] Khai header `Idempotency-Key` và `Accept-Language` trên từng endpoint trong OpenAPI, để SDK sinh tự động có sẵn tham số (D-083)
- [x] Kiểm dữ liệu báo mọi lỗi trong một lần (email sai và tên rỗng cùng báo), đúng như tài liệu hứa (D-083)
- [x] Tạo mới (201) trả header `Location`; OpenAPI có mục `servers` (D-083)
- [x] Chống ghi đè khi hai người cùng sửa: `ETag` / `If-Match` cho các endpoint sửa (D-084)

## 15. Thư viện media (chủ dự án giao ngày 12/11/2026)

Module `media`: ảnh, video, âm thanh, tài liệu trong thư mục lồng nhau, cho website (sắp tới là
module `content`) dùng lại. Điều kiện trước là phần nền chung ở kernel (D-085).

Nền chung (kernel):

- [x] Danh sách sắp theo cột khác (tên, kích thước) bằng keyset pagination; tìm theo chữ dùng chung (D-085)
- [x] Endpoint công khai (`Option<Principal>`); `If-None-Match` → 304; `If-Match` bắt buộc → 428 (D-085)
- [x] `Storage` đọc theo khoảng byte và liệt kê theo tiền tố, có cài đặt mặc định (D-085)

Module `media`:

- [x] Tải lên: kiểm kích thước, loại được phép, loại thật theo chữ ký đầu file; kích thước ảnh; SHA-256; báo item trùng (D-086, D-089)
- [x] Danh sách lọc theo thư mục, loại, `visibility`, thùng rác, tên; sắp theo thời gian, tên, kích thước
- [x] Sửa thông tin và thay nội dung giữ ID, bắt buộc `If-Match`
- [x] Tải nội dung: công khai hay cần quyền; `inline` chỉ cho ảnh raster; `Range`, `HEAD`, 304; cache theo phiên bản (D-086, D-087)
- [x] Thư mục lồng nhau: tạo, đổi tên, chuyển, xoá khi trống (D-088)
- [x] Thùng rác: vào, khôi phục, xoá hẳn (permission riêng); job xoá quá hạn và dọn nội dung mồ côi
- [x] Event cho mọi thay đổi (vào audit log); tool MCP; trait `Media` cho module khác (D-090)
- [x] Backup / restore mang theo `media`; hướng dẫn dùng (tiếng Anh + tiếng Việt)

## Sau core (chưa mở)

GraphQL · tách service — xem bảng "Chưa làm, đã cân nhắc" trong `STACK.md`.
