# Lộ trình

Dựng theo thứ tự. Mỗi bước xong khi `./gate.sh` xanh trên **cả SQLite và
Postgres**; agent đánh dấu `[x]` rồi commit. Chi tiết thư viện: [`STACK.md`](../STACK.md).

Trạng thái: **bước 0–29 xong**; đang làm 30–32 (bước 30: phần trong repo xong, chờ chủ dự án công khai repo rồi kiểm lần chạy đầu của workflow), nền đa khách 33–38 (D-185…D-189); bản phát hành mới nhất `v0.11.0`. CI trên GitHub đã tắt (D-107):
`./gate.sh` cục bộ là gate duy nhất.

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

- [x] ~~CI chạy xanh trên GitHub~~ — chủ dự án tắt CI trên GitHub; `./gate.sh` cục bộ là gate duy nhất (D-107)
- [x] Rà soát bảo mật toàn repo bằng một lượt đọc độc lập; sửa mọi lỗi tìm thấy (D-079)
- [x] Rà soát bảo mật lần hai cho phần thêm sau đó (media, content, webhook, AI, luồng giữa các module); sửa mọi lỗi tìm thấy (D-149)
- [x] Rà soát bảo mật forms, newsletter, comments và nền chung cho dữ liệu người lạ; sửa mọi lỗi tìm thấy (D-150)
- [x] Thử tải dài hạn: không rò bộ nhớ, không rò kết nối database (docs/BENCHMARKS.md)

Hệ thống tài khoản:

- [x] Quản lý phiên đăng nhập: người dùng xem và đăng xuất từng thiết bị; quản trị viên thu hồi mọi phiên của một người (D-080)
- [x] Xoá tài khoản (event cho mọi module dọn dữ liệu của người đó) và xuất dữ liệu cá nhân (D-080)
- [x] Tìm người dùng theo email hoặc tên (D-080)

Phát hành và nâng cấp:

- [x] Số phiên bản, `CHANGELOG`, bản phát hành trên GitHub (làm tay theo `docs/RELEASING.md`, D-107; `v0.2.0`)
- [x] ~~CI build và đăng image Docker lên GitHub Container Registry~~ — thay bằng build image cục bộ khi phát hành (D-107)
- [x] Dependabot đề xuất nâng cấp; kiểm lỗ hổng thư viện bằng `cargo deny check` trong gate (lịch hằng ngày trên GitHub đã tắt, D-107)

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
(D-085, D-099). Bước này và bước 16 làm trên `main` dưới tên "bước 15: thư viện media"; tách
lại thành 15 và 16 khi gộp nhánh đề bài của `content` (D-103).

- [x] Danh sách lọc theo trường và sắp xếp theo trường khác thời gian tạo (tên, kích thước…)
      bằng keyset pagination, không `OFFSET`; tìm theo chữ dùng chung (D-085)
- [x] Tìm được tiếng Việt không dấu ("da lat" ra "Đà Lạt"), như nhau trên SQLite và Postgres (D-104)
- [x] Endpoint đọc công khai (`Option<Principal>`); rate limiting vẫn áp (D-085)
- [x] Cache: `ETag` + `If-None-Match` → `304`, `Cache-Control` do endpoint đặt (D-085)
- [x] Chống ghi đè: `ETag` + `If-Match` → `412` (bước 14, D-084); `If-Match` bắt buộc → `428` (D-085)
- [x] `Storage` đọc theo khoảng byte và liệt kê theo tiền tố, có cài đặt mặc định (D-085)

## 16. Module `media` — thư viện media (chủ dự án giao ngày 12/11/2026)

Module `media`: ảnh, video, âm thanh, tài liệu trong thư mục lồng nhau, cho website và module
`content` dùng lại (D-086, D-100). `files` giữ nguyên.

- [x] Tải lên: kiểm kích thước, loại được phép, loại thật theo chữ ký đầu file; kích thước ảnh; SHA-256; báo item trùng (D-086, D-089)
- [x] Danh sách lọc theo thư mục, loại, `visibility`, thùng rác, tên; sắp theo thời gian, tên, kích thước
- [x] Sửa thông tin và thay nội dung giữ ID, bắt buộc `If-Match`
- [x] Tải nội dung: công khai hay cần quyền; `inline` chỉ cho ảnh raster; `Range`, `HEAD`, 304; cache theo phiên bản (D-086, D-087)
- [x] Thư mục lồng nhau: tạo, đổi tên, chuyển, xoá khi trống (D-088)
- [x] Thùng rác: vào, khôi phục, xoá hẳn (permission riêng); job xoá quá hạn và dọn nội dung mồ côi
- [x] Event cho mọi thay đổi (vào audit log); tool MCP; trait `Media` cho module khác (D-090)
- [x] Backup / restore mang theo `media`; hướng dẫn dùng (tiếng Anh + tiếng Việt)

Bổ sung (chủ dự án giao thêm ngày 12/11/2026):

- [x] Ảnh thu nhỏ làm bằng job nền, lấy bằng `?w=` (D-093)
- [x] Nhãn song song với thư mục; quản lý nhãn (D-094)
- [x] Tải lên qua MCP (base64, tối đa 1 MiB) (D-095)


## 17. Module `content` — nội dung (chủ dự án chọn ngày 12/11/2026, D-096, D-098, D-101)

Chỉ nội dung và những gì liên quan đến nội dung; không thương mại điện tử. Làm sau `media`
(D-099, D-100). Giao diện (trang quản trị, website) là repo riêng (D-097) — bước này chỉ làm API.
Thiết kế: D-102; đề bài đầy đủ: [`docs/prompts/step-17-content.md`](prompts/step-17-content.md);
khảo sát: [`docs/research/content.md`](research/content.md).

- [x] Bài viết và trang chung một mô hình: tiêu đề, slug, tóm tắt, thân bài JSON (máy chủ kiểm
      theo danh sách node được phép, chặn link độc), ảnh đại diện và ảnh trong bài (ID của
      `media`, kiểm qua `api` của `media`); số chữ, thời gian đọc, văn bản thuần để tìm kiếm
- [x] Nháp tách khỏi bản công khai; đăng, gỡ, lưu trữ; hẹn giờ đăng và gỡ qua job queue (ghim
      một revision)
- [x] Lịch sử phiên bản: xem và khôi phục
- [x] Trang có cấp cha con, đường dẫn lồng nhau, chuyển trang sang nhánh khác (D-101)
- [x] Đường dẫn do backend tính; endpoint `resolve`; tự tạo chuyển hướng khi đường dẫn công khai
      đổi (cả cây con), chuyển hướng làm tay, 410 cho nội dung đã gỡ
- [x] Danh mục (cha con) và thẻ; tác giả là hồ sơ của `content`, một bài nhiều tác giả
- [x] Menu điều hướng nhiều tầng, theo ngôn ngữ, trỏ tới ID nội dung hoặc URL
- [x] Đa ngôn ngữ (vi, en): mỗi bản dịch một bản ghi, slug riêng, liên kết với nhau
- [x] SEO: meta title, meta description, ảnh chia sẻ, canonical, `noindex`; dữ liệu cho sitemap
      (kèm bản dịch cho hreflang) và feed
- [x] Xem trước bản nháp bằng token ký, có hạn
- [x] Permission: đọc nháp, viết, đăng, quản lý; người đọc công khai chỉ thấy bản đã đăng
      (endpoint công khai, tìm kiếm và cache của bước 15); `If-Match` bắt buộc (D-102)
- [x] Event và audit cho mọi thay đổi, đủ dữ liệu để website làm mới cache qua webhook; tool MCP
      gọi cùng service; dọn và xuất dữ liệu cá nhân
- [x] Đăng ký trong `crates/app/src/modules.rs`; test trên cả hai database (kể cả test đồng
      thời); hướng dẫn dùng (tiếng Anh + tiếng Việt)

## 18. Tính năng AI (chủ dự án giao ngày 12/11/2026, D-109)

