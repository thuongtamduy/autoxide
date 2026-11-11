# Lộ trình

Dựng theo thứ tự. Mỗi bước xong khi `./gate.sh` xanh trên **cả SQLite và
Postgres**; agent đánh dấu `[x]` rồi commit. Chi tiết thư viện: [`STACK.md`](../STACK.md).

Trạng thái: **bước 0 xong; đang ở bước 1**, mục "CLI: `serve`, `migrate`" (còn `serve`).

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
- [ ] CLI: `serve`, `migrate` (`migrate up | down | status` đã có; còn `serve`, và nó phải
      từ chối chạy khi còn migration chưa áp dụng)

## 2. Đăng nhập, role và permission

- [ ] Người dùng, hash mật khẩu Argon2id
- [ ] Đăng nhập: access token sống ngắn + refresh token rotation, phát hiện reuse
- [ ] Role và permission; module tự khai báo permission của mình
- [ ] Rate limiting; login throttling
- [ ] CLI tạo người dùng đầu tiên
- [ ] Tạo người dùng: quản trị viên tạo; tự đăng ký bật tắt bằng cấu hình (mặc định tắt)
- [ ] API key cho máy và AI agent: tạo, thu hồi, giới hạn theo permission

## 3. MFA

- [ ] TOTP: bật, xác nhận, tắt
- [ ] Recovery code dùng một lần
- [ ] Đăng nhập hai bước khi đã bật MFA

## 4. Event và audit log

- [ ] Event ghi cùng transaction rồi mới phát
- [ ] Audit log chỉ thêm: ai, làm gì, bản ghi nào, lúc nào, trước / sau
- [ ] API xem audit log (có permission riêng)

## 5. Job queue

- [ ] Job lưu trong database; claim job an toàn khi nhiều worker (test đồng thời)
- [ ] Retry có backoff, dead-letter queue
- [ ] Lịch chạy định kỳ
- [ ] Process role: `api`, `worker`, hoặc cả hai

## 6. i18n và idempotency

- [ ] Thông báo lỗi và hệ thống theo i18n key (vi, en), chọn theo header; mặc định tiếng Anh
      (`code` của lỗi là i18n key, câu tiếng Anh mặc định đã nằm ở chỗ tạo lỗi — D-039;
      việc còn lại: bản dịch vi tra theo `code`, chọn ngôn ngữ theo header, và gate kiểm mọi
      `code` đều có bản dịch)
- [ ] Idempotency key cho lệnh ghi (test gửi trùng đồng thời)
- [ ] Đánh dấu endpoint deprecated

## 7. Email

Đặt sau job queue vì mail gửi qua job để có retry.

- [ ] Gửi mail qua SMTP sau một trait; khi chưa cấu hình SMTP thì ghi mail ra file (cho dev và test)
- [ ] Mẫu mail theo i18n key (vi, en)
- [ ] Đặt lại mật khẩu: token dùng một lần, có hạn, không lộ việc email có tồn tại hay không
- [ ] Xác minh email khi tạo tài khoản và khi đổi email

## 8. Lưu file

- [ ] Trait chung; cài đặt ổ đĩa cục bộ và S3
- [ ] Tải lên, tải xuống, xoá; metadata trong database
- [ ] Giới hạn kích thước và loại file

## 9. MCP endpoint

Cho AI agent gọi chức năng của hệ thống như công cụ (Model Context Protocol).

- [ ] Endpoint MCP qua HTTP, xác thực bằng API key hoặc access token
- [ ] Module tự khai báo tool của mình qua hợp đồng module (như route và permission)
- [ ] Mỗi tool gắn permission; chỉ liệt kê và cho gọi tool mà người gọi có permission
- [ ] Tool gọi vào service layer (cùng logic với REST); lệnh ghi đi qua audit log
- [ ] Tool của core: ít nhất đọc thông tin người gọi và danh mục permission
- [ ] Test bằng một MCP client thật trên cả hai database

## 10. Module mẫu

- [ ] Một module nghiệp vụ nhỏ dùng đủ: migration, permission, service, REST, OpenAPI,
      event, audit, MCP tool — làm khuôn cho module về sau
- [ ] Hướng dẫn viết module (tiếng Anh + tiếng Việt)

## 11. Công cụ và deploy

- [ ] Lệnh CLI chép dữ liệu giữa SQLite và Postgres
- [ ] Bộ đo hiệu năng (đọc, list, ghi, ghi đồng thời) trên cả hai database, có số gốc để so về sau
- [ ] Dockerfile multi-stage + `compose.yml` (ứng dụng + Postgres)
- [ ] Backup và restore
- [ ] CI: gate đầy đủ (đã gồm `cargo-deny`) trên Postgres tối thiểu và mới nhất; quét
      secret bằng gitleaks
- [ ] Thử gỡ ngoại lệ RUSTSEC-2026-0459 trong `deny.toml` (D-032)
- [ ] Hướng dẫn cài đặt và deploy (tiếng Anh + tiếng Việt), đủ để người lạ tự làm theo

## Sau core (chưa mở)

GraphQL · passkey · tách service — xem bảng "Chưa làm, đã cân nhắc" trong `STACK.md`.
