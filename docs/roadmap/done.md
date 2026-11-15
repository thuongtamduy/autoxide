# Lộ trình — các bước đã xong (0–32)

Chuyển nguyên văn từ `docs/ROADMAP.md` theo D-228 để lộ trình chỉ còn việc đang mở. Lịch sử từng bước
xem `CHANGELOG.md` và `docs/DECISIONS.md`.

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

Bổ sung (D-202):

- [x] `alt` theo ngôn ngữ: bảng `media_alt_translations`, item trả `alts`, `PUT` / `DELETE
      /{id}/alts/{locale}`, tool `set_alt`, ngôn ngữ theo `AUTOXIDE_MEDIA_ALT_LOCALES`, `Media::find_many_in`
      lùi về `alt` mặc định; `content` tra ảnh công khai theo ngôn ngữ của bài; test trên cả hai database

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

Bổ sung (D-202, cho hỏi đáp sản phẩm của thiết kế giao diện 2):

- [x] Hỏi đáp trên `comments`: event vòng đời do module sở hữu đích khai (`Module::resource_events`, thay
      danh sách viết cứng), nhãn tác giả `seller` / `verified_buyer` qua bộ tính nhãn của kernel (tính lúc
      ghi, lưu cùng bình luận), loại đích chỉ cho tài khoản đã xác thực (điểm nối `PostingVerifier`, NĐ
      147/2024), tìm `q` đúng dấu xếp trước (`fetch_page_tiers`), số câu hỏi tách khỏi số bình luận, tool
      MCP `unanswered` / `answer`; test trên cả hai database

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

Đặc tả: [`docs/steps/30-public-launch.md`](steps/30-public-launch.md) — đọc cùng [`docs/steps/README.md`](steps/README.md).

Đưa Autoxide tới tay người lạ: repo công khai, CI và phát hành tự động, image Docker có chứng
thực, file của một dự án mã nguồn mở, ảnh nhẹ, deploy không gián đoạn, cài được chỉ bằng README.
Không đụng code của bước 29 (trừ `media` và `kernel/src/http/health.rs`), nên làm ngay được.
Tài liệu cho người ngoài viết tiếng Anh kèm bản tiếng Việt cùng nội dung.

Nhóm A — CI và phát hành (D-175, D-197, D-199; phần cần repo công khai ở bước 50):

- [x] Agent xác nhận gitleaks quét toàn lịch sử mọi nhánh xanh và không file nào chứa tên máy chủ,
      IP, đường dẫn riêng hay thông tin cá nhân (13/11/2026); email cá nhân trong metadata commit:
      chủ dự án chọn giữ nguyên (D-197)
- [x] Ba workflow có lại trigger tự chạy (`ci.yml` push lên `main` và pull request, `audit.yml`
      theo lịch, `release.yml` theo tag `vX.Y.Z`), bỏ ghi chú "TẮT tự chạy (D-107)"; mọi job chỉ
      chạy khi repo công khai, riêng tư thì bỏ qua không tốn phí (D-197)
- [x] `release.yml` đăng image `ghcr.io/<owner>/autoxide:X.Y.Z` và `:X.Y` cho `linux/amd64` và
      `linux/arm64` (máy build gốc, không giả lập) bằng `GITHUB_TOKEN`, kèm chứng thực nguồn gốc
      build và SBOM đã ký; `docs/DEPLOY.md` (EN + VI) có lệnh kiểm chứng thực; `docs/RELEASING.md`
      có hai đường: workflow khi công khai, phát hành từ máy khi còn riêng tư (D-199)

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
- [x] Thử cài như người lạ: máy hay container sạch, làm đúng README tiếng Anh theo đường build từ
      mã nguồn (cả SQLite và Postgres), ghi mọi chỗ vấp rồi sửa README / `docs/DEPLOY.md` (EN + VI
      cùng commit). Thử bằng image đã đăng: bước 50

## 31. Hợp đồng API và chống hồi quy (chủ dự án giao ngày 13/11/2026, D-177)

