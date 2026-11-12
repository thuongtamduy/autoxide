# Lộ trình

Dựng theo thứ tự. Mỗi bước xong khi `./gate.sh` xanh trên **cả SQLite và
Postgres**; agent đánh dấu `[x]` rồi commit. Chi tiết thư viện: [`STACK.md`](../STACK.md).

Trạng thái: **bước 0–12 xong**; đang ở bước 13 (củng cố core), mục "CI chạy xanh trên
GitHub". Sau đó: bước 15 (nền chung) → 16 (module `media`) → 17 (`content`) — D-085, D-087,
D-088, D-089.

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

## 15. Nền chung cho media và nội dung (core)

Thứ `media` và `content` cùng cần, viết trung lập với nghiệp vụ để module nào cũng dùng được
(D-088):

- [ ] Danh sách lọc theo trường và sắp xếp theo trường khác thời gian tạo (vd tên, kích thước,
      `published_at`), vẫn dùng cursor, không `OFFSET`
- [ ] Tìm kiếm văn bản sau một trait, cài đặt và test cho cả SQLite và Postgres
- [ ] Endpoint đọc công khai (không cần đăng nhập): module khai route nào công khai qua hợp đồng
      module; rate limiting vẫn áp
- [ ] Cache: `ETag` + `If-None-Match` → `304`, `Cache-Control` do endpoint khai
- [x] Chống ghi đè khi hai người cùng sửa: `ETag` + `If-Match`, sai thì `412` (làm ở bước 14, D-084)

## 16. Module `media` — thư viện media (chủ dự án chọn ngày 12/11/2026, D-089)

Module mới, tách khỏi `files`. `files` giữ nguyên: kho file thô, riêng tư theo người sở hữu,
dùng cho mọi module (D-067). `media` là thư viện ảnh, video, tài liệu để **xuất bản** (website,
`content`, storefront nếu có sau này). Nội dung lưu qua trait `Storage` của kernel (do `files`
cung cấp, D-066) dưới khoá `media/<id>`; metadata ở bảng `media_…`; không tham chiếu bảng của
`files`. Đăng ký sau `files`.

- [ ] Tải lên, xem, tải về, đổi tên, xoá; giới hạn kích thước và loại bằng cấu hình
      `AUTOXIDE_MEDIA_…`; thân request là nội dung thô như `files` (D-067)
- [ ] Thư viện chung của hệ thống (không theo người tải lên); permission xem, tải lên, quản lý
      tách riêng
- [ ] Công khai / riêng tư: item công khai tải được không cần đăng nhập qua URL ổn định
- [ ] Hiển thị inline an toàn: chỉ ảnh raster (JPEG, PNG, WebP, AVIF, GIF) hiển thị inline; loại
      khác (SVG, HTML…) luôn là attachment; kiểm loại thật theo nội dung (magic bytes), khai sai
      thì từ chối
- [ ] Tải về hỗ trợ `Range` (video, PDF lớn) và cache của bước 15 (`304`; item công khai có
      `Cache-Control` dài)
- [ ] Metadata: chiều rộng và chiều cao của ảnh (đọc lúc tải lên); alt text, tiêu đề, chú thích
      sửa được
- [ ] Biến thể ảnh (thumbnail, đổi cỡ, WebP/AVIF): chọn sinh ở core qua job queue hay để website
      tự tối ưu; ghi quyết định trước khi làm
- [ ] Sắp xếp thư viện bằng thư mục **hoặc** nhãn (chọn một, ghi quyết định); lọc theo loại,
      tìm theo tên, sắp xếp theo tên / kích thước / ngày
- [ ] Thay nội dung mà giữ nguyên ID (nơi đang dùng không phải sửa), cache cũ phải hết hiệu lực
- [ ] Xoá an toàn: thùng rác khôi phục được trong một thời hạn; job xoá hẳn và dọn nội dung mồ
      côi ở nơi lưu
- [ ] Báo item trùng (cùng SHA-256) khi tải lên
- [ ] `api` cho module khác: kiểm một item có tồn tại, có công khai không, và lấy URL công khai
      (`content` dùng cho ảnh đại diện và ảnh trong bài)
- [ ] Event cho mọi thay đổi (vào audit log); tool MCP: liệt kê, xem, sửa metadata, xoá
- [ ] Backup và restore (`autoxide backup`) gồm cả nội dung của `media`
- [ ] Đăng ký trong `crates/app/src/modules.rs`; test trên cả hai database, nơi lưu là ổ đĩa và
      S3; hướng dẫn dùng (tiếng Anh + tiếng Việt)

Chưa làm, đã cân nhắc: tải lên thẳng S3 bằng URL ký sẵn, hạn mức dung lượng, quét virus — thêm
khi có nhu cầu.

## 17. Module `content` — nội dung (chủ dự án chọn ngày 12/11/2026, D-085, D-087)

Chỉ nội dung và những gì liên quan đến nội dung; không thương mại điện tử. Làm sau `media`
(D-088, D-089). Giao diện (trang quản trị, website) là repo riêng (D-086) — bước này chỉ làm API.

- [ ] Bài viết và trang: tiêu đề, slug duy nhất, tóm tắt, nội dung JSON có cấu trúc (core không
      phụ thuộc trình soạn thảo), ảnh đại diện và ảnh trong bài (ID của `media`, kiểm qua `api`
      của `media`), tác giả (ID người dùng)
- [ ] Trạng thái nháp / đã đăng / lưu trữ; lên lịch đăng qua job queue
- [ ] Lịch sử phiên bản: xem và khôi phục
- [ ] Danh mục và thẻ
- [ ] Menu điều hướng
- [ ] Đa ngôn ngữ (vi, en): mỗi bản dịch có slug riêng, liên kết với nhau
- [ ] SEO: meta title, meta description, ảnh chia sẻ, canonical; API đủ để website sinh
      sitemap và RSS
- [ ] Permission: đọc nháp, viết, đăng; người đọc công khai chỉ thấy bản đã đăng (endpoint
      công khai và cache của bước 15)
- [ ] Event và audit cho mọi thay đổi; tool MCP (liệt kê, đọc, tạo nháp, sửa, đăng) gọi cùng service
- [ ] Đăng ký trong `crates/app/src/modules.rs`; test trên cả hai database; hướng dẫn dùng
      (tiếng Anh + tiếng Việt)

## Sau core (chưa mở)

GraphQL · tách service — xem bảng "Chưa làm, đã cân nhắc" trong `STACK.md`.
