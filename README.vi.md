# Autoxide

[English](README.md) · **Tiếng Việt**

Autoxide là một lõi backend mã nguồn mở viết bằng Rust. Core được dựng trước;
module nghiệp vụ cắm vào sau. Một bộ code chạy được trên **cả SQLite và
PostgreSQL**, có REST API mô tả bằng OpenAPI, và được thiết kế để AI agent dùng
được như con người.

Autoxide là nền cho sản phẩm mới. Nó không gắn với một công ty hay một ngành nào:
core không chứa khái niệm nghiệp vụ — những thứ đó thuộc về module.

**Backend thương mại sẵn cho thị trường Việt Nam.** Thuế (gói dữ liệu thuế có ngày hiệu lực), chứng từ
kế toán và hoá đơn, danh mục đơn vị hành chính chính thức cho địa chỉ và các quy định pháp luật (thương
mại điện tử, dữ liệu cá nhân, lưu trữ chứng từ) có sẵn. AI agent dùng được qua MCP và hợp đồng OpenAPI,
với cùng permission như con người. Core giữ trung lập, module cắm vào. Nhà cung cấp thanh toán, SMS,
vận chuyển và hoá đơn điện tử nằm sau các interface hiện mới đi kèm bộ giả lập: việc nối nhà cung cấp
thật vẫn chưa làm.

## Chạy thử bằng một lệnh

Chỉ muốn xem thử? `autoxide demo` không cần cài đặt gì: tự tạo một thư mục, database SQLite, secret ngẫu
nhiên và một quản trị viên, nạp dữ liệu tham chiếu, rồi in mật khẩu quản trị ra màn hình. Trang xem API mở
ở `/api/docs`. Không dùng cho production.

```bash
# Bằng Docker (không phải cài hay biên dịch gì):
docker run --rm -it -p 127.0.0.1:8080:8080 ghcr.io/thuongtamduy/autoxide:latest demo
# Hoặc bằng binary tải về (Linux, macOS, Windows: xem trang phát hành):
./autoxide demo
```

Cần bản phát hành sau 0.19.0. [`docs/GETTING-STARTED.vi.md`](docs/GETTING-STARTED.vi.md) có đường tải về, cách
giữ dữ liệu demo và các bước tiếp theo.

## Chạy nhanh

Một lệnh, không phải biên dịch gì: SQLite trong một Docker volume, bảng được tạo, có quản trị viên
đầu tiên và dữ liệu tham chiếu đã nạp. Bạn cần [Docker](https://docs.docker.com/get-docker/) (kèm
Compose) và `openssl`.

```bash
mkdir autoxide && cd autoxide
curl -fsSLO https://raw.githubusercontent.com/thuongtamduy/autoxide/main/compose.quickstart.yml
# Hai secret, giữ trong .env cạnh file đó (không commit)
printf 'AUTOXIDE_AUTH_TOKEN_SECRET=%s\nAUTOXIDE_SECRET_KEY=%s\n' \
  "$(openssl rand -base64 48 | tr -d '\n')" "$(openssl rand -base64 48 | tr -d '\n')" > .env

docker compose -f compose.quickstart.yml up -d
# Mật khẩu tự sinh của quản trị viên (xem bên dưới)
docker compose -f compose.quickstart.yml exec autoxide autoxide initial-password
```

**Mật khẩu của quản trị viên đầu tiên.** Lần chạy đầu, máy chủ tạo quản trị viên `admin@example.com`
với một mật khẩu tự sinh. Mật khẩu không bao giờ vào log (log hay được gom về hệ thống khác): nó được
ghi một lần vào file `data/initial-admin-password`, chỉ người dùng của máy chủ đọc được. Trong container
đó là `/var/lib/autoxide/data/initial-admin-password`, nằm trong Docker volume `data` (Compose đặt tên
`<thư mục>_data`), nên còn nguyên qua các lần khởi động lại. Lệnh trên in hai dòng của nó: email, rồi
mật khẩu. Log chỉ nói file ở đâu (`docker compose -f compose.quickstart.yml logs autoxide | grep "first
administrator"`). Nếu lệnh báo file "does not exist" thì quản trị viên đã có từ trước (volume đã dùng rồi),
hoặc bạn đã tự đặt `AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD`. Đăng nhập và đổi mật khẩu xong thì xoá file:
`docker compose -f compose.quickstart.yml exec autoxide autoxide initial-password --remove`.

