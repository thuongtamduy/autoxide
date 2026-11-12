# Autoxide — Stack

Autoxide là một **lõi backend viết bằng Rust**: dựng core trước, module nghiệp vụ
cắm vào sau. Dự án do AI agent quyết định và thực hiện xuyên suốt (luật cho agent:
[`CLAUDE.md`](CLAUDE.md)).

File này ghi **dùng cái gì và vì sao**. Lý do đầy đủ của từng lựa chọn nằm ở
[`docs/DECISIONS.md`](docs/DECISIONS.md); thứ tự dựng ở
[`docs/ROADMAP.md`](docs/ROADMAP.md).

## Mục tiêu

Một lõi backend bằng Rust **tiêu chuẩn, đầy đủ, nhanh, tối ưu, dễ phát triển, hiện
đại, phù hợp với AI và AI agent**; dễ nâng cấp, dễ bảo trì; chạy được trên **cả
SQLite và Postgres**; có REST + OpenAPI và MCP endpoint. Core dựng trước, module
nghiệp vụ cắm vào sau. Core xong khi mọi bước trong lộ trình xanh
trên cả hai database.

Autoxide là nền cho một **sản phẩm mới**, **mã nguồn mở**, ai cũng dùng được, không
gắn với một công ty hay một ngành. Module đầu tiên là `content` (nội dung, D-085, D-087); core vẫn trung lập với nghiệp vụ.

## Tóm tắt

| Hạng mục     | Lựa chọn                                                             |
| ------------ | -------------------------------------------------------------------- |
| Ngôn ngữ     | Rust (edition 2024), toolchain ghim trong `rust-toolchain.toml`      |
| Kiến trúc    | Một binary, mỗi module một crate, ranh giới chặt, tách service sau   |
| API          | REST dưới `/api/v1` + OpenAPI; MCP endpoint cho AI agent. GraphQL thêm sau |
| Database     | SQLite và Postgres, đổi bằng `DATABASE_URL`. Không cam kết MySQL     |
| ID           | UUIDv7 cho mọi bảng, sinh ở phía ứng dụng                            |
| Multi-tenant | Không có. Nhiều khách = mỗi khách một bản cài riêng                  |
| Deploy       | Docker trên VPS                                                      |
| License      | MIT hoặc Apache-2.0, người dùng chọn (`LICENSE-MIT`, `LICENSE-APACHE`) |

## Thư viện

Rust không có sẵn HTTP server, async runtime hay driver database trong thư viện
chuẩn — ghép thư viện là cách làm bình thường. Phiên bản dưới đây là bản ổn định
mới nhất ngày 11/11/2026; **`Cargo.lock` là nguồn sự thật** sau khi dựng.

### Nền

| Việc               | Thư viện                        | Phiên bản | Ghi chú                                             |
| ------------------ | ------------------------------- | --------- | --------------------------------------------------- |
| Async runtime      | `tokio`                         | 1.54      | Chuẩn thực tế của Rust async                        |
| HTTP / REST        | `axum`                          | 0.8       | Do nhóm tokio làm                                   |
| Middleware HTTP    | `tower-http`                    | 0.7       | Trace, CORS, timeout, giới hạn body, request id     |
| ORM                | `sea-orm`                       | 2.0       | Một bộ code cho SQLite và Postgres                  |
| Migration          | `sea-orm-migration`             | 2.0       | Viết bằng Rust, chạy được trên cả hai database      |
| OpenAPI            | `utoipa` + `utoipa-axum`        | 6.0 / 0.3 | Route và tài liệu đăng ký cùng một lệnh, không lệch |
| Trang xem API      | `utoipa-scalar`                 | 0.4       | Tắt mặc định; bật bằng `AUTOXIDE_HTTP_API_DOCS=true` |
| JSON               | `serde` + `serde_json`          | 1.0       |                                                     |
| Validate đầu vào   | `validator`                     | 0.21      |                                                     |
| ID                 | `uuid` (v7)                     | 1.29      |                                                     |
| Thời gian          | `chrono`                        | 0.4       | Lưu UTC                                             |
| CLI                | `clap`                          | 4.6       | `serve`, `migrate`, `user`, `db copy`…              |
| Đọc file `.env`    | `dotenvy`                       | 0.15      | Chỉ đọc thành cặp tên / giá trị; biến môi trường thật thắng |
| Trait `async` qua `dyn` | `async-trait`                | 0.1       | Hợp đồng giữa các module (`api`); đã có sẵn trong cây phụ thuộc qua SeaORM |
| Gọi router trong test | `tower` (feature `util`)     | 0.5       | Chỉ dùng trong test: gửi request thẳng vào router   |