Tầng kết nối AI trung lập (nhiều nhà cung cấp, mặc định tắt), quyền cho AI agent, gợi ý SEO và
alt text, dịch bài, tìm theo ý nghĩa. Đề bài: [`docs/prompts/ai.md`](prompts/ai.md); khảo sát:
[`docs/research/ai.md`](research/ai.md).

- [x] Quyền cho agent: `content.delete` tách riêng, `content.write_own`, `content.publish_own`, xin
      duyệt, `ai_generated` / `ai_reviewed`, rate limiting theo API key
- [x] Tầng kết nối: trait `TextModel` / `Embedder` ở kernel, module `ai` (adapter `anthropic` và
      `openai`, client embedding, hạn mức token mỗi ngày, metrics, `/ai/status`, `autoxide ai check`)
- [x] Kiểm SEO không dùng AI; gợi ý SEO bằng AI (chỉ gợi ý, không ghi)
- [x] Gợi ý alt text cho ảnh của `media` (ảnh riêng tư chỉ gửi khi bật)
- [x] Dịch bài bằng AI vào nháp: tách mẩu có thẻ giữ chỗ, không ghi đè bản người đã sửa, báo bản dịch đã cũ
- [x] Tìm theo ý nghĩa kết hợp tìm theo chữ (RRF), chạy như nhau trên hai database; endpoint công
      khai tắt mặc định; số đo 10.000 và 100.000 đoạn
- [x] Test trên cả hai database (kể cả đồng thời, máy chủ AI giả); hướng dẫn `docs/AI.md` + `docs/AI.vi.md`

## 19. Nền chung cho dữ liệu người đọc gửi lên (core, chủ dự án giao ngày 12/11/2026, D-112)

Đề bài: [`docs/prompts/forms-newsletter-comments.md`](prompts/forms-newsletter-comments.md),
giai đoạn 0; khảo sát: [`docs/research/forms-newsletter-comments.md`](research/forms-newsletter-comments.md).
Mỗi mục một commit, gate xanh trên cả hai database.

- [x] Khoá ký của kernel (`AUTOXIDE_SECRET_KEY`, dẫn khoá theo mục đích; không cấu hình thì sinh
      một lần và lưu trong database)
- [x] `ClientInfo` dùng chung (IP đã giải, user agent, `Origin`, `Referer`);
      `AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT`
- [x] Băm IP (gộp tiền tố rồi HMAC; dạng `bucket` theo ngày và `fingerprint`)
- [x] `Throttle` lưu trong database, dùng chung; `auth` chuyển sang dùng
- [x] Token dùng một lần trong kernel; `auth` chuyển sang dùng, hành vi không đổi
- [x] Chống spam hai pha: nhận / cách ly / từ chối; trường bẫy, token form ký, giới hạn theo IP
      và email băm, phát hiện trùng; nonce của token form làm idempotency cho người lạ
- [x] ALTCHA v2 tự viết (mặc định tắt)
- [x] `Mail` / `Mailer`: `reply_to`, header theo danh sách cho phép, lỗi vĩnh viễn / tạm thời,
      khoá chống trùng; event thất bại vĩnh viễn
- [x] `Actor` cho người lạ trên outbox
- [x] `ResourceDirectory` trung lập; `content` đăng ký resolver
- [x] `AUTOXIDE_HTTP_PUBLIC_URL`
- [x] Hợp đồng chung cho ba module: `docs/prompts/reader-input-contract.md`

## 20. Module `forms` — biểu mẫu (chủ dự án giao ngày 12/11/2026, D-112)

- [x] Định nghĩa form có phiên bản, trường theo danh sách đóng, nhãn theo ngôn ngữ
- [x] Nhận bài gửi công khai qua chống spam; máy chủ kiểm mọi trường, báo mọi lỗi một lần
- [x] Hộp thư quản trị, email báo quản trị viên, xuất CSV (chặn CSV injection)
- [x] Xoá và xuất theo email; tự xoá theo thời hạn; event không mang câu trả lời; tool MCP
- [x] Đăng ký trong `crates/app/src/modules.rs`; test trên cả hai database; hướng dẫn (EN + VI)

## 21. Module `newsletter` — bản tin email (chủ dự án giao ngày 12/11/2026, D-112)

- [x] Danh sách, người đăng ký, xác nhận hai bước (`POST`, không `GET`)
- [x] Huỷ đăng ký một chạm (RFC 8058) và trang chọn danh sách bằng token
- [x] Chiến dịch: nháp, hẹn giờ, gửi theo lô có giới hạn tốc độ, tạm dừng, tiếp tục, huỷ; không
      gửi trùng khi chạy lại
- [x] Danh sách chặn, endpoint bounce chung; thư quảng cáo theo cấu hình
- [x] Tự tạo bản tin khi `content` đăng bài (tuỳ chọn); tool MCP
- [x] Đăng ký trong `crates/app/src/modules.rs`; test trên cả hai database; hướng dẫn (EN + VI)

## 22. Module `comments` — bình luận (chủ dự án giao ngày 12/11/2026, D-112)

- [x] Bình luận theo đích bất kỳ, hai cấp; người dùng đăng nhập hoặc khách
- [x] Kiểm duyệt theo pipeline cố định; báo cáo vi phạm; cấm theo người dùng / email / IP băm
- [x] Sửa trong thời hạn (token sửa cho khách), xoá mềm giữ luồng, đếm, mở / đóng đích
- [x] Báo email khi có trả lời (chọn nhận, email đã xác minh); giữ dữ liệu theo thời hạn
- [x] Dữ liệu cá nhân; tool MCP
- [x] Đăng ký trong `crates/app/src/modules.rs`; test trên cả hai database; hướng dẫn (EN + VI)

## 23. Đóng các giới hạn còn để lại của D-150 (chủ dự án giao ngày 12/11/2026)

- [x] `comments`: tên viết trọn chữ Kirin / Hy Lạp trông như tên Latin không qua được danh sách
      tên dành riêng và luật không trùng tên (so theo dạng "khung" của chữ)
- [x] Địa chỉ client: tin proxy theo dải địa chỉ (`AUTOXIDE_HTTP_TRUSTED_PROXIES`), header chỉ
      được đọc khi kết nối đến từ proxy tin cậy
- [x] Dấu IP riêng cho từng module (không nối được người gửi giữa các module); cảnh báo lúc khởi
      động khi khoá ký nằm trong database
- [x] `newsletter`: đăng ký công khai trả lời trong cùng một khoảng thời gian dù địa chỉ ở trạng
      thái nào (việc ghi và gửi mail chạy trong job)

## 24. Sổ module cho màn hình quản lý module (chủ dự án giao ngày 12/11/2026, D-155)

Giao diện quản trị ở repo riêng (D-096, D-097); bước này chỉ làm API trong core.

- [x] `GET /api/v1/kernel/modules` và `GET /api/v1/kernel/modules/{name}` (kèm trạng thái
      migration); permission `kernel.modules.read`; tool MCP; test trên cả hai database

## 25. Rà soát bảo mật và hiệu năng toàn core (chủ dự án giao ngày 12/11/2026)

- [x] Rà soát độc lập (bốn vùng: HTTP / MCP / quyền; `auth`; lưu trữ, job, event, webhook, mail;
      hiệu năng đường nóng và index)
- [x] Kiểm lại từng phát hiện với code; sửa kèm test chạy trên cả hai database; ghi quyết định

## 26. Đóng các việc để lại của bước 25 (agent quyết, theo D-156…D-158)

- [x] Lệnh CLI gỡ xác thực hai bước của một tài khoản (admin duy nhất mất thiết bị)
- [x] `files`: job dọn nội dung mồ côi trong nơi lưu (như `media`)
- [x] `autoxide jobs retry` một job `webhooks.send` đã hỏng thì gửi lại thật
- [x] Đặt lại mật khẩu: trả lời trong cùng khoảng thời gian dù email có hay không (việc chạy trong job)
- [x] Đo tải (`scripts/loadtest.sh`) rồi tối ưu xác thực mỗi request nếu đo được lợi — đo: không lợi, không làm (D-163)