Rồi mở <http://127.0.0.1:8080/> (tên, phiên bản và các đường dẫn) và đăng nhập. Trang xem API
(`/api/docs`, nạp script từ CDN) tắt trong bản chạy nhanh: thêm `AUTOXIDE_HTTP_API_DOCS=true` vào `.env`
rồi chạy lại `docker compose -f compose.quickstart.yml up -d` để bật.

```bash
curl -sS http://127.0.0.1:8080/api/v1/auth/login -H 'Content-Type: application/json' \
  -d '{"email":"admin@example.com","password":"<mật khẩu trong file>"}'
```

Các hướng dẫn trong [`docs/guides/`](docs/guides/README.vi.md) viết request này gọn hơn bằng
`--json '…'`, cần curl 7.82 trở lên (`curl --version`); curl cũ hơn thì viết
`-H 'Content-Type: application/json' -d '…'` thay vào.

Muốn tự chọn thì thêm `AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL=…` và `AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD=…`
vào `.env` trước lần chạy đầu. Tiếp theo: nối một AI agent qua MCP
([`docs/guides/mcp.vi.md`](docs/guides/mcp.vi.md#6-từ-chạy-nhanh-tới-một-agent-làm-việc-thật)). Chạy nhanh
cần bản phát hành sau 0.16.0; với PostgreSQL, nhiều máy chủ và HTTPS, xem
[`docs/DEPLOY.vi.md`](docs/DEPLOY.vi.md).

### Bản chạy nhanh có những gì

Image phát hành chứa mọi module của binary `autoxide` (mọi thư mục trong `modules/` trừ module mẫu
`notes`). Máy chủ mặc định chạy các module hạng core và chính thức; module hạng thử nghiệm (đánh dấu
\* bên dưới) tắt cho tới khi bạn nêu tên trong `AUTOXIDE_MODULES` — xem
[`docs/MODULES.vi.md`](docs/MODULES.vi.md) về các hạng và cách chọn.
`GET /api/v1/kernel/modules` liệt kê module đang chạy trên máy chủ của bạn. Mỗi module trả lời dưới
`/api/v1/<module>`.

| Mảng | Module |
| --- | --- |
| Tài khoản và quyền | `auth` (đăng nhập, role, API key, xác thực hai bước, passkey), `organizations` (khách, đơn vị, cửa hàng), `audit`, `idempotency` |
| Nền tảng | `files`, `mail`, `notifications`, `webhooks`, `ai` (tắt tới khi bạn cấu hình nhà cung cấp), `address` (đơn vị hành chính Việt Nam), `compliance`, `plans` (gói và thuê bao), `reporting`\*, `automations`\* |
| Website | `content` (bài viết, trang), `media`, `menus`, `forms`, `newsletter`\*, `comments`\*, `analytics`\* |
| Thương mại | `catalog`, `inventory`, `promotions`, `shipping`, `orders`, `customers`, `tax`, `ledger` (tiền), `invoices` |
| Dịch vụ | `bookings`\*, `support`\*, `conversations`\* |

Không phải bật gì để thử các module hạng core và chính thức, và module bạn không ghi dữ liệu thì
không tốn gì. Module nói chuyện với dịch vụ bên ngoài (nhà cung cấp AI, đối tác thanh toán hay vận
chuyển, SMTP) không gửi gì ra ngoài cho tới khi bạn cấu hình. Mỗi khách được dùng module nào do gói
của khách quyết ([`docs/PLANS.vi.md`](docs/PLANS.vi.md)); tập module máy chủ chạy chọn bằng
`AUTOXIDE_MODULES` ([`docs/MODULES.vi.md`](docs/MODULES.vi.md)), và xem danh sách cần permission
`kernel.modules.read` (quản trị viên có sẵn).

### Sửa một bản ghi: `If-Match`

Đọc một bản ghi trả phiên bản của nó trong header `ETag`. Gửi lại giá trị đó trong `If-Match` khi sửa
bản ghi, để hai người không ghi đè lên nhau: phiên bản cũ nhận `412`, và endpoint bắt buộc header này
trả `428` khi thiếu.

```bash
# Đọc bài viết và giữ header ETag (một số trong ngoặc kép, như "1763700000000000")
curl -sS -i http://127.0.0.1:8080/api/v1/content/entries/<id> -H 'Authorization: Bearer <access token>'
# Sửa bài viết kèm ETag đó
curl -sS -X PATCH http://127.0.0.1:8080/api/v1/content/entries/<id> \
  -H 'Authorization: Bearer <access token>' -H 'If-Match: <the ETag>' \
  -H 'Content-Type: application/json' -d '{"title":"A new title"}'
```

[`docs/guides/errors.vi.md`](docs/guides/errors.vi.md) cho thấy từng câu trả lời (`428`, `412`)
bằng ví dụ chạy thật.

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
ghi trên key, và được gửi như access token (`Authorization: Bearer axk1_…`; key ghi sẵn
khách của nó nên không cần header nào khác). Tạo key,
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
`Idempotency-Key` gửi lại an toàn: lần gửi lại nhận đúng kết quả lần đầu (trừ kết quả chứa
secret chỉ trả một lần, như API key mới, vốn không bao giờ được lưu: lần gửi lại nhận
`409 idempotency_result_not_kept`). Mail gửi qua
SMTP từ job queue (đặt lại mật khẩu, xác minh email). File lưu trên ổ đĩa hoặc S3. AI
agent gọi hệ thống như công cụ qua MCP ở `/mcp`, đọc resource (hướng dẫn của module, JSON
Schema) và prompt của nó, và tự tìm đường từ `/llms.txt`, trang sinh từ code.

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
    backup và restore, một lệnh kiểm tra (`./gate.sh`)
12. Passkey; 13–14. quản lý tài khoản, webhook, hợp đồng API chặt hơn
15. Thư viện media: ảnh, video, âm thanh, tài liệu trong thư mục lồng nhau, phục vụ
    công khai hay riêng tư ([`docs/MEDIA.vi.md`](docs/MEDIA.vi.md))
16. Nội dung: bài viết và trang nhiều ngôn ngữ, bản nháp và lịch sử, hẹn giờ, danh mục,
    thẻ, tác giả, menu, chuyển hướng, và endpoint công khai cho website
    ([`docs/CONTENT.vi.md`](docs/CONTENT.vi.md))
17. Tính năng AI, mặc định tắt, với nhà cung cấp bạn chọn (Claude, OpenAI, Gemini,
    Ollama…): gợi ý SEO và alt text, dịch bài vào bản nháp, tìm theo ý nghĩa, và permission
    giữ AI agent trong phần việc của nó ([`docs/AI.vi.md`](docs/AI.vi.md))
18. Biểu mẫu: form liên hệ, form đăng ký định nghĩa qua API, máy chủ kiểm câu trả lời,
    giữ lại spam, hộp thư có xuất CSV ([`docs/FORMS.vi.md`](docs/FORMS.vi.md))
19. Bản tin email: danh sách, xác nhận hai bước, huỷ một chạm, chiến dịch gửi theo lô có
    tạm dừng và tiếp tục, bounce, mail cho bài mới ([`docs/NEWSLETTER.vi.md`](docs/NEWSLETTER.vi.md))
20. Bình luận: bình luận hai cấp gắn vào đích bất kỳ, cho người dùng đăng nhập hoặc khách,
    kiểm duyệt theo pipeline cố định, báo cáo, cấm, mail báo trả lời ([`docs/COMMENTS.vi.md`](docs/COMMENTS.vi.md))
21. Menu của website: thanh menu chính, bảng xổ lớn, chân trang; mục trỏ tới địa chỉ hay tới đối
    tượng của module bất kỳ, tự lấy mục con, nhãn có hạn, endpoint công khai có cache
    ([`docs/MENUS.vi.md`](docs/MENUS.vi.md))
22. Tổ chức và danh tính: cây tổ chức trong mỗi khách (trụ sở, vùng, cửa hàng…) với quyền theo một
    nút hay một nhánh, cài đặt kế thừa theo cây, website riêng của từng nút, hồ sơ công khai của cửa
    hàng; tài khoản người mua tách khỏi nhân sự, lời mời, đăng nhập bằng link, uỷ quyền theo chuỗi
    khách, quyền vào hỗ trợ có thời hạn; danh mục đơn vị hành chính Việt Nam cho địa chỉ
    ([`docs/ADDRESS.vi.md`](docs/ADDRESS.vi.md))

## Chạy thử bằng Docker (không cần build)

Như mục [Chạy nhanh](#chạy-nhanh), làm từng bước bằng `docker run` (dùng được với mọi bản phát
hành): không phải biên dịch gì, SQLite nằm trong một Docker volume. Bạn cần [Docker](https://docs.docker.com/get-docker/) và `openssl`.
Image nặng khoảng 410 MB trên đĩa, máy chủ sẵn sàng khoảng một giây sau khi khởi động.

```bash
# Hai secret, tự sinh ở đây và giữ trong một file (dùng đúng giá trị này ở mọi lần chạy; không commit)
printf 'AUTOXIDE_AUTH_TOKEN_SECRET=%s\nAUTOXIDE_SECRET_KEY=%s\n' \
  "$(openssl rand -base64 48 | tr -d '\n')" "$(openssl rand -base64 48 | tr -d '\n')" > autoxide.env
chmod 600 autoxide.env

IMAGE=ghcr.io/thuongtamduy/autoxide:0.19.0   # một bản phát hành ở trang GitHub Releases
RUN="docker run --rm --env-file autoxide.env -v autoxide-data:/var/lib/autoxide/data"

# 1. Tạo bảng (máy chủ không khởi động được nếu chưa làm bước này; làm lại sau mỗi lần cập nhật)
$RUN $IMAGE migrate up
# 2. Quản trị viên đầu tiên: mật khẩu mạnh được in ra một lần
$RUN $IMAGE user create --email you@example.com --name "Your Name" --admin
# 3. Dữ liệu tham chiếu (địa chỉ, thuế, giới hạn khuyến mãi); chạy lại lần nữa vẫn an toàn
$RUN $IMAGE task address.import
$RUN $IMAGE task tax.import_pack
$RUN $IMAGE task promotions.load_legal_data
# 4. Chạy máy chủ (nó lắng nghe bên trong container; lệnh này mở ra chỉ trên máy này)
docker run -d --name autoxide --restart unless-stopped --env-file autoxide.env \
  -v autoxide-data:/var/lib/autoxide/data -p 127.0.0.1:8080:8080 $IMAGE serve

curl http://127.0.0.1:8080/health/ready     # {"data":{"status":"ok"}}
$RUN $IMAGE doctor                           # còn thiếu gì trước khi đưa lên thật
```

Giữ `-v autoxide-data:/var/lib/autoxide/data` ở mọi lệnh: database, file tải lên và mail nằm
trong volume đó, thiếu nó thì mất cùng container. Bản backup phải ghi vào một thư mục bạn gắn
vào, không phải một đường dẫn bên trong container (`mkdir backups && sudo chown 10001 backups`,
rồi thêm `-v "$PWD/backups:/backups"` và chạy `backup /backups/<tên>`). Đăng nhập như ở mục
[Cấu hình](#cấu-hình) (dùng `http://127.0.0.1:8080`). Với PostgreSQL, reverse proxy có HTTPS,
cập nhật và restore, xem [`docs/DEPLOY.vi.md`](docs/DEPLOY.vi.md).

**Không có Docker?** Build binary và chạy bằng tay: [`docs/DEPLOY.vi.md`, mục B](docs/DEPLOY.vi.md#b-một-binary-với-sqlite)
(SQLite) hay [B+](docs/DEPLOY.vi.md#b-một-binary-với-postgresql-từ-đầu) (PostgreSQL). Hai secret để trong `.env`,
`autoxide migrate up` chạy trước `autoxide serve`, và `AUTOXIDE_HTTP_ADDRESS` đổi địa chỉ nghe.

## Yêu cầu

Chỉ cần khi build từ mã nguồn (muốn chạy image đã phát hành thì xem mục trên).

- [Rust](https://rustup.rs) — `rustup` tự cài đúng toolchain ghim trong
  `rust-toolchain.toml` ở lần đầu bạn chạy `cargo`
- Một trình biên dịch C (SQLite và libwebp được biên dịch từ mã nguồn và đóng gói kèm binary;
  trên Debian hay Ubuntu: `sudo apt install build-essential`)
- Để chạy gate chất lượng đầy đủ: [Docker](https://docs.docker.com/get-docker/) và
  [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny)
  (`cargo install cargo-deny --locked`)

**Build mất khá lâu.** Lần build đầu (`cargo build --release`, hay `docker build`) biên dịch vài trăm
crate: tính khoảng 20 phút tới hơn một giờ, tuỳ số nhân CPU và bộ nhớ (RAM ít nhất 4 GB, 8 GB thì
thoải mái; build bị dừng giữa chừng trên Docker Desktop thì nâng giới hạn bộ nhớ của nó). Các lần sau
dùng lại phần đã biên dịch. Muốn chạy Autoxide chứ không sửa nó thì dùng image đã phát hành ở trên:
không phải biên dịch gì.

**Sau proxy của công ty.** `cargo` và `rustup` đọc `HTTPS_PROXY` (và `HTTP_PROXY`, `NO_PROXY`) từ biến
môi trường. `docker build` không chuyển biến của shell vào bước build: truyền chúng thành build
argument — Docker biết sẵn các tên này và không giữ chúng trong image —

```bash
docker build --build-arg HTTPS_PROXY=http://proxy.example.com:3128 \
  --build-arg HTTP_PROXY=http://proxy.example.com:3128 --build-arg NO_PROXY=localhost,127.0.0.1 \
  -t autoxide .
```

— hoặc đặt một lần trong mục `proxies` của `~/.docker/config.json`, Docker sẽ truyền cho mọi lần build,
kể cả `docker compose build`. Proxy soi HTTPS bằng chứng chỉ riêng làm việc tải thư viện trong lúc build
lỗi chứng chỉ; khi đó hãy dùng image đã phát hành.

## Bắt đầu

```bash
git clone https://github.com/thuongtamduy/autoxide.git autoxide
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

Nhập dữ liệu tham chiếu đi kèm máy chủ. Các bước này không đổi dữ liệu của bạn, và chạy
lại thì không đổi gì:

```bash
# Danh mục đơn vị hành chính Việt Nam (tỉnh, xã), dùng để kiểm địa chỉ.
# Không có gì tự nhập thay bạn: chạy một lần cho mỗi bản cài, và chạy lại khi bản cập nhật
# mang danh mục mới hơn (`doctor` sẽ nhắc). Xem docs/ADDRESS.vi.md.
cargo run -p autoxide -- task address.import

# Gói dữ liệu thuế (thuế suất, quy tắc khấu trừ). Máy chủ đang chạy tự nhập trong vòng
# một giờ; chạy lệnh này để có ngay. Xem docs/TAX.vi.md.
cargo run -p autoxide -- task tax.import_pack

# Giới hạn pháp lý về khuyến mãi (Việt Nam). Thiếu nó thì không bật được khuyến mãi
# (`doctor` sẽ cảnh báo). Xem docs/PROMOTIONS.vi.md.
cargo run -p autoxide -- task promotions.load_legal_data
```

Kiểm bản cài đặt. `doctor` đi qua cấu hình, database, migration, khoá ký, nơi lưu file,
máy chủ SMTP, tài khoản quản trị và dữ liệu tham chiếu đã nhập, và nói cách sửa từng lỗi (`--json` cho script và AI agent;
thoát với mã 1 khi có bước kiểm hỏng):

```bash
cargo run -p autoxide -- doctor
```

Module có thể thêm lệnh quản trị; `autoxide task` liệt kê chúng kèm tham số
(`cargo run -p autoxide -- task`).

Sau đó chạy máy chủ. Máy chủ từ chối chạy khi còn migration chưa áp dụng, và dừng êm
khi nhận Ctrl-C hoặc `SIGTERM` (request đang chạy có `AUTOXIDE_HTTP_SHUTDOWN_SECONDS`
giây để xong; sau bộ cân bằng tải thì xem
[cập nhật không làm rơi request](docs/DEPLOY.vi.md#cập-nhật-không-làm-rơi-request)):

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

[`docs/guides/mcp.vi.md`](docs/guides/mcp.vi.md) hướng dẫn từng bước: key với ít permission
nhất, client MCP khác, mọi tool kèm permission cần có, và các giới hạn.

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

Đăng nhập bằng Google, Microsoft, GitHub hay nhà cung cấp OpenID Connect bất kỳ tắt mặc định;
bật bằng `AUTOXIDE_AUTH_OIDC=true`, rồi mỗi khách thêm nhà cung cấp của mình bằng
`POST /api/v1/auth/oidc-providers` (permission `auth.oidc.manage`). Tài khoản sẵn có chỉ được liên
kết khi cả hai phía đã xác minh email, và đăng nhập hai bước vẫn áp dụng. Xem
[`docs/guides/oidc.vi.md`](docs/guides/oidc.vi.md).

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
`GET /api/v1/webhooks/{id}/deliveries` cho biết đã gửi gì và cái gì lỗi. Webhook chỉ gửi
tới địa chỉ public: địa chỉ trong mạng riêng hay máy này (`localhost`, `10.x`, `192.168.x`,
`169.254.x`…) bị từ chối, trừ khi đặt `AUTOXIDE_WEBHOOKS_ALLOW_PRIVATE_NETWORKS=true` vì máy
nhận chạy trong mạng của bạn.

Tài khoản mới nhận một mail đề nghị xác nhận địa chỉ; token trong đó (hạn 24 giờ)
được xác nhận bằng `POST /api/v1/auth/email-verification/confirm`, sau đó
`email_verified_at` cho biết lúc xác minh. Muốn đổi email, người dùng gọi
`POST /api/v1/auth/me/email` kèm địa chỉ mới và mật khẩu: email chỉ đổi khi địa chỉ
mới được xác nhận theo cùng cách. Khi đó địa chỉ cũ nhận một mail báo, kèm token (hạn 7
ngày) để báo một lần đổi mà chủ tài khoản không làm: `POST /api/v1/auth/email-change/report`
đăng xuất tài khoản ở mọi nơi, bỏ mật khẩu, passkey, API key và xác thực hai bước, trả lại địa
chỉ cũ khi nó còn trống, rồi gửi mail đặt lại mật khẩu tới đó. Đặt
`AUTOXIDE_AUTH_EMAIL_VERIFICATION_URL` và `AUTOXIDE_AUTH_EMAIL_CHANGE_REPORT_URL` để các mail
này dẫn tới trang của bạn.

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

Test dựng cả ứng dụng chép một database mẫu đã chạy sẵn mọi migration, thay vì
migrate lại từng database trống. Mẫu nằm trong thư mục tạm của hệ thống
(`autoxide-templates/`) và, trên PostgreSQL, là các database tên `autoxide_tpl_…`;
cả hai được dựng lại khi chương trình test hay migration đổi, và bị xoá sau một giờ.
Đặt `TEST_DATABASE_TEMPLATE=off` để mọi database test migrate từ đầu.

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
| `modules/<tên>` | Mỗi module một crate, sở hữu bảng của mình (`auth` tài khoản và đăng nhập, `audit`, `files`, `mail`, `webhooks`, `content`, `media`, `catalog`, `orders`… — khoảng ba mươi module; `ls modules` liệt kê hết, các mục ở trên nói từng module làm gì). |
| `xtask`          | Công cụ cho người phát triển: `cargo xtask check` kiểm ranh giới workspace. |
| `gate.sh`        | Gate chất lượng. Một thay đổi chỉ xong khi lệnh này xanh.               |

Module sở hữu bảng của mình, chỉ gọi nhau qua trait đã công bố, và không bao giờ
dùng foreign key xuyên module. Gate tự động kiểm các luật này.

## Tài liệu

- [`STACK.md`](STACK.md) — dùng thư viện nào và vì sao
- [`docs/ROADMAP.md`](docs/ROADMAP.md) — dựng gì, theo thứ tự nào
- `docs/decisions/` — nhật ký quyết định (mục cũ ở [`docs/DECISIONS.md`](docs/DECISIONS.md), mục mới mỗi mục
  một file ở đây); `cargo xtask decisions` sinh mục lục theo chủ đề `INDEX.md`, file này không commit
- [`CLAUDE.md`](CLAUDE.md) — luật cho người đóng góp, là người hay AI agent
- [`CHANGELOG.md`](CHANGELOG.md) — thay đổi của từng bản phát hành (tiếng Anh); image
  Docker đã phát hành ở `ghcr.io/<owner>/autoxide`
- [`docs/COMPATIBILITY.vi.md`](docs/COMPATIBILITY.vi.md) — thứ giữ ổn định từ 1.0 (API, mã lỗi, cấu hình,
  migration, Rust API cho module) và thứ có thể đổi
- [`docs/guides/`](docs/guides/README.vi.md) — hướng dẫn API theo tình huống, có ví dụ `curl` và
  TypeScript chạy như test: website, đăng nhập, API key, gọi lại an toàn, pagination, lỗi,
  nối AI agent qua MCP
- [`sdk/typescript/`](sdk/typescript/README.vi.md) — client TypeScript có kiểu, sinh từ
  `docs/openapi.json`; [`docs/mcp-tools.json`](docs/mcp-tools.json) — hợp đồng của mọi tool MCP
- [`docs/DEPLOY.vi.md`](docs/DEPLOY.vi.md) — cài đặt và deploy: Docker Compose với
  PostgreSQL hoặc một binary với SQLite, reverse proxy, cập nhật, backup
- [`docs/MEDIA.vi.md`](docs/MEDIA.vi.md) — dùng thư viện media: tải lên, thư mục, link
  công khai, cache, thùng rác
- [`docs/CONTENT.vi.md`](docs/CONTENT.vi.md) — dùng module nội dung: bản nháp, đăng, trang
  và địa chỉ, ngôn ngữ, endpoint công khai cho website, xem trước
- [`docs/AI.vi.md`](docs/AI.vi.md) — tính năng AI: bật với từng nhà cung cấp, embedding, chi
  phí và giới hạn, gửi gì đi, giới hạn cho AI agent
- [`docs/FORMS.vi.md`](docs/FORMS.vi.md) — dùng module biểu mẫu: định nghĩa form, hiện
  trên website, chống spam, hộp thư, yêu cầu dữ liệu theo email
- [`docs/NEWSLETTER.vi.md`](docs/NEWSLETTER.vi.md) — dùng module bản tin: danh sách, ô đăng ký,
  trang xác nhận và trang lựa chọn của website, chiến dịch, bounce, lưu ý pháp lý
- [`docs/COMMENTS.vi.md`](docs/COMMENTS.vi.md) — dùng module bình luận: đích, luồng và ô bình
  luận của website, kiểm duyệt, báo cáo và cấm, mail báo trả lời, lưu ý pháp lý
- [`docs/MENUS.vi.md`](docs/MENUS.vi.md) — dùng module menu: mục, đối tượng của module khác, nhãn,
  endpoint công khai và cache, chuyển từ menu của module nội dung
- [`docs/BOOKINGS.vi.md`](docs/BOOKINGS.vi.md) — đặt lịch dịch vụ: dịch vụ, nhân viên và phòng, lịch
  tuần theo múi giờ của nút, giờ trống, cọc qua đối tác thanh toán, chính sách huỷ, nhắc lịch, khách
  vãng lai đặt bằng mã điện thoại
- [`docs/CUSTOMERS.vi.md`](docs/CUSTOMERS.vi.md) — khách hàng: hồ sơ theo tài khoản hay số điện thoại, lịch
  sử mua từ đơn hàng, che liên hệ và xem đủ có audit, nhóm tay và nhóm theo quy tắc
- [`docs/SUPPORT.vi.md`](docs/SUPPORT.vi.md) — phiếu hỗ trợ khách hàng: khách vãng lai dùng token phiếu,
  phạm vi nhân viên theo nút, nhóm và phân công, hạn trả lời theo giờ làm, chuyển khiếu nại, bản nháp AI
- [`docs/ANALYTICS.vi.md`](docs/ANALYTICS.vi.md) — phân tích truy cập không cookie: gửi sự kiện từ
  website, báo cáo, phễu, đồng ý cho theo dõi theo người, ghi chú pháp lý
- [`docs/ADDRESS.vi.md`](docs/ADDRESS.vi.md) — đơn vị hành chính và địa chỉ: đọc danh mục, nhập
  danh mục chính thức
- [`docs/CATALOG.vi.md`](docs/CATALOG.vi.md) — sản phẩm: biến thể, danh mục, bảng giá có lịch sử,
  sàn giá
- [`docs/PROMOTIONS.vi.md`](docs/PROMOTIONS.vi.md) — khuyến mại: giảm tự động, coupon, voucher, cộng dồn,
  giữ lượt cho đơn hàng, hạn mức pháp luật là dữ liệu có ngày hiệu lực
- [`docs/TAX.vi.md`](docs/TAX.vi.md) — thuế: gói dữ liệu thuế Việt Nam, tính thuế, hồ sơ thuế
- [`docs/MONEY.vi.md`](docs/MONEY.vi.md) — tiền: sổ cái, thanh toán và chế độ dòng tiền, VietQR, chứng từ kế
  toán, tỉ giá, in chứng từ, xuất dữ liệu kế toán, lưu chứng từ 10 năm
- [`docs/NOTIFICATIONS.vi.md`](docs/NOTIFICATIONS.vi.md) — thông báo: loại và nhóm, kênh, đồng ý và
  khung giờ của tin quảng cáo, hộp thư và tuỳ chọn, khai một loại trong module
- [`docs/MODULE_GUIDE.vi.md`](docs/MODULE_GUIDE.vi.md) — cách viết một module, bắt đầu
  từ module mẫu [`modules/notes`](modules/notes) (mặc định không đăng ký)

Bốn tài liệu đầu điều hành dự án và được viết bằng tiếng Việt, thuật ngữ kỹ thuật
giữ nguyên tiếng Anh. Tài liệu cho người dùng (README này, hướng dẫn cài đặt và sử
dụng, mô tả API) viết bằng tiếng Anh, kèm bản dịch tiếng Việt song song.

## Đóng góp

Trong lúc làm, `./gate.sh --quick` kiểm một thay đổi ở một module trong chưa tới một phút: nó chạy test
của các crate bạn đã sửa (và của các crate phụ thuộc vào chúng), chỉ trên SQLite. Trước khi
merge, chạy `./gate.sh` đầy đủ; một thay đổi chỉ được nhận khi gate đầy đủ xanh trên cả
SQLite và PostgreSQL. Không bao giờ commit secret — chỉ `.env.example` được nằm
trong Git, `.env` thì không. [`CONTRIBUTING.vi.md`](CONTRIBUTING.vi.md) hướng dẫn chuẩn
bị máy, những luật quan trọng nhất và cách gửi thay đổi.

Mỗi commit có một dòng `Signed-off-by` ([DCO](DCO), `git commit -s`). Câu hỏi: [`SUPPORT.vi.md`](SUPPORT.vi.md). Quyết
định được đưa ra thế nào và cách đề xuất thay đổi: [`GOVERNANCE.vi.md`](GOVERNANCE.vi.md). Dự án đang đi đâu (tiếng Anh):
[`docs/ROADMAP.public.md`](docs/ROADMAP.public.md). Xây và đăng module của riêng bạn:
[`docs/MODULE_ECOSYSTEM.vi.md`](docs/MODULE_ECOSYSTEM.vi.md). Dùng tên dự án: [`TRADEMARK.vi.md`](TRADEMARK.vi.md).

## Bảo mật

Tìm thấy lỗ hổng? Đừng mở issue công khai: hãy báo riêng theo
[`SECURITY.vi.md`](SECURITY.vi.md), nơi cũng ghi những phiên bản được vá bảo mật.
[`docs/THREAT_MODEL.vi.md`](docs/THREAT_MODEL.vi.md) mô tả các ranh giới tin cậy của core, control ở từng
ranh giới và những gì còn mở.

## License

Phát hành theo một trong hai license, bạn tự chọn:

- Apache License, phiên bản 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

Trừ khi bạn nêu rõ điều ngược lại, mọi đóng góp bạn chủ ý gửi để đưa vào dự án này,
theo định nghĩa trong license Apache-2.0, sẽ được cấp phép kép như trên, không kèm
thêm điều khoản hay điều kiện nào.
