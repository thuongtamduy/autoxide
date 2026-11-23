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
gắn với một công ty hay một ngành. Module nghiệp vụ chủ dự án đã chọn: `media` (thư viện media, D-091) và `content` (nội dung, D-096, D-098); core vẫn trung lập với nghiệp vụ.

## Tóm tắt

| Hạng mục     | Lựa chọn                                                             |
| ------------ | -------------------------------------------------------------------- |
| Ngôn ngữ     | Rust (edition 2024), toolchain ghim trong `rust-toolchain.toml`      |
| Kiến trúc    | Một binary, mỗi module một crate, ranh giới chặt, tách service sau   |
| API          | REST dưới `/api/v1` + OpenAPI; MCP endpoint cho AI agent. GraphQL thêm sau |
| Database     | SQLite và Postgres, đổi bằng `DATABASE_URL`. Không cam kết MySQL     |
| ID           | UUIDv7 cho mọi bảng, sinh ở phía ứng dụng                            |
| Multi-tenant | Có (D-185): mặt điều khiển + `tenant_id`; dữ liệu chung hay riêng theo khách |
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
| WebSocket          | `axum` feature `ws` (`tokio-tungstenite` 0.29) | 0.8 | `GET /api/v1/kernel/events/ws` (bước 69); test dùng client `tokio-tungstenite` cùng bản |
| Middleware HTTP    | `tower-http`                    | 0.7       | Trace, CORS, timeout, giới hạn body, request id, nén response (gzip, brotli — D-20261119-step39-compression) |
| ORM                | `sea-orm`                       | 2.0       | Một bộ code cho SQLite và Postgres                  |
| Migration          | Kernel tự chạy (`autoxide_kernel::migration`: `MigrationTrait`, `SchemaManager`, `schema::*` — cùng tên `sea-orm-migration` cũ) | — | Viết bằng Rust, chạy được trên cả hai database; mỗi migration một transaction; thay `sea-orm-migration` (bật bộ feature mặc định của SeaORM, kéo `chrono`; D-20261121-own-migrations) |
| OpenAPI            | `utoipa` + `utoipa-axum`        | 6.0 / 0.3 | Route và tài liệu đăng ký cùng một lệnh, không lệch |
| Trang xem API      | `utoipa-scalar`                 | 0.4       | Tắt mặc định; bật bằng `AUTOXIDE_HTTP_API_DOCS=true` |
| JSON               | `serde` + `serde_json`          | 1.0       |                                                     |
| Validate đầu vào   | `validator`                     | 0.21      |                                                     |
| ID                 | `uuid` (v7)                     | 1.29      |                                                     |
| Thời gian          | `jiff` qua `autoxide_kernel::time` (`Timestamp`, `Date`) | 1.0 | Lưu UTC; dạng lưu, JSON, OpenAPI như `chrono` trước đây (giá trị đối chiếu ở `crates/kernel/src/values/time/golden.json`). `chrono` / `chrono-tz` đã gỡ khỏi cây phụ thuộc (D-20261121-time-library) |
| Số thập phân       | `rust_decimal` (chỉ `std`)      | 1.43      | Chỉ để TÍNH tiền, thuế, tỉ giá; lưu tiền vẫn là số nguyên `Money`, lưu số thập phân dạng chữ (D-322). Đã có trong cây phụ thuộc qua SeaORM |
| Múi giờ IANA       | `jiff::tz` (cơ sở dữ liệu múi giờ nhúng trong binary) | 1.0 | Kiểm tên múi giờ của nút (`organizations.timezone`, D-306) qua `autoxide_kernel::time::zone`. Đã thay `chrono-tz` (D-20261121-time-library) |
| CLI                | `clap`                          | 4.6       | `serve`, `migrate`, `user`, `db copy`…              |
| Đọc file `.env`    | `dotenvy`                       | 0.15      | Chỉ đọc thành cặp tên / giá trị; biến môi trường thật thắng |
| Trait `async` qua `dyn` | `async-trait`                | 0.1       | Hợp đồng giữa các module (`api`); đã có sẵn trong cây phụ thuộc qua SeaORM |
| Gọi router            | `tower` (feature `util`)     | 0.5       | Gửi request thẳng vào router: test, và lô ghi đồng bộ của kernel chạy từng lệnh qua router con (bước 69) |
| Property test      | `proptest` (chỉ `std`)          | 1.12      | Chỉ dùng trong test: sinh đầu vào ngẫu nhiên cho bộ kiểm đầu vào (email, con trỏ, thân bài, slug, CSV, bình luận), thu gọn ca hỏng (đặc tả 31 B3) |

