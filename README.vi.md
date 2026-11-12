# Autoxide

[English](README.md) · **Tiếng Việt**

Autoxide là một lõi backend mã nguồn mở viết bằng Rust. Core được dựng trước;
module nghiệp vụ cắm vào sau. Một bộ code chạy được trên **cả SQLite và
PostgreSQL**, có REST API mô tả bằng OpenAPI, và được thiết kế để AI agent dùng
được như con người.

Autoxide là nền cho sản phẩm mới. Nó không gắn với một công ty hay một ngành nào:
core không chứa khái niệm nghiệp vụ — những thứ đó thuộc về module.

## Trạng thái

Core đã hoàn chỉnh: **mọi bước của lộ trình đã xong** — khung dự án, nền, đăng nhập
cùng role và permission, đăng nhập hai bước, event và audit log, job queue, i18n và
idempotency, email, lưu file, MCP endpoint, module mẫu, công cụ và deploy, passkey, quản lý phiên, xoá tài khoản và xuất dữ liệu, webhook. Kế tiếp là
các module nghiệp vụ. Core có workspace Cargo, gate chất lượng, cấu hình, truy cập database, migration theo
module, hợp đồng module, máy chủ HTTP dưới `/api/v1` kèm OpenAPI, health check, log,
metrics, tracing và CLI. Module `auth` lưu người dùng
với mật khẩu hash bằng Argon2id và cho đăng nhập bằng access token sống ngắn cùng
refresh token xoay vòng (`/api/v1/auth/login`, `/refresh`, `/logout`, `/me`). Quyền truy
cập theo role: mỗi module khai permission của mình, role gom chúng lại
(`/api/v1/auth/roles`), và role `admin` dựng sẵn có tất cả. Quản trị viên đầu tiên
được tạo bằng dòng lệnh (`autoxide user create`); sau đó quản trị viên quản lý tài
khoản qua `/api/v1/auth/users`. Người dùng chỉ tự tạo được tài khoản khi đặt
`AUTOXIDE_AUTH_SELF_REGISTRATION=true`. Chương trình và AI agent dùng API key
(`/api/v1/auth/api-keys`): key hành động thay người tạo ra nó, chỉ trong các permission
ghi trên key, và được gửi như access token (`Authorization: Bearer axk_…`). Tạo key,
cũng như thêm passkey, phải nhập lại mật khẩu hiện tại; đặt lại mật khẩu thì mọi key và
passkey của tài khoản bị xoá. Người
dùng có thể bật đăng nhập hai bước bằng ứng dụng authenticator (TOTP) kèm recovery
code dùng một lần (`/api/v1/auth/me/mfa`). Mọi thay đổi được giữ trong audit log chỉ
thêm — ai, làm gì, bản ghi nào, lúc nào, trước và sau — xem tại `/api/v1/audit` với
permission `audit.read`. Job chạy nền cũng nằm trong database (không cần Redis): lỗi
thì thử lại với backoff, hỏng hẳn thì được giữ lại (`autoxide jobs failed`, `autoxide
jobs retry`), và chạy theo lịch khi module yêu cầu. `AUTOXIDE_PROCESS_ROLE` tách một
bản triển khai thành tiến trình API và tiến trình worker. Thông báo lỗi có tiếng Anh và
tiếng Việt, chọn bằng header `Accept-Language`. Lệnh ghi gửi kèm header
`Idempotency-Key` gửi lại an toàn: lần gửi lại nhận đúng kết quả lần đầu. Mail gửi qua
SMTP từ job queue (đặt lại mật khẩu, xác minh email). File lưu trên ổ đĩa hoặc S3. AI
agent gọi hệ thống như công cụ qua MCP ở `/mcp`.

Core gồm những gì, theo thứ tự dựng (xem [`docs/ROADMAP.md`](docs/ROADMAP.md)):

1. Nền: cấu hình, truy cập database, migration theo module, hợp đồng module, một
   kiểu lỗi chung, HTTP dưới `/api/v1`, OpenAPI, health check, log, metrics,
   tracing, CLI