Kết nối database dùng TLS của `rustls` (feature `runtime-tokio-rustls` của SeaORM):
binary không phụ thuộc OpenSSL của hệ điều hành.

### Bảo mật

| Việc               | Thư viện          | Phiên bản | Ghi chú                                         |
| ------------------ | ----------------- | --------- | ----------------------------------------------- |
| Hash mật khẩu       | `argon2`          | 0.6       | Argon2id                                        |
| Access token       | JWT HS256 tự viết trên `hmac` + `sha2` + `base64` | 0.13 / 0.11 / 0.23 | Sống ngắn; refresh token rotation lưu database. Không dùng `jsonwebtoken` (D-045) |
| Số ngẫu nhiên      | `getrandom`       | 0.4       | Refresh token, khoá ký tạm thời                 |
| MFA                | `totp-rs`         | 6.0       | TOTP + recovery code                             |
| Passkey            | `webauthn_rp`     | 0.3       | WebAuthn phía máy chủ, thuần Rust (RustCrypto: p256, p384, ed25519, rsa — chỉ kiểm chữ ký bằng khoá công khai), không OpenSSL. Test dùng `p256` làm authenticator phần mềm (D-077) |
| Rate limiting  | Tự viết (GCRA, ~100 dòng) | —   | Theo địa chỉ client, trong bộ nhớ; login throttling đếm trong database. Không dùng `tower_governor` (D-047) |

### Dịch vụ của core

| Việc           | Cách làm                                                                    |
| -------------- | --------------------------------------------------------------------------- |
| Job queue   | Tự viết, lưu trong database (không Redis): retry, lịch định kỳ, dead-letter queue   |
| Event        | Tự viết: ghi vào database cùng transaction với thay đổi rồi mới phát        |
| Audit log      | Tự viết: bảng chỉ thêm — ai, làm gì, bản ghi nào, lúc nào, trước / sau      |
| Lưu file       | `object_store` 0.14 (feature `aws-base` + `ring`, không `aws` để khỏi kéo aws-lc-rs) — một trait chung cho ổ đĩa cục bộ và S3 / R2 / dịch vụ tương thích S3, sau trait `Storage` của kernel. Kèm `reqwest` 0.13 (`rustls-no-provider`) và `rustls` 0.23 (`ring`) chỉ để chọn TLS; `bytes`, `futures-util`, `http-body-util` cho luồng byte (đều đã có sẵn qua axum). Test S3 chạy trên máy chủ giả lập `moto` trong Docker (D-066) |
| i18n    | Tự viết: `locales/<ngôn ngữ>.json` mỗi crate, dịch theo `code` lỗi ở lớp HTTP, chọn theo `Accept-Language`; không dùng `rust-i18n` (D-057) |
| Idempotency    | Tự viết: key do client gửi, lưu kết quả lệnh ghi để gửi lại không tạo trùng |
| MCP endpoint   | `rmcp` 3.9 (SDK chính thức của Model Context Protocol) — module khai báo tool, gắn permission |
| Email          | `lettre` 0.11 (rustls + ring, không native-tls) — SMTP sau trait `Mailer` của kernel, gửi qua job queue; chưa cấu hình SMTP thì ghi file `.eml` (D-061) |
| Webhook        | Module `webhooks`: chữ ký theo chuẩn Standard Webhooks (`hmac` + `sha2` + `base64` sẵn có), gửi bằng `reqwest` 0.13 sẵn có qua job queue; không thêm thư viện (D-081) |
| Dữ liệu cá nhân | Tự viết: trait `PersonalData` của kernel để xuất, event `auth.user_deleted` để mọi module xoá (D-080) |

### Observability

| Việc     | Thư viện                                              | Phiên bản     | Ghi chú                                         |
| -------- | ----------------------------------------------------- | ------------- | ----------------------------------------------- |
| Log      | `tracing` + `tracing-subscriber`                      | 0.1 / 0.3     | Ra stderr; dạng chữ hoặc JSON (`AUTOXIDE_LOG_FORMAT`) |
| Metrics  | `metrics` + `metrics-exporter-prometheus`             | 0.24 / 0.18   | Endpoint `/metrics`, bật bằng `AUTOXIDE_HTTP_METRICS` |
| Tracing  | `opentelemetry` + `opentelemetry_sdk` + `opentelemetry-otlp` + `tracing-opentelemetry` | 0.33 / 0.34 | Xuất OTLP qua HTTP; tắt khi không đặt `OTEL_EXPORTER_OTLP_ENDPOINT` |

Thư viện ở ba bảng sau (bảo mật, dịch vụ của core, observability) được **chốt lại khi tới bước
dựng tương ứng**: agent kiểm bản mới nhất, độ tương thích với phần đã dựng, rồi
ghi quyết định vào `docs/DECISIONS.md` nếu đổi.