Kết nối database dùng TLS của `rustls` (feature `runtime-tokio-rustls` của SeaORM):
binary không phụ thuộc OpenSSL của hệ điều hành.
`sqlx` (nền của SeaORM, phụ thuộc gián tiếp) ≥ 0.9.1: SQLite đóng gói kèm từ `libsqlite3-sys` 0.40,
đã vá RUSTSEC-2026-0459 (D-20261121-sqlx-091).

### Bảo mật

| Việc               | Thư viện          | Phiên bản | Ghi chú                                         |
| ------------------ | ----------------- | --------- | ----------------------------------------------- |
| Hash mật khẩu       | `argon2`          | 0.6       | Argon2id                                        |
| Access token       | JWT HS256 tự viết trên `hmac` + `sha2` + `base64` | 0.13 / 0.11 / 0.23 | Sống ngắn; refresh token rotation lưu database. Không dùng `jsonwebtoken` (D-045) |
| Số ngẫu nhiên      | `getrandom`       | 0.4       | Refresh token, khoá ký tạm thời                 |
| Mã hoá khi lưu     | `ring` (AEAD AES-256-GCM), `zeroize` | 0.17, 1 | `Sealer` của kernel: secret cần đọc lại lưu mã hoá, khoá ngoài database (D-257). Cả hai đã có trong cây phụ thuộc |
| MFA                | `totp-rs`         | 6.0       | TOTP + recovery code                             |
| Passkey            | `webauthn_rp`     | 0.3       | WebAuthn phía máy chủ, thuần Rust (RustCrypto: p256, p384, ed25519, rsa — chỉ kiểm chữ ký bằng khoá công khai), không OpenSSL. Test dùng `p256` làm authenticator phần mềm (D-077) |
| Đăng nhập OIDC / OAuth 2.0 | Tự viết trên `ring` (RS256), `p256` (ES256), `reqwest` sẵn có | 0.17 / 0.13 / 0.13 | Module `auth` là bên dùng của Google, Microsoft, GitHub, issuer OIDC bất kỳ: PKCE, `state` / `nonce`, kiểm `id_token` qua JWKS ~250 dòng. `ring` và `p256` thành phụ thuộc trực tiếp của `auth` (đã có trong cây); không dùng `openidconnect` / `jsonwebtoken` / `rsa` (D-077, D-380). Gọi ra ngoài qua bộ lọc SSRF `autoxide_kernel::net` (dời từ `webhooks`) |
| Rate limiting  | Tự viết (GCRA, ~100 dòng) | —   | Theo địa chỉ client, trong bộ nhớ; login throttling đếm trong database. Không dùng `tower_governor` (D-047) |

### Dịch vụ của core