2. Đăng nhập, role và permission, API key cho máy và AI agent
3. MFA (TOTP và recovery code)
4. Event và audit log chỉ thêm
5. Job queue lưu trong database (không cần Redis)
6. i18n (tiếng Anh, tiếng Việt) và idempotency key
7. Email, đặt lại mật khẩu, xác minh email
8. Lưu file (ổ đĩa cục bộ và S3)
9. MCP endpoint để AI agent gọi hệ thống như công cụ
10. Một module mẫu và hướng dẫn viết module
11. Công cụ và deploy: chép dữ liệu giữa SQLite và PostgreSQL, đo hiệu năng, Docker,
    backup và restore, CI
12. Passkey; 13–14. quản lý tài khoản, webhook, hợp đồng API chặt hơn
15. Thư viện media: ảnh, video, âm thanh, tài liệu trong thư mục lồng nhau, phục vụ
    công khai hay riêng tư ([`docs/MEDIA.vi.md`](docs/MEDIA.vi.md))
16. Nội dung: bài viết và trang nhiều ngôn ngữ, bản nháp và lịch sử, hẹn giờ, danh mục,
    thẻ, tác giả, menu, chuyển hướng, và endpoint công khai cho website
    ([`docs/CONTENT.vi.md`](docs/CONTENT.vi.md))
17. Biểu mẫu: form liên hệ, form đăng ký định nghĩa qua API, máy chủ kiểm câu trả lời,
    giữ lại spam, hộp thư có xuất CSV ([`docs/FORMS.vi.md`](docs/FORMS.vi.md))
18. Bản tin email: danh sách, xác nhận hai bước, huỷ một chạm, chiến dịch gửi theo lô có
    tạm dừng và tiếp tục, bounce, mail cho bài mới ([`docs/NEWSLETTER.vi.md`](docs/NEWSLETTER.vi.md))
19. Bình luận: bình luận hai cấp gắn vào đích bất kỳ, cho người dùng đăng nhập hoặc khách,
    kiểm duyệt theo pipeline cố định, báo cáo, cấm, mail báo trả lời ([`docs/COMMENTS.vi.md`](docs/COMMENTS.vi.md))

## Yêu cầu