## 27. Cửa cho người viết module, core sẵn sàng thành framework (chủ dự án giao ngày 13/11/2026, D-164)

Mượn cái hay của framework khác ([`docs/research/frameworks.md`](research/frameworks.md)).

- [x] `TestApp` trong `autoxide-testkit`: dựng module như binary trên `TestDatabase`, gọi REST và
      MCP với quyền tuỳ chọn; `notes` dùng nó; hướng dẫn (EN + VI)
- [x] Các module còn lại chuyển sang `TestApp` (một cách làm cho mỗi việc)
- [x] `cargo xtask new-module <tên>`: sinh module mới từ khuôn `notes`, qua được gate
- [x] `Module::checks` + `autoxide doctor`: kiểm cấu hình, database, migration và kiểm của module
- [x] `Module::tasks` + `autoxide task`: lệnh quản trị do module khai — D-169
- [x] Hợp đồng ổn định: danh sách public dependency, `#[non_exhaustive]` cho kiểu sẽ lớn thêm,
      chính sách deprecate API Rust; hướng dẫn viết module ở repo riêng (EN + VI) — D-167
- [x] Hàm tạo / builder cho các kiểu module đang dựng bằng literal, rồi `#[non_exhaustive]`
      (danh sách ở D-167) — D-170

## 28. Dọn cấu trúc (chủ dự án giao ngày 13/11/2026)

Từ lượt rà soát kiến trúc cùng ngày: bỏ code lặp, giữ luật handler, module lớn dễ tìm.

- [x] Kiểm email một chỗ: `autoxide_kernel::email` cho `forms`, `comments`, `newsletter`; `auth`
      dùng chung phần chuẩn hoá
- [x] `content`: handler không đọc database (`describe`, `preview_entry` vào service); `cargo xtask
      check` chặn handler gọi database
- [x] `content`: gom file theo nhóm, tách `tests.rs` thành thư mục
- [x] Quy ước bố cục module (theo lớp hay theo tính năng) trong hướng dẫn viết module (EN + VI)
- [x] `kernel`: tách bộ đăng ký `Modules` khỏi `module.rs`
- [x] `crates/app`: test đầu-cuối `media_e2e` ra khỏi chỗ code của binary

## 29. Nền cho nghiệp vụ tiền (chủ dự án giao ngày 13/11/2026)

Từ lượt rà soát cùng ngày: core đã có transaction nguyên tử, event cùng transaction, idempotency
key (24 giờ), ETag / `If-Match`, job không chạy trùng, audit log chỉ thêm — nhưng chưa đủ cho
nghiệp vụ có tiền (số dư, giao dịch, phân bổ). Mọi mục thuộc core, trung lập với ngành, chạy và
có test trên cả SQLite và Postgres; test đồng thời cho mọi thứ ghi đồng thời.

- [x] Kiểm tràn số nguyên ở bản release: `[profile.release] overflow-checks = true` trong
      `Cargo.toml` gốc (hiện không khai, nên bản build thật cộng tràn `i64` sẽ âm thầm quay
      vòng). Đo lại bằng `scripts/loadtest.sh` trước và sau, ghi số đo vào quyết định
- [x] `autoxide_kernel::money`: kiểu `Money` = số nguyên `i64` đơn vị nhỏ nhất + mã tiền tệ
      ISO 4217 (kèm số chữ số thập phân của mã). Cộng, trừ, nhân với số nguyên có kiểm tràn và
      trả `Error`; cấm phép tính giữa hai loại tiền; chia / phân bổ theo tỉ lệ không mất đơn vị
      nào (100 chia 3 → 34, 33, 33); JSON và OpenAPI với `amount` là chuỗi số nguyên (client
      JavaScript làm tròn sai số lớn hơn 2⁵³) và `currency`; cách lưu chuẩn là hai cột
      `<tên>_minor` (`BIGINT`) và `<tên>_currency`
- [x] Cập nhật có điều kiện và khoá dòng thành công cụ chung của kernel, thay mẹo "chạm dòng"
      đang viết tay ở `auth` (`user/service.rs`, `role/service.rs`, `session/service.rs`),
      `forms` (`public.rs`), `comments` và `content` (`content_tree_locks`): hàm khoá một dòng tới
      hết transaction, và cách chuẩn để trừ số dư (`UPDATE … SET balance = balance - x WHERE id = ?
      AND balance >= x`, kiểm số dòng bị đổi). Test: nhiều request trừ cùng lúc trên cả hai
      database, số dư không bao giờ âm, tổng không lệch
- [x] Thử lại transaction khi xung đột tạm thời: một hàm của `Database` chạy lại transaction ghi
      có giới hạn số lần khi gặp deadlock hay xung đột (Postgres `40001`, `40P01`; SQLite bận),
      lỗi khác trả ngay. Tài liệu nói rõ phần việc bên trong phải chịu được chạy lại. Test cả
      hai nhánh
- [x] Audit log chống sửa: mỗi dòng của `audit_entry` mang mã băm của nội dung và của dòng trước
      (chuỗi băm); `autoxide doctor` kiểm chuỗi còn nguyên và báo dòng đầu tiên bị sửa. Giải
      quyết cùng việc xoá dữ liệu cá nhân (`modules/audit/src/recorder.rs` xử lý `USER_DELETED`):
      băm phần không chứa dữ liệu cá nhân, hoặc cách khác giữ được chuỗi. Có migration cho dữ
      liệu cũ
- [x] Chống trùng lệnh ở tầng nghiệp vụ: quy ước mã tham chiếu do client đặt (`reference`), duy
      nhất theo người gọi, lưu vĩnh viễn cùng bản ghi; gửi lại cùng mã thì trả kết quả cũ, cùng
      mã mà nội dung khác thì 409/422 — không dựa riêng vào idempotency key HTTP (hết hạn sau 24
      giờ). Helper trong kernel nếu đủ chung
- [x] Hướng dẫn "Nghiệp vụ có tiền" trong hướng dẫn viết module (EN + VI): dùng `Money`, thứ tự
      khoá dòng chống deadlock, cập nhật có điều kiện, thử lại khi xung đột, mã tham chiếu, event
      cùng transaction, không sửa hay xoá giao dịch mà ghi giao dịch đảo; module `notes` hay một
      test mẫu minh hoạ

Chờ chủ dự án quyết — KHÔNG làm khi chưa có quyết định ghi trong `docs/DECISIONS.md`:

- Thư viện số thập phân (vd `rust_decimal`) để tính lãi, tỉ giá, thuế với quy tắc làm tròn rõ
  ràng; chỉ dùng để tính, vẫn lưu bằng số nguyên. Thêm thư viện thì ghi `STACK.md`.
- Module sổ cái kép (`ledger`: mỗi giao dịch có bút toán nợ và có cân nhau, chỉ thêm, số dư suy ra
  từ bút toán): module nghiệp vụ mới, `CLAUDE.md` chỉ cho dựng module chủ dự án đã giao.

## 30. Ra mắt công khai (chủ dự án giao ngày 13/11/2026, D-175, D-176)

Đưa Autoxide tới tay người lạ: repo công khai, CI và phát hành tự động, image Docker có chứng
thực, file của một dự án mã nguồn mở, ảnh nhẹ, deploy không gián đoạn, cài được chỉ bằng README.
Không đụng code của bước 29 (trừ `media` và `kernel/src/http/health.rs`), nên làm ngay được.
Tài liệu cho người ngoài viết tiếng Anh kèm bản tiếng Việt cùng nội dung.

Nhóm A — repo công khai, CI, phát hành (D-175):

- [ ] Chủ dự án công khai repo (Settings → Change visibility); trước đó agent xác nhận gitleaks
      quét toàn lịch sử xanh và không file nào chứa tên máy chủ, IP, đường dẫn riêng hay thông tin
      cá nhân. Agent không tự đổi chế độ công khai
      — 13/11/2026 agent đã quét: gitleaks toàn lịch sử mọi nhánh xanh, file không chứa IP thật,
      tên máy hay đường dẫn riêng; email cá nhân trong metadata commit: chủ dự án chọn giữ
      nguyên (D-197). Còn chờ chủ dự án bấm công khai