Đặc tả: [`docs/steps/31-api-contract.md`](../steps/31-api-contract.md) — đọc cùng [`docs/steps/README.md`](../steps/README.md).

Người viết giao diện (repo riêng, D-097), nhà phát triển ngoài và AI agent dùng API không gọi sai;
hợp đồng, tài liệu và database của bản cũ không bao giờ âm thầm lệch khỏi code. Bước này sửa
`xtask`, `testkit`, `gate.sh`, tài liệu và thêm thư mục SDK — làm ngay được; riêng test nâng cấp
cần image Docker của bản cũ (bước 30). Test chạy trên cả SQLite và Postgres; thư viện chỉ cho test
khai ở `[dev-dependencies]`.

Nhóm A — hợp đồng và người gọi API:

- [x] Gate chặn thay đổi phá vỡ của `/api/v1` (D-059, D-239): so `docs/openapi.json` với bản ở tag phát
      hành gần nhất (`git show vX.Y.Z:docs/openapi.json`) và báo đỏ khi xoá endpoint hay method,
      thêm tham số hay trường bắt buộc vào request, xoá trường hay đổi kiểu trong response, bỏ
      giá trị của enum. Endpoint đã khai deprecated và quá ngày `sunset` thì được bỏ. Viết bằng
      Rust trong `xtask` (thêm lệnh), có test cho từng loại thay đổi; thông báo nói rõ chỗ nào
      phá và cách làm đúng (endpoint mới + deprecate endpoint cũ)
- [x] SDK TypeScript sinh từ `docs/openapi.json` (D-177, D-243: trong repo, không đăng npm), đủ kiểu cho
      request, response, lỗi `Problem` và pagination; gate kiểm SDK khớp hợp đồng như cách test
      đang kiểm `openapi.json`. Gate không bắt người lạ cài Node: nếu công cụ sinh cần Node thì chạy
      qua image Docker ghim phiên bản (như gitleaks) hoặc chỉ kiểm trong CI — chọn và ghi quyết định
- [x] Hướng dẫn theo tình huống (D-244; EN + VI, thư mục mới trong `docs/`): website đọc bài và ảnh
      (`srcset`, `body_media`, ETag / 304, cache), đăng nhập và làm mới token, API key cho máy,
      gọi lại an toàn bằng `Idempotency-Key`, pagination, đọc lỗi (`code`, `errors`, `Retry-After`).
      Mỗi tình huống có ví dụ `curl` và TypeScript dùng SDK
- [x] Hướng dẫn nối AI agent qua MCP (D-244) (EN + VI), mở rộng đoạn ngắn trong README: tạo API key quyền
      tối thiểu cho agent, cấu hình Claude Code và client MCP khác, danh sách tool theo module và
      permission cần, giới hạn (rate limit theo API key, bản nháp chờ duyệt — D-109)
- [x] Ví dụ trong tài liệu được test (D-244): các ví dụ của hai mục trên chạy thật trên app dựng bằng
      `TestApp` trong gate (hay một lệnh `xtask` gate gọi), để tài liệu không lệch code
- [x] Hợp đồng tool MCP (D-233, D-240): `docs/mcp-tools.json` sinh từ code, `api-diff` chặn thay đổi phá vỡ của tool
      (bỏ tool, thêm tham số bắt buộc, `read_only` thành ghi…), tool cũ khai deprecated như endpoint
- [x] `cargo semver-checks` cho kernel và testkit trong quy trình phát hành (D-233, D-242): báo cáo khi còn `0.x`, chặn
      từ `1.0` hay khi công bố framework (giữ lý do D-167)

Nhóm B — chống hồi quy:

- [x] Test nâng cấp từ bản phát hành trước: dựng database bằng image Docker của bản trước, ghi dữ
      liệu mẫu qua API của nó, rồi chạy bản hiện tại trên database đó (migration, đọc lại dữ liệu,
      vài lệnh ghi) — trên cả SQLite và Postgres; chạy trong CI và trước mỗi lần phát hành
      (`cargo xtask upgrade-test`, D-246)