| Việc           | Cách làm                                                                    |
| -------------- | --------------------------------------------------------------------------- |
| Job queue   | Tự viết, lưu trong database (không Redis): retry, lịch định kỳ, dead-letter queue   |
| Event        | Tự viết: ghi vào database cùng transaction với thay đổi rồi mới phát        |
| Audit log      | Tự viết: bảng chỉ thêm — ai, làm gì, bản ghi nào, lúc nào, trước / sau      |
| Lưu file       | Ổ đĩa tự viết trên `tokio::fs` (ghi file tạm, `fsync`, đổi tên); S3 / R2 / dịch vụ tương thích S3 qua `rusty-s3` 0.12 (chỉ dựng và ký request SigV4, gửi bằng `reqwest` 0.13 `rustls-no-provider` + `stream`; credential tạm của IAM role qua IMDSv2 tự viết) — sau trait `Storage` của kernel; thay `object_store` (kéo `humantime` bị bỏ hoang, D-20261121-s3-client). `rustls` 0.23 (`ring`); `bytes`, `futures-util`, `http-body-util` cho luồng byte (đều đã có sẵn qua axum). Test S3 chạy trên máy chủ giả lập `moto` (D-066) |
| Kích thước ảnh | `imagesize` 0.15 (MIT, không phụ thuộc gì; chỉ bật `jpeg`, `png`, `gif`, `webp`, `heif`) — đọc chiều rộng / cao từ header ảnh, không giải mã ảnh; module `media` dùng. Nhận loại thật theo chữ ký đầu file thì tự viết (D-089) |
| Ảnh thu nhỏ | `image` 0.25 (MIT / Apache-2.0, thuần Rust; chỉ bật `jpeg`, `png`, `gif`, `webp`, không `rayon`) — giải mã, xoay theo EXIF, đổi cỡ, mã hoá JPEG / PNG cho ảnh thu nhỏ của module `media`, chạy trong job nền (D-093) |
| Ảnh thu nhỏ WebP | `webp` 0.3 (MIT / Apache-2.0; không bật feature `img`) bọc `libwebp` qua `libwebp-sys` (crate MIT, mã C của libwebp BSD-3-Clause, biên dịch kèm bằng trình biên dịch C như SQLite) — mã hoá WebP nén mất dữ liệu, có kênh trong suốt, cho bản WebP bên cạnh bản JPEG / PNG (D-176, D-196). `image` chỉ mã hoá WebP không mất dữ liệu |
| Ảnh thu nhỏ AVIF | `ravif` 0.13 (BSD-3-Clause) trên `rav1e` 0.8 (BSD-2-Clause), Rust thuần — không bật `asm` (cần `nasm` lúc build) hay `threading` — mã hoá bản AVIF của ảnh thu nhỏ bằng job nền, tắt mặc định (`AUTOXIDE_MEDIA_AVIF`, D-20261119-step39-avif). Chỉ có trong bản build bật feature cargo `avif` (`rav1e` kéo proc-macro `paste` ngừng bảo trì; D-20261121-avif-feature). Vì sao: cùng chất lượng, nhẹ hơn bản WebP sẵn có ~34% (đo D-232); WebP vẫn là mặc định. Chi phí: build sạch +5–8%, binary +1% |
| Tìm không dấu | `unicode-normalization` 0.1 (MIT / Apache-2.0; đã có sẵn trong cây phụ thuộc qua `webauthn_rp`) — tách chữ khỏi dấu (NFD) để `search::fold` của kernel bỏ dấu: "da lat" tìm ra "Đà Lạt" (D-104) |
| Slug | `slug` 0.1 (MIT / Apache-2.0; kéo theo `deunicode`) — tạo slug từ tiêu đề: bỏ dấu, `đ` → `d`, chữ thường nối bằng `-` ("Đường đến Đà Lạt" → `duong-den-da-lat`); module `content` dùng (D-102, D-105) |
| Markdown của bình luận | `pulldown-cmark` 0.13 (MIT; tắt feature mặc định `html`, `getopts`; kéo `bitflags`, `memchr`, `unicase`) — chỉ bộ phân tích; HTML do module `comments` tự dựng theo danh sách thẻ cho phép, không thêm bộ lọc HTML như `ammonia` (D-252) |
| Mẫu do khách soạn | `minijinja` 3.1 (Apache-2.0; tắt feature mặc định, bật `builtins`, `debug`, `serde`, `fuel`; không phụ thuộc thêm crate nào ngoài `serde`) — MỘT ngôn ngữ mẫu (cú pháp Jinja) cho cả dự án: bộ dựng hộp cát `autoxide_kernel::template` (không include / extends / loader, không bộ lọc `safe`, giới hạn độ dài / bước chạy / độ sâu / kết quả, thoát HTML mặc định, kiểm biến theo schema). Mẫu in chứng từ dùng; mẫu mail, thông báo (bước 37) sẽ dùng (D-351) |
| Khung mail HTML | `mrml` 6.0 (MIT; tắt feature mặc định, chỉ `parse` + `render`: không loader — `mj-include` không đọc file hay mạng —, không `json` / `print`) — MJML viết bằng Rust: khung mail có thương hiệu của `notifications` chạy đúng trên Outlook, Gmail, điện thoại; nội dung của khách không qua bộ phân tích MJML (D-366) |
| PDF mẫu in | `krilla` 0.8 (MIT / Apache-2.0; không bật feature nào — không ảnh raster, không `simple-text` vì kéo `rustybuzz` / `ttf-parser` đã ngừng bảo trì; nhúng tập con font qua `subsetter`; kéo `arrayref` BSD-2-Clause qua `tiny-skia-path`) + `skrifa` 0.42 (MIT / Apache-2.0, đã có qua `krilla`) để tra glyph và đo bề rộng chữ (mỗi ký tự một glyph, không kerning). Thuần Rust, không cần trình duyệt; bố cục PDF cố định dựng từ view model, font Liberation Sans 2.1.5 (OFL 1.1, đủ dấu tiếng Việt, nhúng nguyên bản trong `modules/invoices/fonts/`). Không dựng HTML → PDF (D-353) |
| Excel | `rust_xlsxwriter` 0.102 (MIT / Apache-2.0; không bật feature nào; kéo `zip` có `deflate`) — file `.xlsx` của bản xuất kế toán; CSV tự viết (D-354) |
| i18n    | Tự viết: `locales/<ngôn ngữ>.json` mỗi crate, dịch theo `code` lỗi ở lớp HTTP, chọn theo `Accept-Language`; không dùng `rust-i18n` (D-057) |
| Idempotency    | Tự viết: key do client gửi, lưu kết quả lệnh ghi để gửi lại không tạo trùng |
| MCP endpoint   | Tự viết trong kernel theo đặc tả công khai (`http/mcp/`: JSON-RPC 2.0 + Streamable HTTP, bản `2026-07-28` và các bản cũ còn `initialize`); thay `rmcp`, vốn bắt buộc `chrono` (D-20261121-mcp-server) — module khai báo tool, gắn permission. Client cho test: `autoxide_testkit::mcp` |
| Email          | `lettre` 0.11 (rustls + ring, không native-tls) — SMTP sau trait `Mailer` của kernel, gửi qua job queue; chưa cấu hình SMTP thì ghi file `.eml` (D-061) |
| Tra DNS (TXT) | `hickory-resolver` 0.26 (MIT / Apache-2.0; tắt feature mặc định, chỉ bật `tokio` và `system-config`; không DNS-over-TLS / HTTPS / QUIC, không DNSSEC) — tra bản ghi TXT `_autoxide-challenge.<domain>` khi xác minh tên miền riêng của khách, sau trait `DnsTxtLookup` của kernel; đọc cấu hình DNS của hệ điều hành (`/etc/resolv.conf`); test dùng `FakeDns` của testkit. Thư viện chuẩn của Rust không tra được TXT (D-190) |
| Webhook        | Module `webhooks`: chữ ký theo chuẩn Standard Webhooks (`hmac` + `sha2` + `base64` sẵn có), gửi bằng `reqwest` 0.13 sẵn có qua job queue; không thêm thư viện (D-081) |
| AI | Module `ai` (D-109): trait `TextModel` / `Embedder` ở kernel; hai adapter sinh chữ tự viết trên `reqwest` sẵn có — `anthropic` (Messages API) và `openai` (Chat Completions: OpenAI, Gemini, Ollama, vLLM…) — và client embedding dạng `/embeddings`; kết quả luôn theo JSON Schema và được kiểm lại; hạn mức token mỗi ngày trong database; mặc định tắt. Không thêm thư viện |
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
- Gate (`./gate.sh`) chạy test trên **cả hai** database, gồm test ghi đồng thời.

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
5. Phụ thuộc một chiều; `cargo xtask check` (chạy trong gate) kiểm chiều phụ
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