- [Rust](https://rustup.rs) — `rustup` tự cài đúng toolchain ghim trong
  `rust-toolchain.toml` ở lần đầu bạn chạy `cargo`
- Một trình biên dịch C (SQLite được biên dịch từ mã nguồn và đóng gói kèm binary)
- Để chạy gate chất lượng đầy đủ: [Docker](https://docs.docker.com/get-docker/) và
  [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny)
  (`cargo install cargo-deny --locked`)

## Bắt đầu

```bash
git clone <repository-url> autoxide
cd autoxide

# Biên dịch và chạy giao diện dòng lệnh
cargo run -p autoxide -- --help

# Chạy test trên SQLite (không cần cài đặt gì thêm)
cargo test --workspace

# Chạy gate chất lượng đầy đủ: định dạng, lint, kiểm ranh giới, kiểm thư viện,
# test trên SQLite, test trên PostgreSQL (dựng trong một container Docker dùng xong bỏ)
./gate.sh
```

Nếu bạn sẽ commit, bật Git hook một lần. Hook quét secret trong phần thay đổi sắp
commit (khi đã cài [`gitleaks`](https://github.com/gitleaks/gitleaks)) và kiểm định
dạng code:

```bash
git config core.hooksPath .githooks
```

## Cấu hình

Mọi biến cấu hình đều có giá trị mặc định dùng được, nên bản clone mới chạy được
ngay mà không cần cấu hình gì. Cấu hình đọc từ biến môi trường và từ file `.env`
trong thư mục hiện tại; biến môi trường thắng.

```bash
# Kiểm cấu hình và xem mọi biến kèm giá trị đang có hiệu lực
cargo run -p autoxide -- config

# Bắt đầu cấu hình của riêng bạn từ các giá trị mặc định đã ghi sẵn
cp .env.example .env
```

Cấu trúc database do migration quản lý, mỗi module một bộ. Migration không bao giờ
tự chạy — hãy chạy một lần mỗi lần deploy:

```bash
cargo run -p autoxide -- migrate status          # cái gì đã áp dụng, cái gì còn chờ
cargo run -p autoxide -- migrate up              # áp dụng mọi thứ còn chờ
cargo run -p autoxide -- migrate down <module>   # gỡ migration mới nhất của một module
```

Tạo quản trị viên đầu tiên. Lệnh tự sinh một mật khẩu mạnh và in ra đúng một lần;
muốn tự chọn mật khẩu thì đưa nó vào standard input và thêm `--password-stdin`:

```bash
cargo run -p autoxide -- user create --email you@example.com --name "Your Name" --admin
```

Sau đó chạy máy chủ. Máy chủ từ chối chạy khi còn migration chưa áp dụng, và dừng êm
khi nhận Ctrl-C hoặc `SIGTERM` (request đang chạy có `AUTOXIDE_HTTP_SHUTDOWN_SECONDS`
giây để xong):

```bash
cargo run -p autoxide -- serve
```

Mặc định máy chủ nghe ở `127.0.0.1:8080`: kiểm bằng
`curl http://127.0.0.1:8080/health/ready`. Log ghi ra standard error. Đăng nhập và
xem tài khoản của mình:

```bash
curl -s http://127.0.0.1:8080/api/v1/auth/login -H 'Content-Type: application/json' \
  -d '{"email": "you@example.com", "password": "<mật khẩu>"}'
curl -s http://127.0.0.1:8080/api/v1/auth/me -H 'Authorization: Bearer <access_token>'
```

Hợp đồng của API là một tài liệu OpenAPI sinh ra từ code. Nó nằm trong repo ở
[`docs/openapi.json`](docs/openapi.json), in ra được bằng `autoxide openapi`, và máy
chủ đang chạy phục vụ nó ở `/api/v1/openapi.json`. Phần mô tả của tài liệu liệt kê
các quy ước mọi endpoint tuân theo (envelope của response, lỗi theo RFC 9457 với
`code` ổn định, pagination theo con trỏ, request id).

AI agent dùng cùng các chức năng đó như công cụ (tool) qua
[Model Context Protocol](https://modelcontextprotocol.io) ở `/mcp` (Streamable HTTP),
xác thực như API — thường dùng API key. Agent chỉ thấy và gọi được tool mà permission
của nó cho phép; mỗi tool chạy đúng code của endpoint REST tương ứng, nên lệnh ghi vào
audit log. Ví dụ với Claude Code:

```bash
claude mcp add --transport http autoxide http://127.0.0.1:8080/mcp --header "Authorization: Bearer <api key>"
```

Backup mọi thứ — dữ liệu của mọi module và mọi file đã lưu — vào một thư mục, rồi về
sau khôi phục vào một database mới, trống (với nơi lưu file bất kỳ). Database được đọc
trong một snapshot nhất quán, nên máy chủ vẫn chạy được trong lúc backup:

```bash
cargo run -p autoxide -- backup backups/2026-11-11
cargo run -p autoxide -- restore backups/2026-11-11
```

Chỉ khôi phục bản backup do chính bạn tạo: một bản backup là toàn bộ nội dung của máy
chủ.

Muốn chuyển sang PostgreSQL về sau (hay mang dữ liệu về SQLite), dừng máy chủ rồi chép
mọi thứ sang một database mới, trống; database đó được áp dụng mọi migration trước:

```bash
printf '%s' 'postgres://user:password@host:5432/autoxide' | cargo run -p autoxide -- copy --to-stdin
```

[`.env.example`](.env.example) liệt kê mọi biến kèm mô tả và giá trị mặc định. File
này sinh ra từ code nên không thể lỗi thời. Cấu hình sai được báo đầy đủ — mọi vấn
đề cùng một lượt, mỗi vấn đề nói rõ phải sửa gì — và lệnh thoát với mã 2. Secret
không bao giờ bị in ra.

Mặc định mỗi địa chỉ client được gửi 600 request mỗi phút
(`AUTOXIDE_HTTP_RATE_LIMIT_PER_MINUTE`), và một email bị khoá đăng nhập 15 phút sau 5
lần sai. Chạy sau reverse proxy thì đặt `AUTOXIDE_HTTP_CLIENT_IP_HEADER` (ví dụ
`X-Forwarded-For`) để phân biệt client theo địa chỉ của chính họ thay vì của proxy.

Người dùng cũng có thể đăng nhập bằng passkey (Face ID, vân tay, khoá bảo mật) thay cho
mật khẩu. Đặt `AUTOXIDE_AUTH_PASSKEY_RP_ID` bằng tên miền của trang (và
`AUTOXIDE_AUTH_PASSKEY_ORIGINS` khi ứng dụng web chạy ở tên miền con); người dùng đã đăng
nhập thêm passkey bằng `POST /api/v1/auth/me/passkeys/registration` và
`POST /api/v1/auth/me/passkeys`, rồi đăng nhập bằng `POST /api/v1/auth/login/passkey/options`
và `POST /api/v1/auth/login/passkey` — không cần email. `options` trả về đưa thẳng vào
`PublicKeyCredential.parseCreationOptionsFromJSON()` / `parseRequestOptionsFromJSON()` của
trình duyệt.

Người dùng có permission `files.upload` tải file lên bằng cách gửi nguyên nội dung tới
`POST /api/v1/files?name=report.pdf` kèm `Content-Type` của nó, rồi xem danh sách, tải
xuống (`GET /api/v1/files/{id}/content`) và xoá file của mình; `files.manage` với tới
file của mọi người. File tải lên tối đa 25 MiB (`AUTOXIDE_FILES_MAX_BYTES`) và, khi bạn
đặt `AUTOXIDE_FILES_ALLOWED_TYPES` (ví dụ `image/*,application/pdf`), chỉ thuộc các loại
đó.

File tải lên được lưu trong `data/files` cho tới khi bạn đặt `AUTOXIDE_FILES_S3_BUCKET`
(kèm `AWS_REGION`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, và `AWS_ENDPOINT_URL`
cho dịch vụ tương thích S3 như Cloudflare R2). Backup `data/files` cùng lúc với
database.

Mail được ghi thành file `.eml` vào `data/mail` cho tới khi bạn đặt
`AUTOXIDE_MAIL_SMTP_URL` (ví dụ `smtps://user:password@smtp.example.com:465`) và
`AUTOXIDE_MAIL_FROM`.

Người dùng quên mật khẩu gọi `POST /api/v1/auth/password-reset` kèm email và nhận qua
mail một token dùng một lần, có hạn 30 phút; rồi gọi
`POST /api/v1/auth/password-reset/confirm` kèm token và mật khẩu mới. Câu trả lời
giống nhau dù email có tài khoản hay không. Đặt `AUTOXIDE_AUTH_PASSWORD_RESET_URL` (ví
dụ `https://app.example.com/reset?token={token}`) để gửi đường dẫn tới trang của bạn
thay vì token trần.

Người dùng đã đăng nhập tự đổi mật khẩu bằng `POST /api/v1/auth/me/password` (phải
nhập mật khẩu hiện tại). Quản trị viên đặt được mật khẩu đầu tiên cho tài khoản tạo
không kèm mật khẩu (`PUT /api/v1/auth/users/{id}/password`), nhưng không thay được
mật khẩu người khác đã có.

Mỗi lần đăng nhập là một phiên, có tên là `session_id` trong response token. Người dùng
xem các phiên của mình — thiết bị và lần hoạt động gần nhất — ở
`GET /api/v1/auth/me/sessions`, đăng xuất một phiên bằng
`DELETE /api/v1/auth/me/sessions/{id}`, hoặc mọi phiên khác bằng
`DELETE /api/v1/auth/me/sessions?keep={session_id của mình}`. Người dùng tải mọi dữ liệu
máy chủ giữ về mình bằng `GET /api/v1/auth/me/export` và xoá tài khoản bằng
`DELETE /api/v1/auth/me` (phải nhập mật khẩu hiện tại); sau đó mọi module tự xoá dữ liệu
về người đó. Quản trị viên tìm người dùng bằng `GET /api/v1/auth/users?search=ada`, đăng
xuất một người ở mọi nơi (`DELETE /api/v1/auth/users/{id}/sessions`) và xoá tài khoản
(`DELETE /api/v1/auth/users/{id}`). Quản trị viên không làm được những việc này với người
có permission mà mình không có.

Hệ thống khác nhận event qua webhook: có permission `webhooks.manage` thì đăng ký một địa
chỉ `https://` cùng các event muốn nhận ở `POST /api/v1/webhooks` (danh sách event ở
`GET /api/v1/webhooks/events`). Mỗi event được gửi bằng một request `POST` có chữ ký theo
[Standard Webhooks](https://www.standardwebhooks.com), nên bên nhận kiểm được bằng thư
viện có sẵn và secret nhận lúc tạo. Lần gửi lỗi được thử lại trong khoảng hai giờ;
`GET /api/v1/webhooks/{id}/deliveries` cho biết đã gửi gì và cái gì lỗi.

Tài khoản mới nhận một mail đề nghị xác nhận địa chỉ; token trong đó (hạn 24 giờ)
được xác nhận bằng `POST /api/v1/auth/email-verification/confirm`, sau đó
`email_verified_at` cho biết lúc xác minh. Muốn đổi email, người dùng gọi
`POST /api/v1/auth/me/email` kèm địa chỉ mới và mật khẩu: email chỉ đổi khi địa chỉ
mới được xác nhận theo cùng cách. Đặt `AUTOXIDE_AUTH_EMAIL_VERIFICATION_URL` để mail
dẫn tới trang của bạn.

Khi chạy thật (production), đặt `AUTOXIDE_AUTH_TOKEN_SECRET` (khoá ký access token)
bằng một chuỗi ngẫu nhiên dài, giống nhau trên mọi máy chủ:

```bash
openssl rand -base64 48
```

Không đặt thì máy chủ tự sinh khoá lúc khởi động và ghi một cảnh báo vào log: đủ để
dùng thử Autoxide, nhưng client đã đăng nhập phải đổi access token sau mỗi lần khởi
động lại.

### Chạy test trên PostgreSQL của riêng bạn

Test chọn database theo biến `TEST_DATABASE_URL`. Không đặt nghĩa là SQLite. Để dùng
PostgreSQL 16 trở lên, trỏ biến này tới một database bất kỳ trên máy chủ mà người
dùng có quyền `CREATE DATABASE` — mỗi test tự tạo một database trống riêng và xoá
nó sau khi chạy xong:

```bash
TEST_DATABASE_URL=postgres://user:password@localhost:5432/postgres cargo test --workspace
```

`./gate.sh` làm việc này giúp bạn bằng Docker. Muốn gate dùng một máy chủ có sẵn
thì đặt `GATE_POSTGRES_URL` bằng một URL cùng dạng.

Test lưu file chạy thêm trên S3 khi `TEST_S3_ENDPOINT` trỏ tới một dịch vụ tương thích
S3 có bucket tên `autoxide-test`. Gate tự dựng một máy chủ S3 giả lập dùng xong bỏ
([moto](https://github.com/getmoto/moto)) bằng Docker; muốn dùng dịch vụ có sẵn thì
đặt `GATE_S3_ENDPOINT`.

## Bố cục dự án

| Đường dẫn        | Là gì                                                                   |
| ---------------- | ----------------------------------------------------------------------- |
| `crates/kernel`  | Nền mà mọi module dựa vào. Không biết module nào.                       |
| `crates/app`     | Binary `autoxide`: nơi duy nhất đăng ký module.                         |
| `crates/testkit` | Đồ dùng cho test: mỗi test một database riêng, trên SQLite hoặc PostgreSQL. |
| `modules/<tên>`  | Mỗi module một crate. `modules/auth`: tài khoản, đăng nhập, role, API key, đăng nhập hai bước. `modules/audit`: audit log chỉ thêm. `modules/idempotency`: nơi lưu idempotency key. `modules/mail`: gửi mail. `modules/files`: lưu file. `modules/webhooks`: webhook. |
| `xtask`          | Công cụ cho người phát triển: `cargo xtask check` kiểm ranh giới workspace. |
| `gate.sh`        | Gate chất lượng. Một thay đổi chỉ xong khi lệnh này xanh.               |

Module sở hữu bảng của mình, chỉ gọi nhau qua trait đã công bố, và không bao giờ
dùng foreign key xuyên module. Gate tự động kiểm các luật này.

## Tài liệu

- [`STACK.md`](STACK.md) — dùng thư viện nào và vì sao
- [`docs/ROADMAP.md`](docs/ROADMAP.md) — dựng gì, theo thứ tự nào
- [`docs/DECISIONS.md`](docs/DECISIONS.md) — nhật ký quyết định
- [`CLAUDE.md`](CLAUDE.md) — luật cho người đóng góp, là người hay AI agent
- [`CHANGELOG.md`](CHANGELOG.md) — thay đổi của từng bản phát hành (tiếng Anh); image
  Docker đã phát hành ở `ghcr.io/<owner>/autoxide`
- [`docs/DEPLOY.vi.md`](docs/DEPLOY.vi.md) — cài đặt và deploy: Docker Compose với
  PostgreSQL hoặc một binary với SQLite, reverse proxy, cập nhật, backup
- [`docs/MEDIA.vi.md`](docs/MEDIA.vi.md) — dùng thư viện media: tải lên, thư mục, link
  công khai, cache, thùng rác
- [`docs/CONTENT.vi.md`](docs/CONTENT.vi.md) — dùng module nội dung: bản nháp, đăng, trang
  và địa chỉ, ngôn ngữ, endpoint công khai cho website, xem trước
- [`docs/FORMS.vi.md`](docs/FORMS.vi.md) — dùng module biểu mẫu: định nghĩa form, hiện
  trên website, chống spam, hộp thư, yêu cầu dữ liệu theo email
- [`docs/NEWSLETTER.vi.md`](docs/NEWSLETTER.vi.md) — dùng module bản tin: danh sách, ô đăng ký,
  trang xác nhận và trang lựa chọn của website, chiến dịch, bounce, lưu ý pháp lý
- [`docs/COMMENTS.vi.md`](docs/COMMENTS.vi.md) — dùng module bình luận: đích, luồng và ô bình
  luận của website, kiểm duyệt, báo cáo và cấm, mail báo trả lời, lưu ý pháp lý
- [`docs/MODULE_GUIDE.vi.md`](docs/MODULE_GUIDE.vi.md) — cách viết một module, bắt đầu
  từ module mẫu [`modules/notes`](modules/notes) (mặc định không đăng ký)

Bốn tài liệu đầu điều hành dự án và được viết bằng tiếng Việt, thuật ngữ kỹ thuật
giữ nguyên tiếng Anh. Tài liệu cho người dùng (README này, hướng dẫn cài đặt và sử
dụng, mô tả API) viết bằng tiếng Anh, kèm bản dịch tiếng Việt song song.

## Đóng góp

Chạy `./gate.sh` trước khi commit; một thay đổi chỉ được nhận khi gate xanh trên cả
SQLite và PostgreSQL. Không bao giờ commit secret — chỉ `.env.example` được nằm
trong Git, `.env` thì không.

## License

Phát hành theo một trong hai license, bạn tự chọn:

- Apache License, phiên bản 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

Trừ khi bạn nêu rõ điều ngược lại, mọi đóng góp bạn chủ ý gửi để đưa vào dự án này,
theo định nghĩa trong license Apache-2.0, sẽ được cấp phép kép như trên, không kèm
thêm điều khoản hay điều kiện nào.