- [x] Migration thật của mọi module chạy vòng up → down → up trên database trống và trên database
      có dữ liệu, trên cả hai database (hiện chỉ có test cơ chế bằng bảng mẫu,
      `crates/kernel/tests/migration.rs`)
- [x] Property test cho các bộ kiểm đầu vào: thân bài ProseMirror (`content`), slug, email
      (`autoxide_kernel::email`), con trỏ pagination, CSV chống injection, chữ và link của bình
      luận (chống XSS) — sinh đầu vào ngẫu nhiên, không panic, giữ bất biến; thêm thư viện thì ghi
      `STACK.md`
- [x] Response thật khớp hợp đồng (D-241): test HTTP qua `TestApp` kiểm mỗi response (thân và mã) khớp
      schema của endpoint đó trong `docs/openapi.json`, để tài liệu không hứa điều code không làm
- [x] Hợp đồng quyền đọc được bằng máy (D-245; D-205, đặc tả 54 mục P1): mỗi operation OpenAPI có
      `x-permissions` hay `x-public`, sinh từ một khai báo cạnh handler; `Permission` có mức rủi ro
      (`read` / `write` / `destructive` / `sensitive_data` / `money`)
- [x] Test độ phủ phân quyền trong gate (D-205, D-245): mọi operation không công khai trả 401 khi không token và
      403 khi token không có quyền; operation thiếu khai báo quyền làm gate đỏ; tool MCP `read_only` chỉ
      đòi quyền mức `read`

## 32. Dọn bố cục và việc để lại (agent đề xuất ngày 13/11/2026)

Đặc tả: [`docs/steps/32-layout-cleanup.md`](../steps/32-layout-cleanup.md) — đọc cùng [`docs/steps/README.md`](../steps/README.md).

Bước 29 đã merge — làm được ngay; riêng dời file của `media` làm sau khi WebP (bước 30) merge vì
cùng sửa module đó. Làm nhóm A trước: nhóm B sửa đúng
các module nhóm A dời file. Mỗi mục có test trên cả hai database và dòng CHANGELOG.

Nhóm A — bố cục:

- [x] Áp quy ước "File layout" của hướng dẫn viết module cho `newsletter`, `comments`, `media`,
      `forms`: phần service chia theo tính năng, test dài thành thư mục `tests/`; không đổi hành vi,
      đếm số test trước và sau phải bằng nhau
- [x] `kernel`: gom các file nằm phẳng trong `src/` thành nhóm (vd hợp đồng cho module: `ai`,
      `mail`, `storage`, `search`, `privacy`…), giữ nguyên đường dẫn công khai bằng `pub use` để
      module ở repo riêng không phải sửa (D-167)

Nhóm B — việc nhật ký quyết định đã ghi "để sau":

- [x] `auth`: đổi email thì gửi thêm mail báo tới địa chỉ cũ (kèm cách báo "không phải tôi" — vd
      khoá phiên và đặt lại mật khẩu), để chủ tài khoản biết khi tài khoản bị chiếm (đã ghi để sau
      ở mục về xác minh email trong `docs/DECISIONS.md`); mail qua job, không làm chậm request
- [x] `auth`: quản trị viên xem và gỡ passkey của người khác (permission riêng, ghi event cho
      audit), và lệnh CLI gỡ passkey của một tài khoản cho khi admin duy nhất mất thiết bị —
      `autoxide user reset-mfa` (D-159) hiện chỉ tắt TOTP, không gỡ passkey
- [x] `forms`: xuất CSV lớn bằng job ghi file vào `Storage`, trả link tải có hạn; xuất đồng bộ hiện
      tại giữ cho số bài nhỏ (cách xuất đang có: `modules/forms/src/submissions/csv.rs`)
- [x] `comments`: Markdown giới hạn (đậm, nghiêng, link, trích dẫn, code) với bộ lọc HTML chống
      XSS — thêm thư viện thì đo và ghi `STACK.md`; giữ test XSS sẵn có; chữ thường cũ hiển thị như cũ

AVIF cho ảnh thu nhỏ: đã chuyển sang bước 39, đo trước rồi mới quyết (D-232).