## Database

- Phiên bản tối thiểu: **Postgres 16**. SQLite dùng bản đóng gói kèm binary, người
  cài không phải tự lo.
- **SQLite** cho dev, test và bản cài nhỏ một máy. **Postgres** cho production có
  nhiều người ghi đồng thời.
- Không dùng tính năng chỉ một bên có. Cả hai đều có partial index, `RETURNING`
  và migration trong transaction — được dùng.
- **SQLite là single-writer** (chỉ một lệnh ghi tại một thời điểm). Core tách kết nối ghi (đúng 1 kết nối trên
  SQLite) và kết nối đọc (nhiều), bật WAL, giữ transaction ghi ngắn. Trên
  Postgres cùng cách gọi đó trỏ vào một pool, sau này phía đọc trỏ được sang read
  replica.
- Có lệnh CLI chép toàn bộ dữ liệu giữa hai database, để bắt đầu bằng SQLite
  không thành ngõ cụt.
- CI chạy test trên **cả hai** database, gồm test ghi đồng thời.

## Quy ước dữ liệu

- **ID:** UUIDv7, sinh trong ứng dụng. Thứ tự ID = thứ tự tạo, dùng cho pagination.
- **Tiền:** số nguyên theo đơn vị nhỏ nhất, kèm cột mã tiền tệ (VND không có phần
  lẻ, USD 2 chữ số). Không dùng số thực, không dùng kiểu thập phân của database.
- **Thời gian:** lưu UTC. Hệ thống có một múi giờ cấu hình để hiển thị và tính ngày.

## Kiến trúc

Một binary. Mỗi module là một crate trong cùng workspace và tuân các luật ranh
giới sau, để sau này tách module nào thành service riêng chỉ là thêm network layer:

Bố cục thư mục và crate: [`CLAUDE.md`](CLAUDE.md), mục "Bố cục workspace".

1. Module sở hữu bảng của mình; không module nào đọc, ghi hay join bảng của
   module khác.
2. Module gọi nhau qua trait do module kia công bố.
3. Việc phụ (audit, thông báo) đi qua event, không gọi thẳng.
4. Không foreign key xuyên module — chỉ giữ ID.
5. Phụ thuộc một chiều; `cargo xtask check` (chạy trong gate và CI) kiểm chiều phụ
   thuộc và việc import vào phần bên trong của module khác.

Logic nằm ở service layer. Handler REST chỉ là wrapper mỏng — nhờ vậy thêm GraphQL
sau này không phải sửa code cũ.

## Giao diện

Core chỉ có API (REST, MCP) và CLI. Không có giao diện web hay trang quản trị trong
repo này; giao diện là dự án riêng dùng API.

## Chưa làm, đã cân nhắc

| Thứ            | Trạng thái                                                              |
| -------------- | ----------------------------------------------------------------------- |
| GraphQL        | Thêm sau bằng `async-graphql`; mỗi module thêm một file resolver        |
| Microservice   | Tách sau khi có lý do cụ thể (tải khác biệt, nhiều đội)                 |
| MySQL          | SeaORM vẫn chạy được, nhưng không test và không cam kết                 |
| MongoDB        | Ngoài phạm vi: không ORM SQL nào phủ được                               |
| Multi-tenant   | Ngoài phạm vi core                                                      |

## Công cụ

| Việc                | Công cụ                                                         |
| ------------------- | --------------------------------------------------------------- |
| Toolchain           | `rustup` — phiên bản ghim trong `rust-toolchain.toml`           |
| Định dạng           | `cargo fmt` (cấu hình `rustfmt.toml`)                           |
| Lint                | `cargo clippy`, cảnh báo tính là lỗi                            |
| Test                | `cargo test` trên SQLite và Postgres                            |
| Database cho test   | `autoxide-testkit` — mỗi test một database trống (D-033)        |
| Kiểm ranh giới      | `cargo xtask check` — bố cục, license, lint, chiều phụ thuộc, bề mặt module (D-031) |
| Kiểm phụ thuộc      | `cargo-deny` (license, lỗ hổng đã công bố, nguồn) — cấu hình `deny.toml`, chạy trong gate và CI |
| Postgres cho test   | Docker — `gate.sh` tự dựng một container dùng xong bỏ           |
| Gate                | `./gate.sh` — chạy tất cả các mục trên theo thứ tự              |

## Deploy

Docker trên VPS: image multi-stage (stage build có toolchain, stage chạy chỉ có một
file binary), `compose.yml` gồm ứng dụng và Postgres, kèm backup và restore.
So với Deno: chạy đơn giản hơn (không cần runtime, image nhỏ), build lâu hơn (CI
cần cache).
