# Cài đặt và deploy Autoxide

[English](DEPLOY.md)

Hướng dẫn này đưa bạn từ con số không tới một máy chủ đang chạy, cập nhật được và có
backup. Chọn một trong hai cách chạy:

| | Docker Compose với PostgreSQL | Một binary với SQLite |
| --- | --- | --- |
| Hợp với | Production, nhiều người ghi cùng lúc, nhiều máy chủ | Bản cài nhỏ trên một máy, dùng thử Autoxide |
| Cần có | Docker kèm plugin Compose | Rust (để build) hoặc một binary đã build; không cần gì khác |
| Database | PostgreSQL 16 trở lên, trong container | Một file, `data/autoxide.sqlite` |

Bạn có thể bắt đầu với SQLite rồi chuyển sang PostgreSQL về sau mà không mất dữ liệu
(xem [Chuyển từ SQLite sang PostgreSQL](#chuyển-từ-sqlite-sang-postgresql)).

## A. Docker Compose với PostgreSQL

### 1. Lấy mã nguồn và đặt secret

```bash
git clone <URL của repository> autoxide
cd autoxide
```

Tạo file `.env` cạnh `compose.yml` chứa ba secret. Không bao giờ commit file này.

```bash
printf 'POSTGRES_PASSWORD=%s\nAUTOXIDE_AUTH_TOKEN_SECRET=%s\nAUTOXIDE_SECRET_KEY=%s\n' \
  "$(openssl rand -hex 24)" "$(openssl rand -base64 48 | tr -d '\n')" \
  "$(openssl rand -base64 48 | tr -d '\n')" > .env
chmod 600 .env
```

- `POSTGRES_PASSWORD` bảo vệ database.
- `AUTOXIDE_AUTH_TOKEN_SECRET` ký access token. Giữ nguyên giá trị: đổi nó thì mọi người
  bị đăng xuất.
- `AUTOXIDE_SECRET_KEY` ký các token sống ngắn (biểu mẫu, chống spam), làm khoá băm địa chỉ
  IP của người lạ và mã hoá các secret máy chủ giữ (đăng nhập hai bước, secret ký webhook). Để
  ngoài database thì bản sao database (bản backup) không làm lộ các địa chỉ đó, cũng không làm
  lộ các secret đó.
- Muốn đổi `AUTOXIDE_SECRET_KEY` về sau: đặt giá trị mới vào nó, giá trị cũ vào
  `AUTOXIDE_SECRET_KEY_PREVIOUS`, khởi động lại, chạy `autoxide secrets reseal`, rồi bỏ
  `AUTOXIDE_SECRET_KEY_PREVIOUS`. Lần đầu đặt biến này trên máy chủ đã chạy mà chưa có nó thì
  không cần làm gì thêm: khoá máy chủ đã giữ trong database vẫn mở được thứ nó đã mã hoá.

### 2. Khởi động

```bash
docker compose up -d --build
```

Hoặc khỏi build, dùng image đã phát hành (xem các bản phát hành trên GitHub của dự án):

```bash
echo 'AUTOXIDE_IMAGE=ghcr.io/<owner>/autoxide:<version>' >> .env
docker compose up -d --no-build
```

Image đã phát hành chạy trên `linux/amd64` và `linux/arm64` (vd AWS Graviton hay Raspberry
Pi 4 trở lên). `<version>` là một bản phát hành như `0.12.0`, hoặc `0.12` để nhận theo các bản
sửa lỗi của nó. Mỗi image do workflow `Release` của dự án trên GitHub build, kèm chứng thực
nguồn gốc build đã ký và danh sách thành phần (SBOM) đã ký. Trước khi chạy một image, kiểm nó
được build từ đúng repo này bằng [GitHub CLI](https://cli.github.com/):

```bash
gh attestation verify oci://ghcr.io/<owner>/autoxide:<version> --owner <owner>
```

Lệnh in ra workflow, commit và tag đã build image, và báo lỗi nếu image không do repo này
build. SBOM (SPDX) được đính vào mỗi bản phát hành trên GitHub với tên `sbom.spdx.json`; kiểm
nó thuộc đúng image và đọc nó bằng:

```bash
gh attestation verify oci://ghcr.io/<owner>/autoxide:<version> --owner <owner> \
  --predicate-type https://spdx.dev/Document/v2.3 --format json \
  | jq '.[0].verificationResult.statement.predicate.packages[].name'
```

Compose khởi động PostgreSQL, áp dụng mọi migration một lần (service `migrate`), rồi
khởi động máy chủ ở `127.0.0.1:8080`. Kiểm tra:

```bash
curl http://127.0.0.1:8080/health/ready
```

`{"data":{"status":"ok"}}` nghĩa là máy chủ và database đã sẵn sàng.

### 3. Tạo quản trị viên đầu tiên

```bash
docker compose run --rm app user create --email you@example.com --name "Tên của bạn" --admin
```

Một mật khẩu mạnh được in ra đúng một lần; hãy lưu vào trình quản lý mật khẩu. Đăng
nhập bằng `POST /api/v1/auth/login`; toàn bộ API được mô tả ở
`http://127.0.0.1:8080/api/v1/openapi.json`.

### 4. Đưa lên internet

Máy chủ chỉ nói HTTP thường ở `127.0.0.1:8080`. Đặt một reverse proxy phía trước để lo
TLS. Với [Caddy](https://caddyserver.com) (tự lấy chứng chỉ), toàn bộ cấu hình
(`/etc/caddy/Caddyfile`) là:

```text
api.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

Rồi cho Autoxide biết header nào chứa địa chỉ thật của client, để rate limiting đếm theo
client chứ không theo proxy. Thêm vào `.env` rồi khởi động lại:

```bash
echo 'AUTOXIDE_HTTP_CLIENT_IP_HEADER=X-Forwarded-For' >> .env
docker compose up -d
```

**Nhiều hơn một proxy.** Mỗi proxy thêm địa chỉ của bên kết nối tới nó vào cuối
`X-Forwarded-For`; phần đứng trước là do client gửi và giả được. Đặt
`AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT` bằng số proxy của bạn mà một request đi qua (mặc định
`1`). Trường hợp hay gặp là máy chủ website (như Next.js) gọi API thay cho khách, đứng sau
CDN:

```text
trình duyệt ──▶ CDN ──▶ máy chủ website ──▶ Caddy ──▶ Autoxide
                thêm     thêm địa chỉ        thêm địa chỉ
                trình    CDN                 máy chủ website
                duyệt
X-Forwarded-For:  trình-duyệt, CDN, máy-chủ-website
                  ▲ thứ 3 từ cuối → AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT=3
```

Máy chủ website phải chuyển tiếp `X-Forwarded-For` nó nhận được (và thêm địa chỉ CDN, như
proxy vẫn làm); không thì mọi khách trông như một client. Khi đếm proxy, không proxy nào của
bạn được để internet gọi thẳng, không thì client tự chọn được địa chỉ của mình.

**Proxy mà internet gọi thẳng được.** Khi khách có thể bỏ qua CDN mà gọi thẳng máy chủ
website, hoặc request đi theo nhiều đường khác nhau, đếm proxy sẽ sai với một số request. Khi
đó hãy liệt kê địa chỉ các proxy của bạn, từng địa chỉ hoặc theo dải:

```bash
echo 'AUTOXIDE_HTTP_TRUSTED_PROXIES=10.0.0.0/8, 198.51.100.0/24' >> .env
```

Biến này thay cho `AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT`. Autoxide chỉ đọc header khi kết nối đến
từ một trong các địa chỉ đó, và lấy địa chỉ cuối cùng trong header không thuộc danh sách. CDN
công bố dải địa chỉ của họ; nhớ cập nhật danh sách khi dải đổi.

Log lúc khởi động cũng cảnh báo khi chưa đặt `AUTOXIDE_SECRET_KEY` (xem bảng dưới): hãy đặt nó
trước khi chạy thật.

Các biến cấu hình khác thêm cùng cách: mọi biến được liệt kê kèm giá trị mặc định trong
[`.env.example`](../.env.example). Biến nào `compose.yml` chưa chuyển vào thì thêm dưới
`environment:` của khối `x-autoxide`.

### 5. Cập nhật

```bash
git pull
docker compose up -d --build
```

Service `migrate` áp dụng migration mới trước khi máy chủ mới khởi động.

### 6. Backup và khôi phục

```bash
mkdir -p backups && sudo chown 10001 backups
docker compose run --rm -v "$PWD/backups:/backups" app backup "/backups/$(date +%F)"
```

Bản backup là một thư mục chứa bản sao của database (dạng file SQLite) và của mọi file
đã lưu; tạo được trong lúc máy chủ đang chạy. Hãy chép nó đi nơi khác — một máy khác hay
object storage. Để khôi phục vào một bản cài mới, chỉ khởi động PostgreSQL, khôi phục
(`--no-deps` để Compose không chạy migration trước: restore cần một database trống), rồi
khởi động tất cả:

```bash
docker compose up -d postgres
docker compose run --rm --no-deps -v "$PWD/backups:/backups" app restore /backups/2026-11-11
docker compose up -d
```

Chỉ khôi phục bản backup do chính bạn tạo.

## B. Một binary với SQLite

### 1. Build

Cài Rust từ [rustup.rs](https://rustup.rs) (đúng phiên bản được chọn theo
`rust-toolchain.toml`) và một trình biên dịch C — SQLite và libwebp được biên dịch vào binary
(trên Debian hay Ubuntu: `sudo apt install build-essential`) — rồi:

```bash
cargo build --release --locked -p autoxide
sudo install -m 755 target/release/autoxide /usr/local/bin/autoxide
```

### 2. Chuẩn bị thư mục

```bash
sudo useradd --system --home-dir /var/lib/autoxide --create-home autoxide
sudo -u autoxide sh -c 'cd /var/lib/autoxide && printf "AUTOXIDE_AUTH_TOKEN_SECRET=%s\nAUTOXIDE_SECRET_KEY=%s\n" "$(openssl rand -base64 48 | tr -d "\n")" "$(openssl rand -base64 48 | tr -d "\n")" > .env && chmod 600 .env'
```

Autoxide đọc `.env` trong thư mục nó chạy và giữ mọi thứ nó ghi trong `data/` ở đó:
database `data/autoxide.sqlite`, file tải lên `data/files`, và mail `data/mail` cho tới
khi cấu hình SMTP.

### 3. Migration và quản trị viên đầu tiên

```bash
cd /var/lib/autoxide
sudo -u autoxide autoxide migrate up
sudo -u autoxide autoxide user create --email you@example.com --name "Tên của bạn" --admin
```

Mất thiết bị của tài khoản quản trị duy nhất đang bật xác thực hai bước? Tắt nó ngay trên máy
chủ, rồi đăng nhập bằng mật khẩu và bật lại:

```bash
sudo -u autoxide autoxide user reset-mfa --email you@example.com
```

Mất các thiết bị giữ passkey của tài khoản đó? Gỡ chúng ngay trên máy chủ, rồi đăng nhập bằng
mật khẩu và thêm passkey mới. Quản trị viên khác làm được điều tương tự qua API với permission
`auth.passkeys.manage` (`DELETE /api/v1/auth/users/{id}/passkeys`).

```bash
sudo -u autoxide autoxide user remove-passkeys --email you@example.com
```

Ba lệnh này làm việc với khách mặc định. Khi có nhiều khách (`AUTOXIDE_TENANCY_MODE=multi`), thêm
`--tenant <slug>` để làm với tài khoản của khách khác, ví dụ quản trị viên đầu tiên của khách đó:

```bash
sudo -u autoxide autoxide user create --tenant acme --email owner@acme.example --name "Owner" --admin
```

### 4. Chạy như một service

`/etc/systemd/system/autoxide.service`:

```ini
[Unit]
Description=Autoxide
After=network-online.target
Wants=network-online.target

[Service]
User=autoxide
WorkingDirectory=/var/lib/autoxide
ExecStart=/usr/local/bin/autoxide serve
Restart=on-failure
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/lib/autoxide

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now autoxide
curl http://127.0.0.1:8080/health/ready
```

Đặt reverse proxy phía trước như ở [A.4](#4-đưa-lên-internet) và đặt
`AUTOXIDE_HTTP_CLIENT_IP_HEADER` trong `.env`.

### 5. Cập nhật

Build phiên bản mới, dừng service, chạy migration, khởi động:

```bash
sudo systemctl stop autoxide
sudo install -m 755 target/release/autoxide /usr/local/bin/autoxide
(cd /var/lib/autoxide && sudo -u autoxide autoxide migrate up)
sudo systemctl start autoxide
```

### 6. Backup và khôi phục

```bash
cd /var/lib/autoxide
sudo -u autoxide autoxide backup "backups/$(date +%F)"
```

Để khôi phục trên một máy mới, chuẩn bị thư mục (bước 1–2), rồi chạy
`autoxide restore <thư mục backup>` ở đó trước khi khởi động service.

Với `AUTOXIDE_TENANCY_MODE=multi`, bản backup có database chính (sổ khách và mọi khách trên đó) và
mọi file ở nơi lưu file chung, nhưng không có database của các placement khác, cũng không có nơi lưu
file riêng của placement; lệnh in cảnh báo nêu tên chúng. Backup những thứ đó bằng công cụ của chính
database (và của nơi lưu). Cất `AUTOXIDE_SECRET_KEY` ở nơi an toàn, tách khỏi bản
backup: thiếu nó thì secret đã mã hoá trong bản backup không đọc được.

## Chuyển từ SQLite sang PostgreSQL

Tạo một database PostgreSQL trống, dừng máy chủ, rồi chép. Database đích được áp dụng
mọi migration trước, rồi nhận toàn bộ dữ liệu trong một transaction:

```bash
sudo systemctl stop autoxide
cd /var/lib/autoxide
printf '%s' 'postgres://autoxide:<mật khẩu>@db.example.com:5432/autoxide' \
  | sudo -u autoxide autoxide copy --to-stdin
```

Rồi đặt `DATABASE_URL` trong `.env` bằng URL đó và khởi động máy chủ. File vẫn nằm chỗ
cũ; muốn chuyển cả file lên S3 thì tạo backup, đặt cấu hình S3, rồi khôi phục vào một
database mới.

## Những biến cấu hình nên biết

| Biến | Dùng để |
| --- | --- |
| `DATABASE_URL` | `sqlite://data/autoxide.sqlite` (mặc định) hoặc `postgres://user:password@host:5432/db` |
| `AUTOXIDE_AUTH_TOKEN_SECRET` | Ký access token; bắt buộc ở production, giống nhau trên mọi máy chủ |
| `AUTOXIDE_SECRET_KEY` | Ký các token sống ngắn của mọi module và làm khoá băm địa chỉ IP của người lạ; không bắt buộc: không đặt thì máy chủ sinh một lần và lưu trong database. Khi chạy thật nên đặt (ngoài database, giống nhau trên mọi máy chủ): khoá nằm trong database thì ai có bản backup database cũng mở được các secret đã mã hoá và tính ngược được khối IP từ các bản băm đã lưu |
| `AUTOXIDE_SECRET_KEY_PREVIOUS` | Các giá trị cũ của `AUTOXIDE_SECRET_KEY`, cách nhau dấu phẩy, trong lúc xoay khoá; bỏ đi sau khi chạy `autoxide secrets reseal` |
| `AUTOXIDE_HTTP_CLIENT_IP_HEADER` | Sau reverse proxy: header chứa địa chỉ client |
| `AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT` | Số proxy của bạn mà một request đi qua (mặc định `1`) |
| `AUTOXIDE_HTTP_TRUSTED_PROXIES` | Thay cho số proxy: địa chỉ và dải địa chỉ của proxy, vd `10.0.0.0/8, 192.0.2.7` |
| `AUTOXIDE_HTTP_PUBLIC_URL` | Địa chỉ internet dùng để gọi API, vd `https://api.example.com`; cần cho link trỏ về API trong mail (huỷ đăng ký bản tin) |
| `AUTOXIDE_HTTP_CORS_ORIGINS` | Trang web ở origin khác được gọi API |
| `AUTOXIDE_MAIL_SMTP_URL`, `AUTOXIDE_MAIL_FROM` | Gửi mail (đặt lại mật khẩu, xác minh email) |
| `AUTOXIDE_FILES_S3_BUCKET` và `AWS_…` | Lưu file trên S3 hay dịch vụ tương thích S3; nên đặt thêm lifecycle rule cho bucket để huỷ multipart upload dở sau một ngày |
| `AUTOXIDE_AUTH_PASSWORD_RESET_URL`, `AUTOXIDE_AUTH_EMAIL_VERIFICATION_URL` | Mail dẫn tới trang của bạn |
| `AUTOXIDE_AUTH_PASSKEY_RP_ID`, `AUTOXIDE_AUTH_PASSKEY_ORIGINS` | Bật đăng nhập bằng passkey cho tên miền của bạn |
| `AUTOXIDE_HTTP_DRAIN_SECONDS` | Sau bộ cân bằng tải: lúc tắt, phục vụ thêm bao lâu trong khi `/health/ready` trả `503`, xem [bên dưới](#cập-nhật-không-làm-rơi-request) |
| `AUTOXIDE_PROCESS_ROLE` | `api` / `worker` để chạy API và việc nền ở tiến trình riêng |
| `AUTOXIDE_HTTP_METRICS` | Metrics Prometheus ở `/metrics`, cùng cổng với API: khi bật, hãy chặn `/metrics` ở reverse proxy (Caddy: `respond /metrics 404`) và lấy số đo ngay trên máy chủ |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Gửi trace tới một OpenTelemetry collector |

Kiểm cấu hình mà không khởi động gì: `autoxide config`. Lệnh liệt kê mọi biến kèm giá
trị (secret được giấu) và báo mọi vấn đề cùng một lượt.

## Cấu hình và trạng thái của từng khách

Khi chạy nhiều khách hàng (`AUTOXIDE_TENANCY_MODE=multi`), các biến trên áp cho mọi khách. Một số
hành vi của module còn đặt được theo từng khách, thay giá trị của máy chủ cho riêng khách đó:

| Cấu hình của khách | Thay cho |
| --- | --- |
| `kernel.public_url` | `AUTOXIDE_HTTP_PUBLIC_URL` trong link gửi qua mail |
| `mail.connection.smtp.url`, `mail.connection.smtp.from` | `AUTOXIDE_MAIL_SMTP_URL`, `AUTOXIDE_MAIL_FROM` |
| `ai.connection.text.api_key`, `ai.connection.text.model` | khoá và mô hình của nhà cung cấp AI của máy chủ (`AUTOXIDE_AI_API_KEY`, `AUTOXIDE_AI_MODEL`) |
| `comments.banned_words`, `comments.suspect_words`, `comments.max_links`, `comments.guest_moderation`, `comments.user_moderation` | các biến `AUTOXIDE_COMMENTS_…` tương ứng ([bình luận](COMMENTS.vi.md#theo-từng-khách)) |
| `forms.retention_days` | `AUTOXIDE_FORMS_RETENTION_DAYS` ([form](FORMS.vi.md)) |
| `content.locales`, `content.default_locale` | `AUTOXIDE_CONTENT_LOCALES`, `AUTOXIDE_CONTENT_DEFAULT_LOCALE` ([nội dung](CONTENT.vi.md#ngôn-ngữ)) |
| `media.alt_locales` | `AUTOXIDE_MEDIA_ALT_LOCALES` ([media](MEDIA.vi.md)) |

`GET /api/v1/kernel/settings` liệt kê chúng kèm giá trị khách hiện tại đã đặt. Quản trị của khách
(permission `kernel.settings.manage`) đặt một cấu hình bằng `PUT /api/v1/kernel/settings/{name}` với
`{"value": "…"}`, và bỏ (về giá trị của máy chủ) bằng `DELETE`. Nhân sự nền tảng làm như vậy cho khách
bất kỳ dưới `/api/v1/kernel/tenants/{id}/settings` (`kernel.tenants.read` để xem,
`kernel.tenants.manage` để đổi). Giá trị được kiểm ngay lúc đặt; giá trị secret lưu mã hoá và không
bao giờ hiện lại. Máy chủ khác dùng giá trị mới trong vòng `AUTOXIDE_TENANCY_CACHE_SECONDS`.

Cấu hình nối khách với một dịch vụ của đối tác (máy chủ mail của khách, tài khoản nhà cung cấp AI
của khách…) thuộc nhóm `connection` và mang tên dạng `<module>.connection.<loại>.<trường>`:
`GET /api/v1/kernel/settings?group=connection` chỉ liệt kê các cấu hình đó — đúng thứ màn "Kết nối
đối tác" cần. Mỗi cấu hình trong danh sách mang `group` của nó (`null` khi không thuộc nhóm nào).

Cấu hình kết nối của mail và AI đã đổi tên theo dạng này. Tên cũ (`mail.smtp_url`, `mail.from`,
`ai.api_key`, `ai.model`) vẫn dùng được: giá trị khách đã đặt dưới tên cũ tiếp tục có hiệu lực, và
`PUT` / `DELETE` vẫn nhận tên cũ. Danh sách chỉ hiện tên hiện tại.

Có hai cách dừng một khách, độc lập với nhau:

- **Tạm khoá** — `autoxide tenant suspend <khách>` (mở lại bằng `resume`), hoặc `PATCH
  /api/v1/kernel/tenants/{id}` với `{"status": "suspended"}`: mọi request tới khách, kể cả đọc, nhận
  `403 tenant_suspended`, và tên miền của khách không được cấp chứng chỉ mới.
- **Chỉ đọc** — `autoxide tenant read-only <khách>` (gỡ bằng `writable`), hoặc `{"read_only":
  true}`: đọc được, người dùng của khách vẫn đăng nhập, đặt lại mật khẩu, xuất dữ liệu và dùng link
  trong mail (xác nhận địa chỉ, huỷ đăng ký); mọi thay đổi khác nhận `403 tenant_read_only`, tool MCP
  không chỉ đọc cũng vậy. Dùng khi, ví dụ, khách chưa thanh toán.

Job nền và event của khách ở cả hai trạng thái vẫn chạy: chúng làm nốt việc đã nhận trước đó, và
việc xoá theo thời hạn giữ vẫn diễn ra. Khách nền tảng không tạm khoá được, cũng không đặt chỉ đọc
được.

`GET /api/v1/kernel/tenants` (nhân sự nền tảng) có bộ lọc cho mặt điều khiển — `status`,
`read_only`, `placement_kind`, `schema_state`, và `subscription_status` / `near_limit` (chưa khớp
khách nào tới khi cài module gói dịch vụ: thuê bao và hạn mức) — và `expand=placement` để lấy luôn
placement của từng khách trong cùng câu trả lời.

## Tên miền riêng của khách hàng

Khi chạy nhiều khách hàng (`AUTOXIDE_TENANCY_MODE=multi`), mỗi khách có thể phục vụ trang của mình ở
tên miền riêng, vd `shop.example`: request tới một tên miền đã xác minh vào thẳng khách đó, không
cần header `Autoxide-Tenant`. Bản cài chỉ một khách thì không cần việc này: tên miền của nó là tên
miền của nền tảng.

1. Quản trị viên của khách (permission `kernel.domains.manage`) thêm tên miền:

   ```bash
   curl -X POST https://api.example.com/api/v1/kernel/domains \
     -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
     -d '{"domain": "shop.example"}'
   ```

   Câu trả lời có `"status": "pending"` và `record` cần tạo ở nhà cung cấp DNS của tên miền: một
   bản ghi TXT tên `_autoxide-challenge.shop.example` với giá trị `autoxide-verify=…`.
2. Ở nhà cung cấp DNS, tạo bản ghi TXT đó và trỏ chính tên miền về máy chủ của bạn (bản ghi
   `A` / `AAAA`, hoặc `CNAME` tới tên miền của nền tảng).
3. Xác minh: `POST /api/v1/kernel/domains/{id}/verify` tra bản ghi ngay và trả `verified`, hoặc vẫn
   `pending` kèm lý do trong `check_error`. Tên miền đang chờ cũng được tra lại mỗi 10 phút; quá 7
   ngày chưa xác minh được thì thành `failed` (sửa bản ghi rồi xác minh lại).
   `DELETE /api/v1/kernel/domains/{id}` gỡ một tên miền.
4. Giữ nguyên bản ghi TXT. Tên miền đã xác minh được kiểm lại mỗi ngày một lần: 3 lần liên tiếp không
   thấy bản ghi thì tên miền thành `failed` (event `kernel.domain_failed`) và request tới nó không
   còn vào khách. `failed_checks` đếm số lần trượt; một lần kiểm thấy bản ghi (hay `POST …/verify`
   sau khi sửa) đưa nó về 0. Máy chủ DNS không trả lời thì không tính là trượt.

Tên miền của chính nền tảng (tên máy trong `AUTOXIDE_HTTP_PUBLIC_URL`), địa chỉ IP và `localhost`
bị từ chối. Mỗi lúc một tên miền chỉ thuộc một khách: khách khác chỉ nhận được sau khi nó bị gỡ hay
xác minh hỏng.

Một tên miền có thể thuộc về một nút trong cây tổ chức của khách — vd một cửa hàng có trang riêng:
`PATCH /api/v1/kernel/domains/{id}` với `{"org_id": "<id của nút>"}` gắn tên miền với nút đó,
`{"org_id": null}` gỡ ra (tên miền khi đó là của cả khách). Nút phải là nút của chính khách; khi
chưa module nào đã cài giữ cây tổ chức thì chỉ nhận `null`. Gắn nút không đổi trạng thái hay bản ghi
DNS của tên miền; tên miền đã gỡ trả `409 tenant_domain_revoked`. Mỗi lần đổi ghi event
`kernel.domain_updated`.

**Chứng chỉ TLS.** Để Caddy lấy chứng chỉ khi người xem đầu tiên của một tên miền tới (on-demand
TLS), sau khi hỏi Autoxide tên miền đó có được phép không:
`GET /api/v1/kernel/domains/allowed?domain=…` chỉ trả `200` cho tên miền đã xác minh của khách đang
hoạt động, nên không ai khiến máy chủ của bạn xin chứng chỉ cho tên bạn không phục vụ.

```text
{
    on_demand_tls {
        ask http://127.0.0.1:8080/api/v1/kernel/domains/allowed
    }
}

api.example.com {
    reverse_proxy 127.0.0.1:8080
}

https:// {
    tls {
        on_demand
    }
    reverse_proxy 127.0.0.1:8080
}
```

`reverse_proxy` của Caddy chuyển nguyên header `Host`, và Autoxide nhận ra khách nhờ header đó. Khi
có proxy khác đứng trước và viết lại `Host`, proxy đó phải gửi tên miền người xem gõ trong
`X-Forwarded-Host` (hoặc `Forwarded: host=…`), và địa chỉ của nó phải có trong
`AUTOXIDE_HTTP_TRUSTED_PROXIES`: Autoxide chỉ đọc các header đó từ proxy tin cậy. Endpoint `ask`
cũng bị tính vào rate limiting theo địa chỉ như mọi endpoint khác; Caddy chỉ hỏi một lần cho mỗi
chứng chỉ mới. Xác minh, gỡ tên miền hay tạm khoá khách có hiệu lực trên mọi máy chủ sau tối đa
`AUTOXIDE_TENANCY_CACHE_SECONDS` giây (mặc định `10`).

## Tài khoản của một khách: nhân sự, người mua, uỷ quyền

Mỗi khách có hai loại tài khoản. **Nhân sự** (`realm: staff`, mặc định) làm việc bằng role và
permission. **Người mua** (`realm: customer`) là tài khoản của cửa hàng: đăng nhập được, không có
permission nào, tách khỏi nhân sự dù trùng email. Người mua đăng ký bằng
`POST /api/v1/auth/register` và đăng nhập bằng `POST /api/v1/auth/login`, cùng với
`"realm": "customer"`; khách đóng đăng ký người mua bằng cài đặt theo cây tổ chức
`auth.customer_registration` (`false`). Nhân sự quản lý người mua bằng `GET /api/v1/auth/customers`
và `PATCH /api/v1/auth/customers/{id}` (permission `auth.customers.read`, `auth.customers.manage`).

**Mời nhân sự.** Thay vì tạo tài khoản kèm mật khẩu, hãy mời: `POST /api/v1/auth/invitations` với
`email`, các `role_ids` và nút (`org_id`) họ nhận (permission `auth.invitations.manage`; không mời kèm
role có permission mà bạn không có), hoặc từ dòng lệnh:

```bash
autoxide user invite --email new.colleague@example.com --role editor
```

Mail mang link dùng một lần tới `AUTOXIDE_AUTH_INVITATION_URL` (trang của giao diện có `{token}`;
không đặt thì mail chứa token), sống `AUTOXIDE_AUTH_INVITATION_DAYS` ngày (7). Chấp nhận thì tài
khoản được tạo (hay, khi email đã có tài khoản, role được thêm sau khi người đó đăng nhập). **Đăng
nhập bằng link** (không mật khẩu) tắt mặc định; bật bằng `AUTOXIDE_AUTH_LOGIN_LINK=true` và đặt
`AUTOXIDE_AUTH_LOGIN_LINK_URL` là trang của giao diện. Cả hai cần mail chạy được
(`AUTOXIDE_MAIL_SMTP_URL`) và worker đang chạy. Cây tổ chức của một khách sâu tối đa
`AUTOXIDE_ORGANIZATIONS_MAX_DEPTH` cấp (mặc định 8). Nhập danh mục đơn vị hành chính một lần để kiểm
địa chỉ: `autoxide task address.import` ([`docs/ADDRESS.vi.md`](ADDRESS.vi.md)).

**Nhân sự nền tảng và dữ liệu của khách.** Nhân sự nền tảng (tài khoản của khách nền tảng) không bao
giờ vào dữ liệu của khách chỉ nhờ role: người có `auth.support.grant` cấp cho một nhân sự quyền vào có
thời hạn bằng `POST /api/v1/auth/support-grants` (`tenant_id`, `staff_user_id`, một `role_id` của nền
tảng, `basis` là `support` hay `legal_request` kèm số văn bản, `reason`, tối đa
`AUTOXIDE_AUTH_SUPPORT_ACCESS_MAX_HOURS` giờ, mặc định 8). Khách thấy mọi lần cấp, lần vào đầu và mọi
thay đổi đã làm (`GET /api/v1/auth/support-access`), tự thu hồi lần cấp `support` còn hạn
(`DELETE /api/v1/auth/support-access/{id}`), và tắt được quyền vào hỗ trợ bằng cài đặt
`auth.support_access` (khi đó chỉ lần cấp `legal_request` còn hiệu lực).

Trong chuỗi khách (khách có `parent_id`), khách cha cho người giữ một role của mình làm việc ở khách
con: `POST /api/v1/auth/delegations` với `child_tenant_id`, `role_id`, `org_id` tuỳ chọn (một nút của
khách con), `expires_at` tuỳ chọn và `reason` (permission `auth.delegations.manage`). Những người đó
giữ cách đăng nhập của mình và thêm header `Autoxide-Tenant: <id khách con>`;
`GET /api/v1/auth/me/tenants` liệt kê những nơi họ vào được. Mọi thay đổi họ làm ở khách con ghi kèm
đường vào là uỷ quyền (`actor_access` trong event và audit log của khách con); thu hồi
(`DELETE /api/v1/auth/delegations/{id}`) có hiệu lực từ request kế. Khách con xem những gì nhận được
bằng `GET /api/v1/auth/delegations?direction=incoming`.

## Dời khách hàng sang database khác

Khi chạy nhiều khách hàng, dữ liệu của mỗi khách nằm ở một *placement* (vị trí dữ liệu): `main`
(database của `DATABASE_URL`, nhiều khách dùng chung) hay một database khác thêm bằng
`autoxide placement add`. Dời khách để cho một khách đang lớn database riêng, hay đưa khách về lại:

1. Thêm database đích và áp dụng migration cho nó:

   ```bash
   printf '%s' 'postgres://autoxide:<password>@db2.example.com:5432/acme' \
     | autoxide placement add acme-db --kind dedicated --url -
   autoxide migrate up --all
   ```

2. Thử trước. Chạy thử (dry run) không đổi gì:

   ```bash
   autoxide tenant move acme --to acme-db --dry-run
   ```

   Lệnh liệt kê số dòng của khách sẽ được chép theo từng bảng, và mọi thứ đang chặn việc dời: một
   bên còn migration chưa áp dụng, đích là cùng database với nguồn, khách không ở trạng thái hoạt
   động, khách đang có một lượt dời khác. Khách nền tảng (`default`) luôn ở `main`.

3. Dời, trong một khoảng bảo trì:

   ```bash
   autoxide tenant move acme --to acme-db
   ```

   Ngay từ đầu, khách trả `503 tenant_maintenance` kèm `Retry-After`; khách khác không bị ảnh
   hưởng. Lệnh chờ tới khi mọi máy chủ đã thấy việc dời (`AUTOXIDE_TENANCY_CACHE_SECONDS`) và
   request đang chạy đã xong (`AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS`), và tới khi job và event của
   khách đang được chạy đã xong. Rồi lệnh chép mọi dòng của khách, kể cả job đang chờ và event chưa
   giao, trong một transaction; so số dòng của từng bảng; chuyển khách sang placement mới (khách
   chạy lại từ đó); và xoá bản cũ. SQLite và PostgreSQL trộn được theo cả hai chiều. File (dưới
   `tenants/<id>/`) ở nguyên chỗ cũ khi hai placement dùng cùng nơi lưu file; khi đích có nơi lưu
   riêng (xem dưới), file được chép sang đó trước khi chép dòng và bị xoá ở nơi lưu cũ ở cuối. Chạy
   thử cho biết số file sẽ chép.

`--keep-source` giữ bản cũ làm bản dự phòng thay vì xoá (job đang chờ và event của bản cũ bị xoá,
để không gì chạy hai lần; file của bản cũ cũng được giữ). Không gì dùng bản đó; xoá nó khi đã yên
tâm:

```bash
autoxide tenant purge-copy acme --placement main
```

Qua API, nhân sự nền tảng có `kernel.tenants.move` gọi `POST /api/v1/kernel/tenants/{id}/move`
với `{"to": "acme-db"}`: đó là chạy thử. Thêm `"dry_run": false` để bắt đầu dời bằng một job nền, và
theo dõi ở `Location` của câu trả lời (`GET /api/v1/kernel/tenants/{id}/moves/{move_id}`); tool MCP
là `kernel.move_tenant`. Một job chạy được khoảng bốn phút: khách lớn (chạy thử cho biết số dòng)
thì dời bằng lệnh, lệnh không giới hạn thời gian.

**Nơi lưu file riêng của placement.** Mặc định file của mọi khách nằm ở nơi lưu theo cấu hình của
module `files` (`AUTOXIDE_FILES_DIR` hay `AUTOXIDE_FILES_S3_BUCKET`), dưới `tenants/<id>/`. Cho một
placement — thường là placement `dedicated` — bucket hay thư mục riêng, để file của khách đó cũng
nằm riêng:

```bash
printf '%s' "$SECRET_ACCESS_KEY" | autoxide placement storage acme-db \
  --s3-bucket acme-files --s3-region eu-west-1 \
  --s3-endpoint https://<account>.r2.cloudflarestorage.com \
  --s3-access-key-id "$ACCESS_KEY_ID" --s3-secret-access-key -
# hoặc một thư mục trên máy chủ:
autoxide placement storage acme-db --dir /var/lib/autoxide/files-acme
```

Cấu hình được lưu mã hoá bằng `AUTOXIDE_SECRET_KEY`; `autoxide placement list` và
`GET /api/v1/kernel/placements` cho thấy bucket hay thư mục, không bao giờ cho thấy khoá. Bucket
phải có sẵn; khoá sai lộ ra ở lần tải lên đầu tiên. Đặt nơi lưu khi placement chưa có khách, rồi dời
khách sang: việc dời chép file của họ. Đổi về sau bị từ chối (file của họ đang ở nơi lưu cũ), trừ khi
thêm `--force` khi nơi lưu mới đã có đúng các file đó — ví dụ khoá truy cập mới cho cùng bucket.
`--clear` đưa file tải lên mới về nơi lưu chung (cùng luật). Mọi máy chủ phải chạy module `files`.

**Khi việc dời dừng giữa chừng** (tiến trình chết, không kết nối được database, số dòng không
khớp), không mất gì. Trước bước chuyển, khách vẫn trả `503` ở placement cũ; sau bước chuyển, khách
chạy từ placement mới và chỉ còn bản cũ chờ xoá.

- Chạy lại đúng lệnh `autoxide tenant move acme --to acme-db`: lệnh chạy tiếp từ chỗ dừng (trước
  bước chuyển thì xoá các dòng đã chép sang đích rồi chép lại).
- Hoặc, trước bước chuyển, bỏ cuộc: `autoxide tenant move acme --abort` xoá những gì đã chép và cho
  khách hoạt động lại ở placement cũ.
- `GET /api/v1/kernel/tenants/{id}/moves/{move_id}` cho biết `state` của lượt dời và `error` đã làm
  nó dừng. Lượt dời bắt đầu qua API được job của nó thử lại; bắt đầu một lượt khác khi một lượt
  đang chạy thì nhận `tenant_move_running`.

## Chạy nhiều máy chủ

Với PostgreSQL, chạy bao nhiêu tiến trình `app` cũng được sau reverse proxy: chúng chỉ
dùng chung database (và S3 cho file), và job nền được nhận sao cho mỗi job chạy đúng một
lần. Đặt cùng `AUTOXIDE_AUTH_TOKEN_SECRET` ở mọi nơi. Muốn tăng API và việc nền riêng
rẽ, chạy vài tiến trình với `AUTOXIDE_PROCESS_ROLE=api` và ít nhất một tiến trình
`worker`. SQLite chỉ cho một tiến trình máy chủ.

### Cập nhật không làm rơi request

Khi được bảo dừng (`SIGTERM`, tín hiệu Docker, systemd và Kubernetes gửi), máy chủ đi qua
hai giai đoạn:

1. **Rút** trong `AUTOXIDE_HTTP_DRAIN_SECONDS` giây (mặc định `0`): vẫn xử lý mọi request,
   nhưng `/health/ready` trả `503` với mã `shutting_down`, để bộ cân bằng tải thôi gửi
   request mới tới. `/health/live` vẫn `200`: không được khởi động lại tiến trình.
2. **Dừng**: ngừng nhận kết nối và cho request đang chạy tối đa
   `AUTOXIDE_HTTP_SHUTDOWN_SECONDS` giây (mặc định `30`) để xong.

Để cập nhật lần lượt từng máy chủ sau bộ cân bằng tải:

- Trỏ health check của bộ cân bằng tải (hay `readinessProbe` của Kubernetes) tới
  `/health/ready`, còn `livenessProbe` tới `/health/live`.
- Đặt `AUTOXIDE_HTTP_DRAIN_SECONDS` dài hơn thời gian bộ cân bằng tải cần để gỡ một máy
  chủ: khoảng giữa hai lần kiểm nhân với số lần hỏng nó chờ. Kiểm 5 giây một lần và bỏ sau
  2 lần hỏng thì cần hơn 10, vậy đặt `15`.
- Cho tiến trình đủ thời gian cho cả hai giai đoạn trước khi bị giết: `stop_grace_period`
  của Docker Compose, `terminationGracePeriodSeconds` của Kubernetes, `TimeoutStopSec` của
  systemd phải ít nhất bằng rút + dừng (`45s` cho `15` + `30`). Mặc định Docker chỉ chờ
  10 giây; `compose.yml` trong repo này đặt `40s`, đủ khi không rút — đặt khoảng rút thì
  tăng nó lên.
- Chạy máy chủ mới và chờ `/health/ready` của chúng trả `200` rồi mới dừng máy chủ cũ.

Chỉ một máy chủ, không có bộ cân bằng tải thì để khoảng rút là `0`: không có nơi nào khác
để gửi request, rút chỉ làm việc dừng chậm thêm.

## Kiểm trước khi chạy thật

- [ ] `AUTOXIDE_AUTH_TOKEN_SECRET` và `AUTOXIDE_SECRET_KEY` là chuỗi ngẫu nhiên dài, giữ bí mật
      (`autoxide doctor` cảnh báo khi thiếu)
- [ ] Có TLS phía trước (reverse proxy), port 8080 không mở ra internet
- [ ] `AUTOXIDE_HTTP_CLIENT_IP_HEADER` đặt theo header của proxy
- [ ] Đã cấu hình SMTP, để đặt lại mật khẩu và xác minh email chạy được
- [ ] Backup theo lịch và chép sang máy khác; đã thử khôi phục một lần
- [ ] `/health/ready` được hệ thống giám sát theo dõi