## Công cụ

| Việc                | Công cụ                                                         |
| ------------------- | --------------------------------------------------------------- |
| Toolchain           | `rustup` — phiên bản ghim trong `rust-toolchain.toml`           |
| Định dạng           | `cargo fmt` (cấu hình `rustfmt.toml`)                           |
| Lint                | `cargo clippy`, cảnh báo tính là lỗi                            |
| Test                | `cargo test` trên SQLite và Postgres                            |
| Chạy test trên CI   | `cargo-nextest` (`cargo install cargo-nextest --locked`; CI cài bằng `taiki-e/install-action`) — build test một lần thành archive, chạy ở nhiều máy (`./gate.sh --only build`, `--archive`); gate trên máy không cần nó (D-20261120-ci-nextest-archive) |
| Cache biên dịch trên CI | `sccache` qua `mozilla-actions/sccache-action` (cache GitHub Actions) — chỉ job build tests của `gate.yml`; nhanh hơn 37% khi đổi một crate lá, lint không dùng vì chậm hơn (D-20261121-ci-sccache-build) |
| Cache lớp Docker      | `cargo-chef` 0.1.78 (cài trong Dockerfile bằng `cargo install --locked`, không cần cài ở máy dev) — tách build thư viện ngoài khỏi build code của mình để cache gha giữ được lớp thư viện (D-20261120-docker-chef) |
| Database cho test   | `autoxide-testkit` — mỗi test một database trống (D-033)        |
| Kiểm ranh giới      | `cargo xtask check` — bố cục, license, lint, chiều phụ thuộc, bề mặt module (D-031) |
| Module ở repo riêng | `cargo xtask check-module <thư mục>` — các luật đọc mã nguồn module của `check`, trên crate ngoài repo; ví dụ chạy thật ở `examples/external-module` (workspace Cargo riêng, `./gate.sh --only example`); `cargo xtask publish-check` — đóng gói và build thử mọi crate như trên crates.io, lúc phát hành (D-20261121-external-modules) |
| Kiểm phụ thuộc      | `cargo-deny` (license, lỗ hổng đã công bố, nguồn) — cấu hình `deny.toml`, chạy trong gate |
| Postgres cho test   | Docker — `gate.sh` tự dựng một container dùng xong bỏ           |
| Hợp đồng Rust       | `cargo-semver-checks` 0.51.0 (`cargo install cargo-semver-checks --version 0.51.0 --locked`) — `cargo xtask semver` báo thay đổi phá vỡ của kernel, testkit, `autoxide` và module core / official ở job CI `semver` (PR chạm phần công khai và PR phát hành), không trong gate; từ 1.0 thì chặn (D-242, D-20261123-compatibility-policy) |
| Quét secret         | `gitleaks` (cài sẵn hoặc image Docker ghim) — cấu hình `.gitleaks.toml`, chạy trong gate (D-108) |
| Gate                | `./gate.sh` — chạy tất cả các mục trên theo thứ tự; `--quick` chỉ test crate bị sửa trên SQLite (D-173) |

## Deploy

Docker trên VPS: image multi-stage (stage build có toolchain, stage chạy chỉ có một
file binary), `compose.yml` gồm ứng dụng và Postgres, kèm backup và restore.
So với Deno: chạy đơn giản hơn (không cần runtime, image nhỏ), build lâu hơn (cần
cache).