- [ ] Ba workflow tự chạy lại: `ci.yml` (push lên `main`, pull request), `audit.yml` (lịch),
      `release.yml` (tag `vX.Y.Z`) — trigger cũ có trong `git log -p -- .github/workflows`; chạy
      xanh trên GitHub; bỏ ghi chú "TẮT tự chạy (D-107)" ở đầu các file — trigger đã bật lại, job
      chỉ chạy khi repo công khai (D-197); còn chờ lần chạy xanh đầu tiên sau khi công khai
- [ ] `release.yml` đăng image `ghcr.io/<owner>/autoxide:X.Y.Z` và `:X.Y` bằng `GITHUB_TOKEN`
      (`packages: write`), gói image để công khai; thử với bản phát hành kế tiếp. `docs/RELEASING.md`
      sửa theo: agent chỉ còn đẩy tag, bản phát hành và image do workflow làm — workflow và
      `RELEASING.md` đã sửa (D-197); còn chờ bản phát hành đầu tiên sau khi công khai, và chủ
      repo đổi gói image sang Public một lần
- [ ] Secret scanning và push protection của GitHub bật; Dependabot alerts bật; gitleaks trong
      `./gate.sh` giữ nguyên — chủ dự án bật trong Settings → Code security sau khi công khai,
      cùng "Private vulnerability reporting" mà `SECURITY.md` dựa vào
- [ ] Image Docker cho máy ARM64 (`linux/arm64`) trong workflow `Release` khi repo đã công khai
      (máy build ARM của GitHub miễn phí cho repo công khai; D-078 để sau vì build giả lập quá lâu)
      — đã có trong `release.yml` (D-197); còn chờ bản phát hành đầu tiên sau khi công khai

Nhóm B — file của dự án mã nguồn mở:

- [x] `SECURITY.md` (EN + VI): phiên bản còn được vá, cách báo lỗ hổng riêng tư (bật "Private
      vulnerability reporting" của GitHub — chủ dự án bấm), thời gian phản hồi dự kiến, những gì
      không tính là lỗ hổng; README trỏ tới nó
- [x] `CONTRIBUTING.md` (EN + VI) cho người đóng góp là người: dựng môi trường, hai mức gate
      (D-173), luật quan trọng nhất tóm từ `CLAUDE.md` (hai database, ranh giới module, lỗi có
      `code`, test), cách viết commit và PR, license của phần đóng góp (MIT OR Apache-2.0, như
      repo); trỏ `CLAUDE.md` cho người dùng AI agent
- [x] Quy tắc ứng xử (`CODE_OF_CONDUCT.md`, theo Contributor Covenant bản mới nhất) với địa chỉ
      liên hệ do chủ dự án đặt; mẫu issue (báo lỗi: phiên bản, database, cách tái hiện; đề xuất) và
      mẫu PR (đã chạy `./gate.sh`, đã sửa EN + VI, đã thêm CHANGELOG) trong `.github/`
- [ ] Bản phát hành đáng tin: workflow `Release` (sau nhóm A) đính kèm chứng thực nguồn gốc build
      (GitHub artifact attestation) và danh sách thành phần (SBOM) cho image Docker; `docs/DEPLOY.md`
      (EN + VI) có lệnh kiểm chứng thực trước khi chạy image — đã có trong `release.yml` và
      `DEPLOY.md` (D-197); còn chờ bản phát hành đầu tiên để kiểm bằng `gh attestation verify`
- [x] `cargo xtask check` hay test giữ các file trên không mất và hai bản EN / VI không lệch
      tiêu đề mục (cách làm như kiểm bản dịch i18n), để agent phiên sau không quên sửa một bên

Nhóm C — sản phẩm (D-176):

- [x] `media`: bản nhỏ WebP bên cạnh JPEG / PNG. Chọn bộ mã hoá sau khi kiểm thư viện đúng phiên
      bản ghim (kiểu nén mất dữ liệu cho ảnh chụp, có alpha cho ảnh trong suốt); website nhận được
      WebP qua API công khai (`PublicMedia`, `body_media`, `srcset`) mà client cũ không hỏng; ảnh đã
      có làm lại bằng `autoxide task media.refresh_variants`; đo dung lượng và thời gian mã hoá
      trên ảnh thử của D-168, ghi vào quyết định; hướng dẫn `docs/MEDIA.md` (EN + VI)
- [x] Deploy không gián đoạn: nhận tín hiệu dừng thì `/health/ready` trả 503 trong một khoảng
      cấu hình được (biến mới qua `Setting`, sinh lại `.env.example`) trước khi ngừng nhận kết nối,
      `/health/live` vẫn 200; rồi mới tới `AUTOXIDE_HTTP_SHUTDOWN_SECONDS`. Test; `docs/DEPLOY.md`
      (EN + VI)
- [ ] Thử cài như người lạ: máy hay container sạch, làm đúng README tiếng Anh bằng image Docker đã
      đăng (cả SQLite và Postgres), ghi mọi chỗ vấp rồi sửa README / `docs/DEPLOY.md` (EN + VI cùng
      commit)

## 31. Hợp đồng API và chống hồi quy (chủ dự án giao ngày 13/11/2026, D-177)

Người viết giao diện (repo riêng, D-097), nhà phát triển ngoài và AI agent dùng API không gọi sai;
hợp đồng, tài liệu và database của bản cũ không bao giờ âm thầm lệch khỏi code. Bước này sửa
`xtask`, `testkit`, `gate.sh`, tài liệu và thêm thư mục SDK — làm ngay được; riêng test nâng cấp
cần image Docker của bản cũ (bước 30). Test chạy trên cả SQLite và Postgres; thư viện chỉ cho test
khai ở `[dev-dependencies]`.

Nhóm A — hợp đồng và người gọi API:

- [ ] Gate chặn thay đổi phá vỡ của `/api/v1` (D-059): so `docs/openapi.json` với bản ở tag phát
      hành gần nhất (`git show vX.Y.Z:docs/openapi.json`) và báo đỏ khi xoá endpoint hay method,
      thêm tham số hay trường bắt buộc vào request, xoá trường hay đổi kiểu trong response, bỏ
      giá trị của enum. Endpoint đã khai deprecated và quá ngày `sunset` thì được bỏ. Viết bằng
      Rust trong `xtask` (thêm lệnh), có test cho từng loại thay đổi; thông báo nói rõ chỗ nào
      phá và cách làm đúng (endpoint mới + deprecate endpoint cũ)
- [ ] SDK TypeScript sinh từ `docs/openapi.json` (D-177: trong repo, không đăng npm), đủ kiểu cho
      request, response, lỗi `Problem` và pagination; gate kiểm SDK khớp hợp đồng như cách test
      đang kiểm `openapi.json`. Gate không bắt người lạ cài Node: nếu công cụ sinh cần Node thì chạy
      qua image Docker ghim phiên bản (như gitleaks) hoặc chỉ kiểm trong CI — chọn và ghi quyết định
- [ ] Hướng dẫn theo tình huống (EN + VI, thư mục mới trong `docs/`): website đọc bài và ảnh
      (`srcset`, `body_media`, ETag / 304, cache), đăng nhập và làm mới token, API key cho máy,
      gọi lại an toàn bằng `Idempotency-Key`, pagination, đọc lỗi (`code`, `errors`, `Retry-After`).
      Mỗi tình huống có ví dụ `curl` và TypeScript dùng SDK
- [ ] Hướng dẫn nối AI agent qua MCP (EN + VI), mở rộng đoạn ngắn trong README: tạo API key quyền
      tối thiểu cho agent, cấu hình Claude Code và client MCP khác, danh sách tool theo module và
      permission cần, giới hạn (rate limit theo API key, bản nháp chờ duyệt — D-109)
- [ ] Ví dụ trong tài liệu được test: các ví dụ của hai mục trên chạy thật trên app dựng bằng
      `TestApp` trong gate (hay một lệnh `xtask` gate gọi), để tài liệu không lệch code

Nhóm B — chống hồi quy:

- [ ] Test nâng cấp từ bản phát hành trước: dựng database bằng image Docker của bản trước, ghi dữ
      liệu mẫu qua API của nó, rồi chạy bản hiện tại trên database đó (migration, đọc lại dữ liệu,
      vài lệnh ghi) — trên cả SQLite và Postgres; chạy trong CI và trước mỗi lần phát hành
- [ ] Migration thật của mọi module chạy vòng up → down → up trên database trống và trên database
      có dữ liệu, trên cả hai database (hiện chỉ có test cơ chế bằng bảng mẫu,
      `crates/kernel/tests/migration.rs`)
- [ ] Property test cho các bộ kiểm đầu vào: thân bài ProseMirror (`content`), slug, email
      (`autoxide_kernel::email`), con trỏ pagination, CSV chống injection, chữ và link của bình
      luận (chống XSS) — sinh đầu vào ngẫu nhiên, không panic, giữ bất biến; thêm thư viện thì ghi
      `STACK.md`
- [ ] Response thật khớp hợp đồng: test HTTP qua `TestApp` kiểm mỗi response (thân và mã) khớp
      schema của endpoint đó trong `docs/openapi.json`, để tài liệu không hứa điều code không làm

## 32. Dọn bố cục và việc để lại (agent đề xuất ngày 13/11/2026)

Bắt đầu SAU khi bước 29 đã merge vào `main` (đụng cùng module). Làm nhóm A trước: nhóm B sửa đúng
các module nhóm A dời file. Mỗi mục có test trên cả hai database và dòng CHANGELOG.

Nhóm A — bố cục:

- [ ] Áp quy ước "File layout" của hướng dẫn viết module cho `newsletter`, `comments`, `media`,
      `forms`: phần service chia theo tính năng, test dài thành thư mục `tests/`; không đổi hành vi,
      đếm số test trước và sau phải bằng nhau
- [ ] `kernel`: gom các file nằm phẳng trong `src/` thành nhóm (vd hợp đồng cho module: `ai`,
      `mail`, `storage`, `search`, `privacy`…), giữ nguyên đường dẫn công khai bằng `pub use` để
      module ở repo riêng không phải sửa (D-167)

Nhóm B — việc nhật ký quyết định đã ghi "để sau":

- [ ] `auth`: đổi email thì gửi thêm mail báo tới địa chỉ cũ (kèm cách báo "không phải tôi" — vd
      khoá phiên và đặt lại mật khẩu), để chủ tài khoản biết khi tài khoản bị chiếm (đã ghi để sau
      ở mục về xác minh email trong `docs/DECISIONS.md`); mail qua job, không làm chậm request
- [ ] `auth`: quản trị viên xem và gỡ passkey của người khác (permission riêng, ghi event cho
      audit), và lệnh CLI gỡ passkey của một tài khoản cho khi admin duy nhất mất thiết bị —
      `autoxide user reset-mfa` (D-159) hiện chỉ tắt TOTP, không gỡ passkey
- [ ] `forms`: xuất CSV lớn bằng job ghi file vào `Storage`, trả link tải có hạn; xuất đồng bộ hiện
      tại giữ cho số bài nhỏ (cách xuất đang có: `modules/forms/src/csv.rs`)
- [ ] `comments`: Markdown giới hạn (đậm, nghiêng, link, trích dẫn, code) với bộ lọc HTML chống
      XSS — thêm thư viện thì đo và ghi `STACK.md`; giữ test XSS sẵn có; chữ thường cũ hiển thị như cũ

Chờ chủ dự án quyết: định dạng AVIF cho ảnh thu nhỏ (cần thư viện mã hoá AVIF, nặng hơn WebP).

## 33. Nền đa khách: khách hàng, vị trí dữ liệu, mặt điều khiển (D-185)

Thiết kế: `docs/research/business-foundation.md`, trụ 1. Bắt đầu SAU nhóm A của bước 32 (dời bố
cục module). Mọi mục chạy và có test trên cả SQLite và Postgres, ở cả chế độ `single` và nhiều khách.

- [ ] Mặt điều khiển: sổ khách hàng (trạng thái, khách cha), vị trí dữ liệu (`shared` /
      `dedicated`, database nào), tên miền → khách / nút; chế độ `single` mặc định (một khách,
      mặt điều khiển và dữ liệu chung một database, không cấu hình gì thêm)
- [ ] Ngữ cảnh khách hàng cho mọi request: suy từ tên miền, API key, access token, header của nhân
      sự nền tảng; sổ kết nối database theo khách (mở lười, có giới hạn); extractor cho handler,
      tool MCP; thiếu ngữ cảnh ở route cần khách thì lỗi rõ
- [ ] Phân loại bảng: mặt điều khiển hay cấp khách hàng; mọi bảng cấp khách hàng của 13 module có
      `tenant_id` (migration mới), khoá unique và index bắt đầu bằng nó; testkit và `cargo xtask
      check` chặn bảng cấp khách hàng thiếu `tenant_id`
- [ ] Mọi truy vấn của service lọc theo khách qua một công cụ của kernel; testkit có bài **rò rỉ
      giữa khách** chạy cho mọi module (ghi ở khách A, đọc bằng người của khách B: không thấy gì)
- [ ] Job, event, outbox, lịch chạy theo từng database; worker và bộ phát event duyệt mọi database
      được giao; `autoxide migrate --all` cho cả đội database; dời một khách `shared` ↔ `dedicated`
      bằng chép dữ liệu (mở rộng `autoxide copy`)
- [ ] Đo đường nóng trước và sau (`scripts/loadtest.sh`); hướng dẫn viết module (EN + VI) có quy
      tắc phân loại bảng và lọc theo khách
- [ ] Mã hoá dữ liệu nhạy cảm khi lưu (dời từ bước 37, D-190 — kho secret theo khách cần nó):
      secret TOTP của MFA (hiện lưu chữ thường trong `auth`), secret ký webhook và mọi secret module cần đọc lại được — mã hoá bằng khoá nằm ngoài
      database (dẫn xuất từ `AUTOXIDE_SECRET_KEY`), có số phiên bản khoá để xoay khoá; một công cụ
      dùng chung trong `kernel`, thuật toán chuẩn (AEAD) qua thư viện đã kiểm; migration mã hoá dữ
      liệu cũ; bản backup không còn chứa các secret này ở dạng đọc được. Khi `AUTOXIDE_SECRET_KEY`
      chưa đặt thì khoá gốc nằm trong database (`kernel/src/secret.rs`), mã hoá không bảo vệ được
      bản backup: `autoxide doctor` và log lúc khởi động nói rõ điều đó. Ghi quyết định
- [ ] Cấu hình và secret theo khách (D-190): thông tin kết nối đối tác của từng khách (cổng thanh
      toán, SMTP, hoá đơn điện tử, khoá AI) lưu mã hoá bằng cơ chế trên; module đọc qua một trait
      của kernel, không đọc biến môi trường cho thứ thuộc về khách
- [ ] Mọi thứ ngoài bảng cũng theo khách (D-190): khoá file lưu trữ có tiền tố khách (khách
      `dedicated` được bucket riêng); cache trong bộ nhớ (vd cache tìm theo ý nghĩa của `content`),
      rate limiting, login throttling, idempotency key, con trỏ pagination có khách trong khoá;
      token và URL đã ký mang khách, dùng ở khách khác thì bị từ chối
- [ ] Tên miền riêng của khách (D-190): xác minh quyền sở hữu bằng bản ghi DNS TXT, trạng thái tên
      miền, endpoint "có phục vụ tên miền này không" cho reverse proxy cấp chứng chỉ TLS tự động
      (vd Caddy on-demand TLS); tài liệu deploy (EN + VI)
- [ ] An toàn khi nhiều database (D-190): mỗi database ghi phiên bản migration của nó; database
      chưa migrate xong thì chỉ các khách trên nó nhận lỗi `tenant_maintenance` (503), các khách khác
      chạy bình thường; `migrate --all` chạy lại được từ chỗ dừng; worker chia lượt công bằng giữa
      các khách trong database chung (một khách không chiếm hết job queue)

## 34. Tổ chức, quyền theo độ sâu, danh tính (D-185)

Thiết kế: trụ 2. Bắt đầu SAU bước 33.

- [ ] Cây tổ chức trong khách hàng: loại nút tự đặt, độ sâu có giới hạn, bảng tổ tiên – con cháu,
      dời nút (khoá dòng), vòng đời nút (chờ duyệt, hoạt động, tạm khoá, lưu trữ — khoá một nút thì
      cả nhánh ngừng ghi)
- [ ] Thành viên và quyền có độ sâu `own` / `unit` / `subtree` / `tenant`; `Principal` mang quyền
      theo nút; công cụ lọc theo nhánh cho service; dữ liệu nghiệp vụ mang `org_id`; module mẫu
      `notes` dùng làm ví dụ; testkit có bài rò rỉ giữa hai nhánh anh em
- [ ] Cài đặt kế thừa theo cây (múi giờ, tiền tệ, ngôn ngữ, hồ sơ thuế, chính sách) và khuôn mẫu
      để module áp cùng cách cho dữ liệu của mình (vd giá: chuỗi đặt, cửa hàng ghi đè)
- [ ] Danh tính hai lớp: nhân sự là tài khoản toàn nền tảng (mặt điều khiển), một người làm cho
      nhiều khách / nút; người mua là tài khoản theo từng khách; API key gắn với khách và nút
- [ ] Uỷ quyền theo cây khách hàng: role ở khách cha có hiệu lực ở các khách con khi được cấp; vào
      ngữ cảnh khách con bằng cùng API; audit ghi ở cả hai phía
- [ ] Đăng nhập bằng link qua email (không cần mật khẩu): token dùng một lần, hết hạn ngắn, gắn
      thiết bị yêu cầu khi được; MFA vẫn áp dụng; trả lời như nhau về thời gian dù email có hay
      không (như D-154, D-162); tắt mặc định, bật bằng cấu hình
- [ ] Mời người dùng qua email: quản trị viên mời kèm role định sẵn, người được mời đặt mật khẩu
      (hay passkey) qua link dùng một lần; lời mời hết hạn, thu hồi được, có event cho audit; lệnh
      CLI tương ứng
- [ ] Nhân sự nền tảng vào dữ liệu của khách (D-190): chỉ khi có quyền vào cấp có thời hạn, ghi lý
      do; mọi lần vào ghi audit mà quản trị của khách xem được (Luật BVDLCN 2025); khách tắt được
      quyền vào hỗ trợ, trừ yêu cầu của cơ quan có thẩm quyền (có audit)

## 35. Gói, quyền dùng tính năng, hạn mức, thuê bao (D-185)

Thiết kế: trụ 3. Bắt đầu SAU bước 34. Giá, kỳ thanh toán, hạn mức là dữ liệu, không phải code.

- [ ] Module khai tính năng (`Module::features`) và hạn mức (`Module::limits`), mặc định rỗng
      (D-038); gói = tập module + tính năng + hạn mức + giá theo kỳ; dùng thử, gói thêm, ghi đè
      riêng cho từng khách; API và tool MCP quản trị gói
- [ ] Kernel cưỡng chế theo khách: module chưa bật thì route, tool MCP, job, event của nó bị chặn
      (`module_not_enabled`); `has_feature`; hạn mức giữ chỗ / trả lại nguyên tử; số đo dùng ghi
      lại cho tính tiền; test đồng thời trên cả hai database
- [ ] Vòng đời thuê bao: dùng thử → đang dùng → quá hạn (ân hạn) → tạm khoá (chỉ đọc) → huỷ (cho
      xuất dữ liệu) → xoá sau thời hạn lưu; thu hồi module giữ dữ liệu theo chính sách lưu; gia hạn
      và nhắc nợ bằng job
- [ ] Tính tiền thuê bao SaaS: hoá đơn theo kỳ, đánh số tuần tự, thu qua `PaymentGateway` (bước 36)
      bằng tài khoản của nhà vận hành nền tảng
- [ ] Cấp phát: khách tự đăng ký (kèm KYC khi gói yêu cầu, bước 37) → tạo khách → chọn vị trí dữ
      liệu theo gói → migrate → dữ liệu mẫu → mời quản trị; lệnh CLI và API tương ứng

## 36. Tiền, sổ cái, thuế, chứng từ (D-187, D-188)

Thiết kế: trụ 4. Bắt đầu SAU bước 34 (làm song song với 35 được; mục tính tiền của 35 dùng cổng
thanh toán của bước này). Core không bao giờ tự giữ tiền.

- [ ] Số thập phân `rust_decimal` cho tính toán (ghi `STACK.md`), quy tắc làm tròn theo từng chỗ
      dùng, chuyển qua lại với `Money` không mất đơn vị
- [ ] Module `ledger`: tài khoản theo khách / nút / người bán, bút toán nợ = có, chỉ thêm, đảo
      bút toán để sửa, số dư suy từ bút toán, khoá dòng khi ghi đồng thời; test đồng thời trên cả
      hai database
- [ ] Trait `PaymentGateway`, `PayoutGateway` trong kernel (như `TextModel`); nhận webhook của đối
      tác: kiểm chữ ký, ghi nguyên văn, xử lý qua job, không trùng (mã tham chiếu D-184); đối soát
      định kỳ sổ cái với sao kê đối tác; một bộ cài đặt giả cho test
- [ ] Ba chế độ dòng tiền theo khách / sàn: tự thu, thu hộ qua đối tác, chia tại cổng — mỗi chế độ
      sinh đúng bút toán (phải trả người bán, phí, thuế khấu trừ, tạm giữ tới hết hạn đổi trả)
- [ ] Engine thuế: nhóm hàng hoá, biểu thuế theo vùng thuế với ngày hiệu lực, giá gồm / chưa gồm
      thuế, hồ sơ thuế người bán, thuế khấu trừ thay; ảnh chụp thuế suất trên từng dòng chứng từ;
      gói dữ liệu "vn" (GTGT, mức giảm tạm thời, thuế tiêu thụ đặc biệt, tỉ lệ khấu trừ hộ kinh
      doanh) cập nhật bằng CLI / API; `autoxide doctor` cảnh báo thuế suất sắp hết hạn
- [ ] Chứng từ: đánh số tuần tự không nhảy số theo phạm vi (khách / người bán / năm / ký hiệu), an
      toàn đồng thời; trait `EInvoiceProvider` cho hoá đơn điện tử; phát hành thay người bán khi
      được uỷ quyền
- [ ] Lưu trữ và khoá sổ kế toán (D-190): chứng từ, sổ cái, hoá đơn giữ ít nhất 10 năm (Luật Kế
      toán 88/2015, Điều 41; NĐ 174/2016) — xoá khách hay yêu cầu xoá dữ liệu cá nhân thì phần kế
      toán được giữ lại (lưu trữ / xuất ra), phần còn lại xoá; khoá sổ theo kỳ (kỳ đã khoá không
      nhận bút toán, sửa bằng bút toán ở kỳ mở)
- [ ] Hoá đơn khởi tạo từ máy tính tiền (D-190): `EInvoiceProvider` hỗ trợ cả loại này cho bán lẻ
      tại quầy (NĐ 123/2020 sửa bởi NĐ 70/2025: hộ kinh doanh doanh thu từ 1 tỷ đồng / năm từ
      01/6/2025; ngưỡng theo lộ trình — để dạng dữ liệu, cần đối chiếu văn bản mới nhất)

## 37. Tuân thủ pháp luật Việt Nam (D-186)

Thiết kế: trụ 5 và mục 4 (đăng ký Bộ Công Thương). Bắt đầu SAU bước 34. Mỗi mục ghi căn cứ pháp lý
trong quyết định của nó; quy định hay đổi để dạng dữ liệu có ngày hiệu lực.

- [ ] Định danh (KYC): hồ sơ cá nhân / tổ chức, trait `IdentityVerifier` (VNeID qua nhà cung cấp,
      eKYC), trạng thái và hạn, lưu tối thiểu (kết quả và mã tham chiếu, hạn chế lưu ảnh giấy tờ,
      sinh trắc học); cổng chặn: nút loại người bán, người livestream, tiếp thị liên kết chưa xác
      thực thì không được bán / phát (Luật TMĐT 122/2025, Điều 17, từ 01/7/2026)
- [ ] Hồ sơ pháp lý website và logo Bộ Công Thương: hồ sơ chủ sở hữu theo khách và theo người bán;
      loại website (bán hàng → thông báo, logo xanh; nền tảng trung gian / sàn → đăng ký, logo đỏ)
      theo từng tên miền; danh sách kiểm tuân thủ là dữ liệu có phiên bản; API và `autoxide doctor`
      báo thứ còn thiếu; xuất bộ hồ sơ (dữ liệu tờ khai, nháp đề án, quy chế); lưu trạng thái và
      đoạn mã logo, endpoint công khai trả về cho giao diện
- [ ] Trang bắt buộc có mẫu (module `content`): thông tin chủ sở hữu, chính sách bảo vệ dữ liệu cá
      nhân, đổi trả, vận chuyển, thanh toán, kiểm hàng, giải quyết khiếu nại, điều khoản; với sàn
      thêm quy chế hoạt động, mẫu hợp đồng với người bán, thông tin người bán trên trang sản phẩm
      (EN + VI, nội dung mẫu ghi rõ cần người có chuyên môn pháp lý duyệt)
- [ ] Báo vi phạm sở hữu trí tuệ / hàng giả: tiếp nhận → ẩn → phản hồi của người bị báo → quyết
      định, có hạn xử lý, lưu vết; kênh khiếu nại của sàn; cung cấp thông tin người bán khi cơ quan
      có thẩm quyền yêu cầu (có audit)
- [ ] Xác thực tài khoản đăng nội dung (NĐ 147/2024): cờ "đã xác thực số điện thoại / định danh";
      khách bật yêu cầu cho bình luận, đánh giá, livestream
- [ ] Dữ liệu cá nhân (Luật 91/2025, NĐ 356/2025): sổ căn cứ xử lý / đồng ý theo mục đích; thời hạn
      lưu theo loại dữ liệu; vị trí lưu theo khách (lưu tại Việt Nam chọn được); quy trình sự cố có
      mốc 72 giờ kể từ khi phát hiện trong sổ tay vận hành
- [ ] Văn bản pháp lý có phiên bản và bằng chứng chấp nhận (D-190): điều khoản, chính sách dữ liệu
      cá nhân, quy chế sàn, hợp đồng với người bán, điều kiện giao dịch chung — mỗi bản có số phiên
      bản và ngày hiệu lực; ghi ai chấp nhận bản nào, lúc nào, qua kênh nào (Luật Giao dịch điện tử
      20/2023); bản mới thì yêu cầu chấp nhận lại khi cần
- [ ] Bảo vệ người tiêu dùng (D-190; Luật 19/2023/QH15 + NĐ 55/2024, từ 01/7/2024): nền tảng
      không chặn, không ẩn đánh giá trái quy định; minh bạch điều kiện giao dịch chung; kênh tiếp nhận
      và hạn xử lý khiếu nại của người tiêu dùng — cần đối chiếu văn bản gốc từng điều
- [ ] Gắn nhãn nội dung do AI tạo (D-190; Luật Trí tuệ nhân tạo 134/2025/QH15, từ 01/3/2026, và
      nghị định hướng dẫn — cần đối chiếu): nội dung tạo hay sửa từ gợi ý AI mang dấu nguồn gốc
      `ai_assisted`, API trả về để giao diện gắn nhãn khi công khai
- [ ] Gửi SMS (D-190): trait `SmsSender` (nhà cung cấp SMS có brandname), mã một lần qua SMS để xác
      thực số điện thoại theo NĐ 147/2024; một bộ cài đặt giả cho test

## 38. Cây khách hàng xuyên database và báo cáo gộp (D-185)

Thiết kế: trụ 2, phần xuyên database. Bắt đầu SAU bước 34 và 36 (báo cáo gộp cần số liệu tiền).

- [ ] Kho báo cáo gộp: event của các khách con đổ về khách cha (hay một database báo cáo) gần thời
      gian thực, mang `tenant_id`, `org_id`; báo cáo gộp nhiều cửa hàng, nhiều tiền tệ; người có quyền
      ở cha mới xem
- [ ] Cha thao tác trên con: vào ngữ cảnh con bằng cùng API (uỷ quyền của bước 34); job phân phối dữ
      liệu cha → con (vd danh mục, bảng giá) gọi service của con, chạy lại được, không trùng
- [ ] Tách một nhánh tổ chức thành khách riêng (database riêng) và gộp ngược lại, giữ ID và lịch sử;
      test trên cả hai database

## 39. Vận hành và hiệu năng (chủ dự án giao ngày 13/11/2026; số cũ 33, D-189)

Khi đã có người dùng thật: biết sớm khi hệ thống kẹt, không mất dữ liệu, xử lý sự cố không phải
đoán, và chịu được dữ liệu cỡ thật. Bắt đầu SAU bước 38 (nền đa khách): backup, số đo, cảnh báo theo từng khách hàng và từng database.
Làm nhóm A trước: số đo tồn đọng dùng để quan sát lúc chạy tải ở nhóm B. Mọi số đo ghi kèm máy,
ngày, cỡ dữ liệu; mọi thay đổi chạy và có test trên cả hai database; không đổi tính đúng lấy tốc độ.

Nhóm A — vận hành:

- [ ] Backup tự động theo lịch: job định kỳ (qua `Schedule` của kernel) tạo bản backup như
      `autoxide backup` (`crates/app/src/commands/backup.rs`) rồi ghi vào `Storage` (ổ đĩa hay
      S3), giữ N bản gần nhất, lịch và số bản cấu hình qua `Setting` (sinh lại `.env.example`);
      bản hỏng thì ghi log lỗi và đếm bằng metric. `autoxide doctor` cảnh báo khi bản mới nhất cũ
      hơn ngưỡng. Không giữ transaction ghi trong lúc backup (dùng `Database::begin_snapshot`)
- [ ] Diễn tập restore: test đầu-cuối backup → restore vào database trống → chương trình khởi
      động, dữ liệu và file khớp, trên cả hai database (cả chép chéo SQLite ↔ Postgres nếu
      `autoxide copy` hỗ trợ); `docs/DEPLOY.md` (EN + VI) có quy trình restore từng bước
- [ ] Số đo tồn đọng (gauge, cập nhật định kỳ, tên có tiền tố như luật code): job đang chờ và tuổi
      của job chờ lâu nhất theo module, job trong dead-letter queue, event chưa giao và tuổi của
      event cũ nhất, kết nối database đang dùng / còn rảnh. Hiện chỉ có bộ đếm cộng dồn
      (`jobs_failed_total`, `events_failed_total`…), không thấy được hàng đợi kẹt
- [ ] Dashboard và cảnh báo có sẵn: file dashboard Grafana và luật cảnh báo Prometheus trong repo
      (tỉ lệ 5xx, độ trễ p95, job chờ quá lâu, dead-letter tăng, event tồn, hết kết nối database,
      backup quá cũ), mỗi cảnh báo kèm câu "làm gì"; hướng dẫn dùng trong `docs/DEPLOY.md` (EN + VI)
- [ ] Xoay khoá không gián đoạn: `AUTOXIDE_AUTH_TOKEN_SECRET` (và các khoá ký khác) nhận thêm khoá
      cũ trong một khoảng chuyển tiếp — ký bằng khoá mới, kiểm bằng cả hai — để đổi khoá (định kỳ
      hay khi bị lộ) không đăng xuất mọi người; test; quy trình trong `docs/DEPLOY.md`
- [ ] Sổ tay xử lý sự cố trong `docs/DEPLOY.md` (EN + VI): hàng đợi kẹt, job trong dead-letter
      (`autoxide jobs retry`), database đầy, lộ khoá, mất admin cuối cùng (`autoxide user …`),
      nâng cấp khi nhiều máy chủ chạy hai phiên bản code cùng lúc (migration theo kiểu mở rộng
      trước, thu hẹp sau). Mỗi tình huống: dấu hiệu, lệnh kiểm, cách sửa

Nhóm B — hiệu năng ở quy mô thật (bài đo hiện có chạy trên database gần như trống, chỉ đi qua
`notes` và `auth`):

- [ ] Bộ dữ liệu lớn để đo: một lệnh (vd trong `xtask` hay `scripts/`) sinh nhanh dữ liệu cỡ thật
      trên cả hai database — vd 100 nghìn bài nhiều ngôn ngữ có danh mục, thẻ, tác giả; 50 nghìn
      ảnh kèm bản nhỏ; 1 triệu bình luận; vài nghìn người dùng — chỉ ghi qua service hay đúng định
      dạng bảng của module, không dùng cho dữ liệu thật
- [ ] Đo tải đường đọc công khai trên bộ dữ liệu đó, mở rộng `scripts/loadtest.sh`: một bài
      (`/public/resolve`), danh sách bài và theo danh mục (`/public/posts`, `/public/terms/…`),
      `/public/sitemap`, `/public/feed`, `/public/search`, ảnh và bản nhỏ, bình luận của bài
      (`/public/threads`, `/public/comments`). Đặt ngưỡng p95 mỗi đường, ghi vào `docs/BENCHMARKS.md`
- [ ] Rà index bằng `EXPLAIN` (SQLite `EXPLAIN QUERY PLAN`, Postgres `EXPLAIN ANALYZE`) cho mọi
      truy vấn danh sách, lọc, đếm của các module trên bộ dữ liệu lớn; index thiếu thêm bằng
      migration mới (không sửa migration cũ); ghi bảng truy vấn → kế hoạch trước / sau
- [ ] Nén response (gzip, brotli): đo cỡ và thời gian trên JSON thật (danh sách, `openapi.json`
      hơn 1 MB), rồi quyết nén trong app (`tower-http`) hay để reverse proxy làm; không nén lại
      file đã nén (ảnh). Ghi quyết định; sửa `docs/DEPLOY.md` (EN + VI) theo
- [ ] Cache cho CDN: `Cache-Control`, `ETag`, `Vary` đúng cho mọi endpoint công khai; file media có
      địa chỉ mang phiên bản thì cache dài hạn (`immutable`); nội dung riêng tư hay xem trước không
      bao giờ được cache chung; hướng dẫn đặt CDN trước Autoxide trong `docs/DEPLOY.md` (EN + VI)
- [ ] Tìm theo ý nghĩa khi dữ liệu lớn: đo cách duyệt hết vector hiện tại trên bộ dữ liệu lớn so
      với ngưỡng 50 ms của D-110 (`docs/BENCHMARKS.md`, mục "Tìm theo ý nghĩa"); vượt ngưỡng thì
      chọn cách giảm (vector nhỏ hơn, chia nhiều nhân, hay index sau một trait) chạy được trên cả
      hai database, ghi quyết định; không vượt thì ghi số đo và giữ nguyên
- [ ] Quan sát theo khách (D-190): log và trace mang trường `tenant_id`; **không** dùng `tenant_id`
      làm nhãn số đo (trái luật code về nhãn); lượng dùng theo khách lấy từ số đo dùng của bước 35

## 40. Năng lực còn thiếu của core (agent đề xuất; số cũ 34, D-189)

So với những gì một lõi backend hiện đại thường có sẵn, core còn thiếu sáu năng lực chung, không
gắn với ngành nào. Bắt đầu SAU bước 34 (cây tổ chức, danh tính): OIDC, SSE, tải thẳng lên S3 và rate limit phải theo khách hàng và tổ chức. Endpoint mới có OpenAPI và tool MCP
tương ứng; thêm thư viện thì kiểm đúng phiên bản và ghi `STACK.md`; mọi mục chạy và có test trên
cả SQLite và Postgres.

- [ ] Đăng nhập qua nhà cung cấp danh tính theo OpenID Connect (Google, Microsoft, GitHub qua
      OAuth 2.0, và một nhà cung cấp OIDC bất kỳ khai bằng cấu hình): PKCE, kiểm `state` / `nonce`,
      kiểm chữ ký `id_token` qua JWKS; liên kết với tài khoản sẵn có theo email đã xác minh, không
      tự gộp khi email chưa xác minh; MFA của Autoxide vẫn áp dụng; tắt mặc định, bật bằng cấu hình
- [ ] Tải file lớn thẳng lên nơi lưu: khi nơi lưu là S3, cấp link tải lên ký sẵn có hạn và giới hạn
      cỡ / loại, rồi một lệnh xác nhận kiểm file (cỡ, loại thật, hash) trước khi ghi bản ghi; nơi
      lưu là ổ đĩa thì đi qua máy chủ như cũ. Một trait trong `Storage`, `files` và `media` dùng
      chung; file tải lên mà không xác nhận được dọn bằng job
- [ ] Header bảo mật chung cho mọi response của API (`X-Content-Type-Options: nosniff`,
      `Referrer-Policy`, `X-Frame-Options` hay `frame-ancestors`, HSTS khi cấu hình chạy sau
      HTTPS, CSP chặt cho trang tài liệu API): một lớp trong `kernel/src/http/`, cấu hình được,
      có test; hai endpoint trả file đang tự đặt header thì dùng chung lớp này
- [ ] Đẩy event theo thời gian thực bằng Server-Sent Events — stream event do module phát, KHÔNG
      tự đẩy mọi thay đổi của bảng ("realtime theo bảng" mà `docs/research/frameworks.md` đã không
      mượn); ghi quyết định nêu khác biệt đó. Một endpoint stream các event mà người
      gọi có permission được xem (lọc theo tên event và theo quyền đọc bản ghi), nối lại bằng
      `Last-Event-ID` không mất event trong khoảng giữ, giới hạn số kết nối; chạy đúng khi nhiều
      máy chủ (đọc từ bảng event, không chỉ từ bộ nhớ một tiến trình)
- [ ] MCP đầy đủ hơn: resources (vd tài liệu của module, schema một loại bản ghi), prompts mẫu cho
      việc hay làm (viết nháp bài, kiểm duyệt bình luận) do module khai qua `Module`, kèm cài đặt
      mặc định rỗng (D-038); một trang mô tả cho agent tự khám phá API (vd `/llms.txt` trỏ tới
      OpenAPI, MCP và hướng dẫn), sinh từ code để không lệch
- [ ] Rate limit đúng khi chạy nhiều máy chủ: giới hạn theo API key (và tuỳ chọn theo địa chỉ) đếm
      chung qua database như `throttle` (D-116) hay một cách khác chạy trên cả hai database; đo
      chi phí mỗi request trước và sau; giữ đếm trong bộ nhớ làm mặc định nếu đo thấy đắt, ghi
      quyết định

## Sau core (chưa mở)

GraphQL · tách service — xem bảng "Chưa làm, đã cân nhắc" trong `STACK.md`.

Module thương mại (sau nền 33–38, D-185): sản phẩm, giỏ hàng, đơn hàng, kho, vận chuyển, sàn —
module nghiệp vụ, dùng `tenant_id` + `org_id` + engine thuế + sổ cái ngay từ đầu.

Chờ chủ dự án quyết (không làm khi chưa có quyết định): module thông báo (trong ứng dụng, web push)
· đăng nhập thay người dùng để hỗ trợ · định dạng ảnh AVIF. Đã cân nhắc và không làm: API dữ liệu tự
sinh từ bảng, plugin WASM nạp lúc chạy, trang quản trị dựng sẵn (`docs/research/frameworks.md`,
D-005, D-026). Thư viện số thập phân và module sổ cái: đã quyết ở D-187, D-188.
