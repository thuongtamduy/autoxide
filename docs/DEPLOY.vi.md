# Cài đặt và deploy Autoxide

[English](DEPLOY.md)

Hướng dẫn này đưa bạn từ con số không tới một máy chủ đang chạy, cập nhật được và có
backup. Chọn một trong hai cách chạy (cách thứ ba, một container với SQLite và không cần build,
ở [mục C](#c-một-container-với-sqlite)):

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
git clone https://github.com/thuongtamduy/autoxide.git autoxide
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
- `AUTOXIDE_AUTH_TOKEN_SECRET` ký access token. Muốn đổi mà không đăng xuất ai (định kỳ, vài
  tháng một lần): đặt giá trị mới vào nó và giá trị cũ vào `AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS` trên
  mọi máy chủ, khởi động lại, rồi bỏ `AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS` sau
  `AUTOXIDE_AUTH_ACCESS_TOKEN_SECONDS` (mặc định 15 phút): token ký bằng khoá cũ vẫn được nhận tới khi
  hết hạn. Đổi mà không giữ khoá cũ thì mọi access token hết hiệu lực.
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

Lần build đầu biên dịch mọi thứ từ mã nguồn: tính khoảng 20 phút tới hơn một giờ, và ít nhất 4 GB bộ nhớ
cho Docker. Sau proxy của công ty thì cấu hình proxy cho Docker trước (xem
[Yêu cầu trong README](../README.vi.md#yêu-cầu)).

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

Autoxide tự nén response JSON và chữ (gzip hay brotli, tuỳ client nhận cách nào; ảnh và file
khác gửi nguyên), nên proxy không cần chỉ thị `encode`. Nếu proxy hay CDN đã nén, đặt
`AUTOXIDE_HTTP_COMPRESSION=false` để khỏi nén hai lần.

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

**Cập nhật từ 0.17 trở về trước:** module thử nghiệm (bookings, newsletter, comments, support,
conversations, analytics, reporting, automations) không còn chạy nếu `AUTOXIDE_MODULES` không nêu
tên. Không gì bị xoá, và log khởi động lẫn `autoxide doctor` báo khi database còn dữ liệu của một
module đang tắt, kèm dòng cần thêm — xem
[MODULES.vi.md](MODULES.vi.md#nâng-cấp-từ-017-trở-về-trước). Điều này áp dụng cả cho cách B và C.

**Cập nhật lên bản có chỉ mục từ của content** (migration `content.m0036_word_index`): migration điền từ
của mọi bài đã đăng vào bảng mới `content_published_words` theo từng lô, rồi dựng index của bảng. Website lớn
thì lâu hơn và giữ lock của migration trong lúc đó: hãy cập nhật vào giờ vắng. Điều này áp dụng cả cho cách B
và C.

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
docker compose up -d --wait postgres
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
# Cao hơn AUTOXIDE_HTTP_MAX_CONNECTIONS (4096) cộng chỗ cho file và kết nối database.
LimitNOFILE=8192

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

## C. Một container với SQLite

Để dùng thử Autoxide hoặc một bản cài nhỏ trên một máy, không phải build gì. Các bước tới lần
chạy đầu nằm ở README, mục [Chạy thử bằng Docker](../README.vi.md#chạy-thử-bằng-docker-không-cần-build).
Những luật cần nhớ sau đó, với `IMAGE` và `RUN` như định nghĩa ở đó:

- Mặc định máy chủ không tự áp dụng migration. Chạy `serve` trên database trống hay cũ hơn sẽ dừng
  ngay với `N migration(s) have not been applied to the database (first: …); run autoxide migrate
  up first` (chỉ nêu số lượng và vài tên đầu; `autoxide migrate status` liệt kê đủ; mã lỗi
  `kernel.migrations_pending`): chạy
  `$RUN $IMAGE migrate up` rồi mới khởi động.
  Với một máy chủ, có thể đặt `AUTOXIDE_MIGRATE_ON_START=true` thay vào đó: `serve` tự áp dụng
  migration lúc khởi động (database chính trước khi mở port; rồi mọi database riêng của khách chạy
  nền, thành một lượt migrate mà `GET /api/v1/kernel/placements/migrations` cho xem; khách trên
  database chưa xong nhận 503 cho tới khi xong), dưới một khoá để hai máy chủ khởi động cùng lúc
  không cùng migrate. Kèm
  `AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL` thì tạo luôn quản trị viên đầu tiên, kèm `AUTOXIDE_STARTUP_TASKS`
  thì nạp dữ liệu nền — đúng những gì `compose.quickstart.yml` làm. Nhiều máy chủ thì vẫn chạy
  `migrate up` một lần mỗi lần deploy: thấy rõ cái gì đổi, và migration hỏng thì deploy dừng trước
  khi bản mới nhận request.
- Mọi thứ máy chủ ghi ra nằm trong volume `autoxide-data` (`/var/lib/autoxide/data`): database,
  file tải lên, và mail cho tới khi cấu hình SMTP. Container chạy không gắn volume đó chỉ giữ dữ
  liệu cho tới khi bị xoá.
- `docker run -p 8080:8080` mở cổng trên mọi giao diện mạng của máy; hãy dùng
  `-p 127.0.0.1:8080:8080` và một reverse proxy ([A.4](#4-đưa-lên-internet)). Thêm cấu hình vào
  `autoxide.env` (mỗi dòng một `TÊN=giá trị`, không có dấu nháy).
- Cập nhật: kéo image mới, áp dụng migration của nó, thay container. `--all` nâng thêm mọi
  database riêng của khách (placement) ở chế độ nhiều khách, ngoài chế độ đó không đổi gì; một
  placement hỏng không chặn các placement khác, lệnh nêu tên nó và thoát với mã 1.

```bash
IMAGE=ghcr.io/thuongtamduy/autoxide:<phiên bản mới>
docker pull $IMAGE
$RUN $IMAGE migrate up --all
docker rm -f autoxide
docker run -d --name autoxide --restart unless-stopped --env-file autoxide.env \
  -v autoxide-data:/var/lib/autoxide/data -p 127.0.0.1:8080:8080 $IMAGE serve
```

- Backup vào một thư mục bạn gắn vào (image chạy bằng user 10001, user đó phải ghi được vào
  đó), và restore vào volume mới trước khi khởi động máy chủ:

```bash
mkdir -p backups && sudo chown 10001 backups
$RUN -v "$PWD/backups:/backups" $IMAGE backup "/backups/$(date +%F)"
# trên máy mới, hoặc sau khi xoá volume:
$RUN -v "$PWD/backups:/backups" $IMAGE restore /backups/2026-11-11
```

Thiếu `chown`, backup dừng với `permission denied` và nói rõ thư mục nào cần giao cho uid 10001.
Xong, lệnh in đường dẫn tuyệt đối của bản backup. Backup ghi vào đường dẫn không được gắn vào sẽ
biến mất cùng container: trong container, `backup` cảnh báo ở stderr khi đích không nằm trên
volume hay bind mount (vẫn chạy tiếp).

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
| `AUTOXIDE_MODULES` | Module nào chạy: `official` (mặc định), `core`, `all`, tên để thêm, `-tên` để bỏ. Module thử nghiệm phải nêu tên. Xem [MODULES.vi.md](MODULES.vi.md) |
| `AUTOXIDE_AUTH_TOKEN_SECRET` | Ký access token; bắt buộc ở production, giống nhau trên mọi máy chủ |
| `AUTOXIDE_SECRET_KEY` | Ký các token sống ngắn của mọi module và làm khoá băm địa chỉ IP của người lạ; không bắt buộc: không đặt thì máy chủ sinh một lần và lưu trong database. Khi chạy thật nên đặt (ngoài database, giống nhau trên mọi máy chủ): khoá nằm trong database thì ai có bản backup database cũng mở được các secret đã mã hoá và tính ngược được khối IP từ các bản băm đã lưu |
| `AUTOXIDE_SECRET_KEY_PREVIOUS` | Các giá trị cũ của `AUTOXIDE_SECRET_KEY`, cách nhau dấu phẩy, trong lúc xoay khoá; bỏ đi sau khi chạy `autoxide secrets reseal` |
| `AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS` | Các giá trị cũ của `AUTOXIDE_AUTH_TOKEN_SECRET`, cách nhau dấu phẩy, trong lúc xoay khoá; chỉ dùng để kiểm token, bỏ đi sau thời gian sống của access token |
| `AUTOXIDE_HTTP_CLIENT_IP_HEADER` | Sau reverse proxy: header chứa địa chỉ client |
| `AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT` | Số proxy của bạn mà một request đi qua (mặc định `1`) |
| `AUTOXIDE_HTTP_TRUSTED_PROXIES` | Thay cho số proxy: địa chỉ và dải địa chỉ của proxy, vd `10.0.0.0/8, 192.0.2.7` |
| `AUTOXIDE_HTTP_MAX_CONNECTIONS` | Số kết nối mở cùng lúc tối đa, tính cả WebSocket (mặc định `4096`, `0`: không giới hạn); quá số này thì kết nối mới phải chờ. Đặt thấp hơn giới hạn file mở (`ulimit -n`, thường 1024; systemd `LimitNOFILE`, `docker run --ulimit nofile=…`) và chừa chỗ cho kết nối database và file — máy chủ cảnh báo lúc khởi động khi không như vậy. Reverse proxy vẫn nên giới hạn số kết nối theo từng client |
| `AUTOXIDE_HTTP_PUBLIC_URL` | Địa chỉ internet dùng để gọi API, vd `https://api.example.com`; cần cho link trỏ về API trong mail (huỷ đăng ký bản tin) |
| `AUTOXIDE_HTTP_CORS_ORIGINS` | Trang web ở origin khác được gọi API; tên miền đã xác minh của khách được gọi cho chính khách đó mà không cần liệt kê, xem [Tên miền riêng của khách hàng](#tên-miền-riêng-của-khách-hàng) |
| `AUTOXIDE_HTTP_HSTS_SECONDS` | Khi mọi client đã gọi API qua HTTPS: gửi `Strict-Transport-Security` (vd `31536000`, một năm); `autoxide doctor` nhắc khi URL công khai là `https://` mà biến này là `0`. Mọi response đã có sẵn `nosniff`, `Referrer-Policy`, `X-Frame-Options` và `Content-Security-Policy` chặt (`AUTOXIDE_HTTP_SECURITY_HEADERS`, `AUTOXIDE_HTTP_REFERRER_POLICY`) |
| `AUTOXIDE_MAIL_SMTP_URL`, `AUTOXIDE_MAIL_FROM` | Gửi mail (đặt lại mật khẩu, xác minh email) |
| `AUTOXIDE_FILES_S3_BUCKET` và `AWS_…` | Lưu file trên S3 hay dịch vụ tương thích S3; nên đặt thêm lifecycle rule cho bucket để huỷ multipart upload dở sau một ngày |
| `AUTOXIDE_AUTH_PASSWORD_RESET_URL`, `AUTOXIDE_AUTH_EMAIL_VERIFICATION_URL` | Mail dẫn tới trang của bạn |
| `AUTOXIDE_AUTH_PASSKEY_RP_ID`, `AUTOXIDE_AUTH_PASSKEY_ORIGINS` | Bật đăng nhập bằng passkey cho tên miền của bạn |
| `AUTOXIDE_HTTP_DRAIN_SECONDS` | Sau bộ cân bằng tải: lúc tắt, phục vụ thêm bao lâu trong khi `/health/ready` trả `503`, xem [bên dưới](#cập-nhật-không-làm-rơi-request) |
| `AUTOXIDE_PROCESS_ROLE` | `api` / `worker` để chạy API và việc nền ở tiến trình riêng |
| `AUTOXIDE_HTTP_COMPRESSION` | `true` (mặc định): nén JSON và chữ bằng gzip hay brotli; `false` khi reverse proxy hay CDN phía trước đã nén |
| `AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS` | CDN được giữ một lượt đọc công khai bao lâu (mặc định `60`), xem [CDN phía trước Autoxide](#cdn-phía-trước-autoxide) |
| `AUTOXIDE_HTTP_RATE_LIMIT_TENANT_PER_MINUTE` | Nhiều khách: số request một khách gửi được mỗi phút, mọi người dùng và API key của khách cộng lại (mặc định `0`, tắt), để một khách ồn ào không làm chậm khách khác; vượt thì 429 `tenant_rate_limited` kèm `Retry-After`. Hạn mức `kernel.api_rate_per_minute` của gói thay giá trị này cho khách đó (trong gói `0` là chặn mọi request; ở biến này `0` là tắt). Chỉ tính người gọi đã đăng nhập, vào khách họ đang làm việc (khách của chính họ, hay khách đã uỷ quyền cho họ): request không có thông tin xác thực hợp lệ (trang công khai, webhook gửi đến), nhân sự nền tảng thao tác trong khách và quyền hỗ trợ chỉ chịu giới hạn theo địa chỉ, nên người ngoài không làm cạn hạn mức của khách được. Một lô ghi tính một lần. Với `AUTOXIDE_HTTP_RATE_LIMIT_STORE=database`, burst nhỏ nghĩa là gần như mỗi request một lần ghi database: mức thấp trên một máy chủ thì nên dùng `memory` |
| `AUTOXIDE_HTTP_METRICS` | Metrics Prometheus ở `/metrics`, không có xác thực. Thêm `AUTOXIDE_HTTP_METRICS_ADDRESS` (vd `127.0.0.1:9090`) để phục vụ nó ở port riêng, tách khỏi API; không có biến đó thì `/metrics` nằm cùng port với API: hãy chặn nó ở reverse proxy (Caddy: `respond /metrics 404`) và lấy số đo ngay trên máy chủ |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Gửi trace tới một OpenTelemetry collector |

Kiểm cấu hình mà không khởi động gì: `autoxide config`. Lệnh liệt kê mọi biến kèm giá
trị (secret được giấu) và báo mọi vấn đề cùng một lượt.

## Thư bị dội

Khi máy chủ nhận từ chối hẳn chính địa chỉ người nhận lúc gửi — SMTP `5xx` với mã mở rộng `5.1.1`
(không có hộp thư), `5.1.2` (không có tên miền), `5.1.3` (địa chỉ sai), `5.1.6` (hộp thư đã chuyển) hay
`5.1.10` (tên miền không nhận thư) — địa chỉ có một bản ghi thư bị dội ở khách của nó. Lỗi của bên gửi
(`5.1.7`, `5.1.8`, địa chỉ người gửi sai hay tài khoản SMTP bị khoá), hộp thư bị khoá (`5.2.1`), hộp thư đầy
(`5.2.2`), thư quá cỡ (`5.2.3`), hay câu trả lời không có mã mở rộng không được tính, nên cấu hình gửi sai không bao giờ làm chặn người nhận. Mỗi 24
giờ tính tối đa một lần: sau `AUTOXIDE_MAIL_HARD_BOUNCE_LIMIT` (mặc định 3) lần được tính, địa chỉ bị
**chặn** ở riêng khách đó: thư mới tới nó — kể cả thư đặt lại mật khẩu — không được gửi, và mỗi thư bị bỏ được đếm trên
bản ghi kèm nguồn (vd `auth.password_reset`), nên câu hỏi "vì sao người này không nhận được thư" có câu
trả lời. API vẫn trả lời như đã gửi, để không ai biết địa chỉ nào bị chặn.

Quản trị của khách (hay nhân sự nền tảng vào khách):

- `GET /api/v1/mail/bounces?email=ada@example.com` — thư tới địa chỉ này có bị dội không, bao nhiêu
  lần, mã SMTP cuối, có đang bị chặn không (`mail.bounces.read`);
- `GET /api/v1/mail/bounces/stats` — số địa chỉ, đang chặn, bị dội và bị bỏ thư trong 30 ngày;
- `POST /api/v1/mail/bounces/{id}/release` — gửi lại khi địa chỉ đã dùng được (`mail.bounces.manage`);
  người dùng đổi sang địa chỉ khác thì nhận thư ngay;
- `DELETE /api/v1/mail/bounces/{id}` — xoá bản ghi (yêu cầu của chủ dữ liệu).

Bản ghi bị xoá sau `AUTOXIDE_MAIL_BOUNCE_RETENTION_DAYS` (mặc định 365) ngày kể từ lần dội hay lần bỏ thư
cuối. Số đo: `mail_bounces_total{kind}`, `mail_suppressed_total`, `mail_skipped_suppressed_total`. Thư dội
báo về sau dạng báo cáo giao thư (DSN) và webhook báo dội của nhà cung cấp mail chưa được đọc; newsletter
giữ danh sách chặn riêng của nó (`POST /api/v1/newsletter/bounces`).

## CDN phía trước Autoxide

Đọc công khai của website (`/api/v1/content/public/…`, `/api/v1/catalog/public/…`) trả
`Cache-Control: public, max-age=0, s-maxage=60, stale-while-revalidate=300`, một `ETag` và
`Vary: Accept-Language, Accept-Encoding, Origin, autoxide-tenant`. CDN (Cloudflare, CloudFront, Fastly…) giữ
response đó `AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS` giây (mặc định `60`; `0` thì lần nào cũng hỏi lại); trình duyệt
luôn hỏi lại và nhận `304 Not Modified` khi không có gì đổi. Bài vừa sửa tới người đọc trong chừng đó giây. File
media có `?v=` trong địa chỉ không bao giờ đổi nên được giữ một năm (`immutable`).

Autoxide không bao giờ để cache dùng chung giữ thứ gì riêng tư: request có `Authorization` hay cookie nhận
`private` (hay `no-store`), bản xem trước nhận `no-store`; lệnh ghi không bao giờ được cache. Cấu hình CDN để:

- **tôn trọng header cache của máy chủ gốc** (Cloudflare: Cache Rules → "Use cache-control header if present";
  CloudFront: cache policy có minimum TTL `0`, theo `Cache-Control`);
- **không bao giờ cache request có `Authorization`** — Cloudflare và CloudFront mặc định bỏ qua khi máy chủ gốc
  trả `private`; kiểm tra không có luật nào ghi đè;
- đưa **host** vào khoá cache (host chọn khách) và chuyển tiếp `Accept-Language`, `Origin` và header
  `autoxide-tenant` nếu website của bạn gửi nó;
- không nén thêm lần nữa nếu CDN đã nén — khi đó đặt `AUTOXIDE_HTTP_COMPRESSION=false`.

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
| `content.embed_hosts` | `AUTOXIDE_CONTENT_EMBED_HOSTS` |
| `analytics.allowed_hosts` | `AUTOXIDE_ANALYTICS_ALLOWED_HOSTS` (tên miền đã xác minh của khách luôn được nhận) |
| `catalog.locales` | `AUTOXIDE_CATALOG_LOCALES` |
| `menus.locales`, `menus.default_locale` | `AUTOXIDE_MENUS_LOCALES`, `AUTOXIDE_MENUS_DEFAULT_LOCALE` |
| `auth.passkey_rp_id` | `AUTOXIDE_AUTH_PASSKEY_RP_ID` (chỉ là một tên miền khách này đã xác minh; kiểm lúc đặt và mỗi lần dùng — tên miền mất xác minh hay chuyển sang khách khác thì cấu hình của máy chủ áp lại và passkey đăng ký trên tên miền đó hết dùng được) |
| `newsletter.postal_address` | `AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS`; chưa đặt thì tên và địa chỉ chủ sở hữu trong hồ sơ pháp lý của khách (module `compliance`) được dùng trước |
| `<module>.link.<trang>` (nhóm `link`, xem dưới) | địa chỉ trang `AUTOXIDE_…_URL` trong mail: `auth.link.password_reset`, `auth.link.email_verification`, `auth.link.invitation`, `auth.link.login`, `auth.link.email_change_report`, `support.link.guest_ticket`, `compliance.link.case_tracking`, `inventory.link.restock_confirm`, `notifications.link.unsubscribe`, `comments.link.verify`, `comments.link.unsubscribe`, `newsletter.link.confirm`, `newsletter.link.preferences` |

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

### Link trong mail, theo từng khách

Mail gửi cho một khách có link tới trang của website hay app của khách đó. Hai lớp, lớp sau thắng:

1. `kernel.public_url` (vd `https://shop.example`) thay scheme và host của mọi địa chỉ trang mà máy
   chủ có (`AUTOXIDE_AUTH_PASSWORD_RESET_URL`…), giữ đường dẫn.
2. Một cấu hình link (nhóm `link`, `GET /api/v1/kernel/settings?group=link`) cho cả địa chỉ trang của
   một link, kể cả đường dẫn, cho website có trang ở chỗ khác với máy chủ — vd
   `auth.link.password_reset` = `https://shop.example/dat-lai-mat-khau?token={token}`. Khách cũng bật
   được một link mà máy chủ chưa có địa chỉ.

Cả `kernel.public_url` lẫn cấu hình link đều đưa mail tới một host, nên host được kiểm
(D-20261125-tenant-site-verified):

- **Nhiều khách** (`AUTOXIDE_TENANCY_MODE=multi`): địa chỉ phải là `https://`, port mặc định, trên một tên
  miền khách **đang xác minh** (xem "Tên miền riêng của khách hàng") hay trên host của chính nền tảng
  (`AUTOXIDE_HTTP_PUBLIC_URL`). Tên miền đang chờ (kể cả sau khi xác minh lại thất bại hay khai lại tên miền
  đã thu hồi), hỏng bản ghi DNS, bị thu hồi, chưa từng khai, hay do khách khác xác minh đều bị từ chối: `PUT`
  trả `422` (`kernel.public_url_not_allowed`, `kernel.link_host_not_allowed`). **Hãy xác minh tên miền trước
  khi đặt `kernel.public_url`.**
- **Một khách** (chế độ mặc định, không có danh sách tên miền): cấu hình link phải trên host của nền tảng hay
  địa chỉ của `kernel.public_url`; `kernel.public_url` mới phải là `https://` trừ `localhost` hay `127.0.0.1`.

Cấu hình link còn phải có đủ mọi placeholder mà mô tả của nó nêu (`{token}`, `{id}`…), không có tên người
dùng, không có placeholder trong phần host (`kernel.link_placeholder_missing`,
`kernel.link_template_invalid`). Host được kiểm lại mỗi lần gửi mail: tên miền bị thu hồi, hỏng bản ghi DNS
hay chuyển sang khách khác thì mail quay về địa chỉ của máy chủ (ghi cảnh báo vào log) — ngay trên máy chủ
thấy thay đổi, trong vòng `AUTOXIDE_TENANCY_CACHE_SECONDS` (mặc định 10 giây) trên máy chủ khác. Giá trị đã
lưu trước các luật này mà không thoả thì được giữ nhưng không dùng; `GET /api/v1/kernel/settings` hiện nó kèm
`problem: kernel.public_url_unverified` hay `kernel.link_unverified`, và nó chạy lại khi tên miền được xác
minh.

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

**Gọi từ trình duyệt (CORS).** Trang ở `https://<tên miền>` của một tên miền đã xác minh gọi được API
từ trình duyệt mà không phải thêm vào `AUTOXIDE_HTTP_CORS_ORIGINS` hay khởi động lại — nhưng chỉ cho
chính khách đó: request từ trang đó mà token, header `Autoxide-Tenant` hay `Host` trỏ tới khách khác
nhận `403 tenant_origin_mismatch`, và refresh token hay link trong mail của khách khác bị coi là sai
trong mọi request từ trang đó, dù request nêu khách bằng cách nào; ngoài các trang đó, token tự nêu
khách như trước.
Đây là lớp bảo vệ thêm trong trình duyệt, không phải ranh giới bảo mật: token gửi trong header, không
bao giờ là cookie, nên trang chỉ dùng được bí mật nó đã có, còn chương trình ngoài trình duyệt gửi
`Origin` tuỳ ý hay không gửi. Chỉ đúng origin `https://<tên miền>` được tính (không `http`, không
port, không tên miền con khác); giao diện thứ hai, vd `admin.shop.example`, thì xác minh thành một tên
miền riêng. Origin trong `AUTOXIDE_HTTP_CORS_ORIGINS` (giao diện của chính nền tảng,
`http://localhost:5173` lúc phát triển) gọi được cho mọi khách: form đăng ký hay đăng nhập dùng chung của
nền tảng mà bạn đặt trên tên miền của một khách thì cũng phải thêm origin đó vào danh sách này. Tên miền bị gỡ hay xác minh hỏng mất
quyền ngay trên máy chủ đã đổi nó, và trong `AUTOXIDE_TENANCY_CACHE_SECONDS` (mặc định 10 giây) trên
các máy khác.

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
`POST /api/v1/auth/registrations` và đăng nhập bằng `POST /api/v1/auth/login`, cùng với
`"realm": "customer"`; khách đóng đăng ký người mua bằng cài đặt theo cây tổ chức
`auth.customer_registration` (`false`). Nhân sự quản lý người mua bằng `GET /api/v1/auth/customers`
và `PATCH /api/v1/auth/customers/{id}` (permission `auth.customers.read`, `auth.customers.manage`).

**Chính sách đăng nhập theo khách.** Cài đặt tổ chức ở nút gốc của khách (`PUT
/api/v1/organizations/units/{id}/settings/{key}`) cho khách tự quyết, trong giới hạn của máy chủ:

| Cài đặt | Tác dụng |
| ------- | -------- |
| `auth.staff_registration` | `true` / `false`: ai cũng tự tạo được tài khoản nhân sự (`POST /api/v1/auth/register`, hay đăng nhập lần đầu qua nhà cung cấp cho đăng ký). Chưa đặt: `AUTOXIDE_AUTH_SELF_REGISTRATION` |
| `auth.password_min_length` | 12 tới 128: độ dài tối thiểu của mật khẩu mới. Chỉ nâng mức 12 của máy chủ |
| `auth.session_days` | 1 tới 365: thời hạn mỗi refresh token. Chỉ rút ngắn `AUTOXIDE_AUTH_REFRESH_TOKEN_DAYS`; giá trị dài hơn bị bỏ qua |
| `auth.totp_issuer` | Tên hiện trong app xác thực (không có `:`), thay `AUTOXIDE_AUTH_TOTP_ISSUER` |

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
`AUTOXIDE_AUTH_LOGIN_LINK_URL` là trang của giao diện. **Đăng nhập bằng Google, Microsoft, GitHub
hay OpenID Connect** tắt mặc định; bật bằng `AUTOXIDE_AUTH_OIDC=true`. Mỗi khách đăng ký ứng dụng ở
nhà cung cấp rồi thêm bằng `POST /api/v1/auth/oidc-providers` ([`docs/guides/oidc.vi.md`](guides/oidc.vi.md));
client secret lưu mã hoá bằng `AUTOXIDE_SECRET_KEY`. Máy chủ chỉ gọi nhà cung cấp ở địa chỉ public;
giữ `AUTOXIDE_AUTH_OIDC_ALLOW_PRIVATE_NETWORKS=false` khi chạy thật, và cho máy chủ gọi ra ngoài qua
HTTPS (cổng 443). Link và lời mời cần mail chạy được
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

   Nhân sự nền tảng cũng migrate được các placement qua API, không cần vào máy chủ:
   `POST /api/v1/kernel/placements/migrations` với `{"dry_run": true}` liệt kê những gì sẽ chạy;
   bỏ `dry_run` thì một lượt chạy bắt đầu ở nền (theo dõi bằng
   `GET /api/v1/kernel/placements/migrations/{id}`). `main` luôn được migrate bằng
   `autoxide migrate up` trước khi máy chủ chạy.

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

**Nơi lưu dữ liệu.** Ghi nước đặt máy chủ của từng placement bằng `autoxide placement country <tên> VN`
(`-` khi chưa rõ; `autoxide placement list` cho xem). Khách buộc lưu dữ liệu ở một nước (nhân sự nền tảng đặt
bằng `PUT /api/v1/compliance/residency`) không dời được sang placement ở nước khác hay chưa rõ nước: lượt dời
trả `tenant_residency_violation`. Xem [COMPLIANCE.vi.md](COMPLIANCE.vi.md#nơi-lưu-dữ-liệu).

## Tách một nhánh thành khách riêng, và gộp khách

Cây tổ chức của một khách có thể lớn hơn khách đó: một vùng thành công ty riêng, hay một cửa hàng
đang có khách riêng nhập vào chuỗi. Nhân sự nền tảng tách một nhánh thành khách mới ở database
khác, hoặc gộp một khách vào khách khác dưới một nút của nó. Mọi dòng giữ nguyên ID, nên tách rồi
gộp lại cho đúng dữ liệu ban đầu.

```bash
# Tách nút <north> của `acme` thành khách `acme-north` ở placement `acme-north-db`.
autoxide tenant split acme --org <north> --slug acme-north --name "Acme North" \
  --placement acme-north-db --reason "Hợp đồng 2026-11: vùng thành công ty riêng" --dry-run
# Gộp `acme-north` trở lại `acme` dưới nút <gốc-hay-nút-khác>.
autoxide tenant merge acme-north --into acme --under-org <unit> --reason "…" --dry-run
```

- **Chạy thử trước.** Kế hoạch liệt kê, cho từng bảng, số dòng đi, được chép, ở lại, hay phải chạy
  xong trước (event chưa giao, job đến hạn); những gì chặn (khách không hoạt động, một lần tách /
  gộp / dời khác đang dở, thiếu migration, đích là cùng database); và **xung đột**: dòng trỏ vắt qua
  ranh giới (dòng đi trỏ tới dòng ở lại, hay ngược lại) và giá trị unique khách nhận đã có (hai
  trang cùng slug). Sửa xong thì chạy lại đúng lệnh đó, bỏ `--dry-run`.
- **Hai database.** Placement đích phải là database khác database của khách nguồn. Muốn tách vào
  database chung, hay gộp hai khách cùng database, hãy dời một khách trước (`autoxide tenant move`).
- **Chuyện gì xảy ra.** Khách (cả hai khách khi gộp) trả `503 tenant_maintenance`; event chưa giao
  và job đến hạn của khách chạy hết trước (`AUTOXIDE_RELOCATION_DRAIN_SECONDS`, mặc định 300). Rồi
  các dòng được chép trong một transaction ở đích, giữ nguyên ID; khách mới mang ID của nút gốc
  nhánh, nút đó thành gốc của nó. Các dòng được kiểm, hai khách chuyển trong một transaction (tên
  miền gắn nút trong nhánh đi theo; khách bị gộp thành `merged`), rồi dòng đã đi bị xoá ở nguồn. File
  của thứ đi theo được chép sang tiền tố của khách mới. Audit log về thứ đi theo được ghi lại vào
  chuỗi băm của đích; nguồn giữ nguyên chuỗi của mình. Sổ cái đi theo nút của sổ và chuỗi niêm phong
  vẫn kiểm được. Kho báo cáo của cây đi theo (`reporting.rehome`).
- **Thứ ở lại.** Mỗi module khai bảng của mình đi thế nào (`Module::relocation`): dữ liệu cá nhân
  của khách (người đăng ký, đồng ý, định danh, bình luận) ở lại khách nguồn khi tách và đi theo khi
  gộp; tài khoản nhân viên có role ở nút trong nhánh được chép (đăng nhập lại); role và cấu hình
  tương tự được chép; thuê bao, phiên đăng nhập, bộ đếm ở lại.
- **Căn cứ pháp lý.** Chuyển dữ liệu giữa hai khách có thể là chuyển dữ liệu cá nhân giữa hai bên
  kiểm soát; `--reason` là bắt buộc, được lưu cùng lần chạy và vào audit log.
- **Chạy tiếp.** Dừng giữa chừng thì chạy `autoxide tenant split --resume <id>` (hay `merge`); mọi
  bước chạy lại được. `autoxide tenant relocation-status <id>` cho biết trạng thái và lỗi.

Qua API, nhân sự nền tảng có `kernel.tenants.relocate` gọi `POST /api/v1/kernel/relocations`
(`{"kind": "split", "tenant": "acme", "org_id": "…", "slug": "…", "name": "…", "placement": "…",
"reason": "…"}`): đó là chạy thử; thêm `"dry_run": false` để chạy bằng job nền và theo dõi ở
`GET /api/v1/kernel/relocations/{id}`. Khách không tự tách / gộp: quản trị viên của khách
(`kernel.relocations.request`) gửi yêu cầu bằng `POST /api/v1/kernel/tenant/relocation-requests`,
yêu cầu vào hộp việc của nhân sự nền tảng, người chạy nó bằng `"request_id"`.

## Sự cố dữ liệu cá nhân

Khi dữ liệu cá nhân có thể đã lộ (mất bản backup, database bị mở ra ngoài, gửi nhầm mail), ở Việt Nam báo cáo
gửi cơ quan có hạn **72 giờ kể từ khi phát hiện**. Ghi sự cố vào API ngay (`POST /api/v1/compliance/incidents`),
đặt chính sách `incident.alert_emails` của khách để lời nhắc tới đúng người, và làm theo sổ tay ở
[COMPLIANCE.vi.md](COMPLIANCE.vi.md#sự-cố-dữ-liệu-cá-nhân-sổ-tay-72-giờ). `autoxide doctor` cảnh báo khi sự cố
đã quá hạn.

## Chạy nhiều máy chủ

Với PostgreSQL, chạy bao nhiêu tiến trình `app` cũng được sau reverse proxy: chúng chỉ
dùng chung database (và S3 cho file), và job nền được nhận sao cho mỗi job chạy đúng một
lần. Đặt cùng `AUTOXIDE_AUTH_TOKEN_SECRET` ở mọi nơi. Muốn tăng API và việc nền riêng
rẽ, chạy vài tiến trình với `AUTOXIDE_PROCESS_ROLE=api` và ít nhất một tiến trình
`worker`. SQLite chỉ cho một tiến trình máy chủ.

Mặc định giới hạn request đếm theo từng tiến trình: với N máy chủ, một địa chỉ hay một API key
được tới N lần giới hạn. Muốn các máy chủ chia chung giới hạn theo API key, đặt
`AUTOXIDE_HTTP_RATE_LIMIT_STORE=database`: mọi máy chủ đếm trong database, lấy lượt theo lô nhỏ
(một phần tư `AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_BURST`), nên mỗi phút một key được vượt giới hạn
nhiều nhất một lô cho mỗi máy chủ. Giới hạn theo địa chỉ vẫn đếm theo từng tiến trình.

## Kubernetes

[`deploy/kubernetes/autoxide.yaml`](../deploy/kubernetes/autoxide.yaml) là một ví dụ tối thiểu, có chú
thích: một `Job` chạy migration, một `Deployment` hai pod, một `Service` và một `Ingress` có HTTPS. Nó theo
đúng các luật ở [Chạy nhiều máy chủ](#chạy-nhiều-máy-chủ):

- PostgreSQL chạy bên ngoài file này (database có quản lý hay operator của bạn); SQLite chỉ cho một tiến
  trình nên không hợp ở đây.
- File lưu trên S3 (`AUTOXIDE_FILES_S3_BUCKET`), thư gửi qua SMTP (`AUTOXIDE_MAIL_SMTP_URL`): pod không giữ
  gì trên đĩa, nên pod nào cũng thay được.
- Secret (`DATABASE_URL`, `AUTOXIDE_AUTH_TOKEN_SECRET`, `AUTOXIDE_SECRET_KEY`, key S3, URL SMTP) nằm trong
  một `Secret` của Kubernetes bạn tạo bằng `kubectl create secret`, không bao giờ nằm trong file.
- Migration chạy một lần mỗi bản phát hành trong `Job`; pod từ chối khởi động khi còn migration chưa áp
  dụng, nên pod cũ vẫn phục vụ tới khi `Job` xong.
- `readinessProbe` trỏ `/health/ready`, `livenessProbe` trỏ `/health/live`, khoảng rút 15 giây và
  `terminationGracePeriodSeconds: 50` cho phép cập nhật cuốn chiếu không rơi request (xem
  [Cập nhật không làm rơi request](#cập-nhật-không-làm-rơi-request)).
- `AUTOXIDE_HTTP_RATE_LIMIT_STORE=database` chia giới hạn theo key giữa các pod, và
  `AUTOXIDE_HTTP_CLIENT_IP_HEADER` với một proxy tin cậy đọc địa chỉ client mà ingress controller chuyển
  tiếp.
- Pod chạy bằng người dùng không phải root của image (`10001`), không thêm đặc quyền.

Lệnh tạo secret, áp file và cập nhật nằm ở đầu file. Muốn tách API khỏi việc nền thì chép `Deployment`
thành một cái thứ hai với `AUTOXIDE_PROCESS_ROLE=worker` (không cần `Service`, ít nhất một pod) và đặt
`AUTOXIDE_PROCESS_ROLE=api` cho cái thứ nhất.

## Tải thẳng lên S3

Khi file nằm trên S3, client tải được file lớn thẳng lên bucket (`POST /api/v1/files/uploads`,
hướng dẫn: [`docs/guides/direct-uploads.vi.md`](guides/direct-uploads.vi.md)). Trình duyệt cần
bucket cho phép (CORS). Ví dụ với Amazon S3:

```json
[{"AllowedOrigins": ["https://admin.example.com"], "AllowedMethods": ["PUT"],
  "AllowedHeaders": ["content-type", "x-amz-checksum-sha256"], "MaxAgeSeconds": 3600}]
```

Máy chủ kiểm mọi lượt được xác nhận bằng cách đọc lại file (cỡ và SHA-256), vì không phải dịch
vụ tương thích S3 nào cũng kiểm checksum đã ký; tăng `AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS` nếu
cho phép file rất lớn. Lượt bỏ dở bị một job chạy mỗi giờ xoá.

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

## Backup theo lịch

Ngoài `autoxide backup` chạy tay, mọi tiến trình chạy việc nền backup mọi database nó lo, mỗi
`AUTOXIDE_BACKUP_INTERVAL_HOURS` giờ (mặc định 24; `0` để tắt), cùng cách với `autoxide backup`, vào
nơi lưu file của module `files` (thư mục hay bucket S3) dưới `kernel/backups/<database>/<id>/`. Mỗi
database chỉ giữ `AUTOXIDE_BACKUP_KEEP` bản tốt mới nhất (mặc định 7). Mỗi bản gồm `backup.json`,
`database.sqlite` và, khi đặt `AUTOXIDE_BACKUP_INCLUDE_FILES=true`, các file đã lưu trong `files/`.

- Bản backup nằm cùng nơi lưu với file tải lên, nên nó bảo vệ khi dữ liệu mất hay hỏng, không bảo vệ
  khi mất chính nơi lưu đó: chép bucket hay thư mục (cả backup lẫn file tải lên) sang nơi khác, vd
  bằng tính năng nhân bản của bucket hay một lệnh `rclone sync` hằng đêm. Đó cũng là lý do mặc định
  không chép file vào từng bản backup.
- Muốn khôi phục một bản thì chép thư mục của nó ra khỏi nơi lưu (S3:
  `aws s3 cp --recursive s3://<bucket>/kernel/backups/main/<id>/ ./restore/`; thư mục: `cp -r`), rồi khôi phục
  thư mục đó như mục "Khôi phục từng bước" bên dưới.
- Mỗi lượt được ghi vào bảng `kernel_backups` (trạng thái, cỡ, lỗi). `autoxide doctor` cảnh báo khi một
  database không có bản tốt nào trong hai chu kỳ gần nhất, và `/metrics` có
  `backup_last_success_timestamp_seconds`, `backup_runs_total`, `backup_duration_seconds` và
  `backup_size_bytes` (cảnh báo `AutoxideBackupTooOld`, `AutoxideBackupFailed`).
- Bản backup chứa dữ liệu cá nhân. Dữ liệu đã xoá theo yêu cầu của một người vẫn còn trong các bản
  backup làm trước đó, tới khi chúng bị xoay vòng (`AUTOXIDE_BACKUP_KEEP` × chu kỳ): ghi điều này vào
  thông báo xử lý dữ liệu. Giữ nơi lưu ở chế độ riêng tư, ở nước mà dữ liệu của khách phải ở, và giữ
  `AUTOXIDE_SECRET_KEY` tách khỏi nó. Chứng từ kế toán phải giữ 10 năm nằm ở module `invoices` và
  `ledger`, không nằm trong bản backup xoay vòng. (Chi tiết pháp lý: cần đối chiếu văn bản gốc / người
  có chuyên môn pháp lý.)
- Trong lúc backup một database SQLite, file `-wal` của nó lớn dần tới khi backup xong: chừa chỗ trống
  trên đĩa cho nó.

### Khôi phục từng bước

Tập trước khi cần: test `dien_tap_backup_restore_roi_khoi_dong` làm đúng các bước này ở mỗi lần chạy
gate.

1. **Lấy thư mục backup.** Bản backup theo lịch: chép `kernel/backups/<database>/<id>/` ra khỏi nơi lưu
   file (S3: `aws s3 cp --recursive s3://<bucket>/kernel/backups/main/<id>/ ./restore/`; thư mục:
   `cp -r`). Lấy dòng `succeeded` mới nhất trong `kernel_backups`, hay thư mục mới nhất. Bản của
   `autoxide backup`: dùng nguyên thư mục đó. Chỉ khôi phục bản backup do chính bạn tạo.
2. **Chuẩn bị đích trống**: một database mới, trống (`createdb` trên PostgreSQL, hay đường dẫn tới một
   file SQLite chưa có) và một nơi lưu file trống. Dùng cùng `AUTOXIDE_SECRET_KEY` như lúc tạo bản
   backup, không thì secret đã mã hoá trong đó không đọc được. Bản backup lấy từ PostgreSQL khôi phục
   được vào SQLite và ngược lại.
3. **Dừng các máy chủ** trỏ vào đích, rồi chạy `autoxide restore <thư mục>` với `DATABASE_URL` và cấu
   hình nơi lưu file của đích. Lệnh áp dụng migration của phiên bản này, chép mọi bảng và file, và từ
   chối database đã có bảng của Autoxide.
4. **Khởi động và kiểm**: `autoxide doctor`, rồi `autoxide serve`; `/health/ready` phải trả `200` và quản
   trị viên phải đăng nhập được. Ở chế độ nhiều khách, chạy `autoxide migrate up --all` trước.
5. **Chế độ nhiều khách**: một bản backup chứa một database. Khôi phục database của từng placement theo
   cùng cách. Muốn lấy lại dữ liệu của riêng một khách: khôi phục bản backup vào một bản cài tạm rồi đọc
   hay xuất dữ liệu của khách đó từ đó; chưa có lệnh khôi phục một khách vào database đang chạy.

## Backup mã hoá ở nơi khác và khôi phục về một thời điểm

`autoxide backup` và backup theo lịch là ảnh chụp nhất quán: khôi phục một bản đưa dữ liệu về đúng lúc
chụp, còn những gì xảy ra sau đó thì mất. Hai phần dưới đây bù chỗ còn lại.

### Mã hoá backup trước khi rời máy

Một bản backup chứa dữ liệu của mọi module, kể cả dữ liệu cá nhân. Mã hoá nó trước khi chép sang máy khác
hay lên bucket, ví dụ bằng [age](https://age-encryption.org) với một cặp khoá mà nửa riêng tư cất ngoại
tuyến:

```bash
age-keygen -o backup-key.txt            # một lần; cất backup-key.txt ngoại tuyến, tách khỏi backup
age-keygen -y backup-key.txt            # in khoá công khai age1…, dùng ở dưới

autoxide backup "backups/$(date +%F)"
tar -C backups -c "$(date +%F)" | age -r age1… > "autoxide-$(date +%F).tar.age"
# chép autoxide-<ngày>.tar.age đi nơi khác, rồi xoá thư mục chưa mã hoá

# khôi phục: giải mã, rồi khôi phục thư mục như ở "Khôi phục từng bước"
age -d -i backup-key.txt autoxide-2026-11-21.tar.age | tar -x -C restore/
```

Cũng giữ `AUTOXIDE_SECRET_KEY` tách khỏi backup: thiếu nó thì secret đã mã hoá bên trong backup không đọc
được, và đó chính là mục đích.

### Khôi phục về một thời điểm trên PostgreSQL

Muốn đưa database về đúng trạng thái ở một giây bất kỳ — ví dụ ngay trước một lần sửa hàng loạt sai —
PostgreSQL phải giữ nhật ký ghi trước (WAL) bên cạnh các bản backup nền định kỳ. Autoxide không cần gì đặc
biệt: đây là cách cài chuẩn của PostgreSQL.

- **PostgreSQL có quản lý** (Amazon RDS, Google Cloud SQL, Azure, …): bật backup tự động có khôi phục về
  thời điểm (point-in-time recovery) và mã hoá ổ lưu trong trang quản lý của nhà cung cấp; khôi phục tạo
  một database mới ở thời điểm bạn chọn. Trỏ `DATABASE_URL` tới nó.
- **Máy chủ của bạn**: [pgBackRest](https://pgbackrest.org) giữ backup nền và WAL trong một bucket, mã hoá
  ngay trên máy database trước khi gửi đi. Phác thảo (đối chiếu hướng dẫn của pgBackRest đúng phiên bản bạn
  dùng; trên Debian hay Ubuntu: `apt install pgbackrest`; với container PostgreSQL của `compose.yml` thì
  dùng một image có thêm nó):

  ```ini
  # /etc/pgbackrest/pgbackrest.conf
  [global]
  repo1-type=s3
  repo1-s3-bucket=<bucket>
  repo1-s3-endpoint=s3.<region>.amazonaws.com
  repo1-s3-region=<region>
  repo1-path=/pgbackrest
  # mã hoá phía client: một giá trị ngẫu nhiên dài, cất tách khỏi bucket
  repo1-cipher-type=aes-256-cbc
  repo1-cipher-pass=<openssl rand -base64 48>
  repo1-retention-full=2

  [autoxide]
  pg1-path=/var/lib/postgresql/16/main
  ```

  ```ini
  # postgresql.conf
  wal_level = replica
  archive_mode = on
  archive_command = 'pgbackrest --stanza=autoxide archive-push %p'
  ```

  Key S3 đặt trong `PGBACKREST_REPO1_S3_KEY` và `PGBACKREST_REPO1_S3_KEY_SECRET` (hoặc `repo1-s3-key`,
  `repo1-s3-key-secret` trong một file chỉ `postgres` đọc được). Rồi, bằng người dùng `postgres`:

  ```bash
  pgbackrest --stanza=autoxide stanza-create
  pgbackrest --stanza=autoxide check
  pgbackrest --stanza=autoxide --type=full backup   # hằng tuần, từ cron
  pgbackrest --stanza=autoxide --type=diff backup   # hằng ngày, từ cron
  ```

  Muốn quay về một thời điểm: dừng các máy chủ Autoxide và PostgreSQL, rồi

  ```bash
  pgbackrest --stanza=autoxide --delta --type=time --target-action=promote \
    "--target=2026-11-21 09:59:00+00" restore
  ```

  khởi động PostgreSQL, chạy `autoxide doctor`, rồi khởi động các máy chủ.

Khôi phục về thời điểm chỉ áp cho database. File trên S3 không quay lui theo: bật versioning của bucket để
lấy lại file đã xoá hay bị ghi đè. Ở chế độ nhiều khách, mỗi placement là một database riêng và cần cài
riêng. Tập khôi phục trước khi cần, như với ảnh chụp.

## Theo dõi (monitoring)

Bật `AUTOXIDE_HTTP_METRICS=true`. `/metrics` không có xác thực, nên cho nó port riêng bằng
`AUTOXIDE_HTTP_METRICS_ADDRESS` — `127.0.0.1:9090` trên một máy, `0.0.0.0:9090` trong container hay pod
Kubernetes (chỉ mở port đó cho Prometheus, không bao giờ ra internet) — rồi trỏ Prometheus vào đó;
`/metrics` khi ấy rời port của API. Không có biến đó thì `/metrics` nằm cùng port với API: chặn nó ở
reverse proxy và để Prometheus đọc ngay trên máy. Ngoài số đo request (`http_requests_total`, `http_request_duration_seconds`), mọi tiến
trình chạy việc nền (`AUTOXIDE_PROCESS_ROLE` là `all` hay `worker`) cập nhật các gauge tồn đọng mỗi
`AUTOXIDE_METRICS_BACKLOG_SECONDS` giây (mặc định 30), gắn nhãn `database` và `module`:

| Gauge | Nghĩa |
|---|---|
| `jobs_pending` | Job chưa xong, kể cả job hẹn giờ |
| `jobs_oldest_pending_seconds` | Job tới hạn chờ lâu nhất đã chờ bao lâu (0 khi không có) |
| `jobs_dead` | Job trong dead-letter queue (`autoxide jobs failed`) |
| `events_pending`, `events_oldest_pending_seconds`, `events_dead` | Như trên, cho event đang chờ giao tới bên nhận |
| `database_connections{pool,state}`, `database_connections_max{pool}` | Kết nối của pool `read` và `write`, `in_use` hay `idle`, và số tối đa |

Nhãn `database` là tên database (`main`, hay placement của khách có database riêng); quá
`AUTOXIDE_METRICS_MAX_DATABASE_LABELS` (mặc định 50) database thì phần còn lại cộng chung dưới
`database="other"`. Không có nhãn theo khách: tra một khách bằng log và trace.

**Một khách.** Số đo không có nhãn theo khách (mỗi khách một nhãn làm số chuỗi của Prometheus tăng
không giới hạn). Để xem một khách:

- **Log và trace.** Mỗi dòng log của request mang `tenant_id` khi đã biết khách (log JSON: ở
  `span.tenant_id`), span của job nền (`job`), lượt giao event (`event_delivery`) và tool MCP
  (`mcp_tool`) cũng vậy; khi đặt `OTEL_EXPORTER_OTLP_ENDPOINT`, trace mang cùng thuộc tính `tenant_id`.
  Lọc theo nó: `jq 'select(.span.tenant_id == "<id>")'`, hay `tenant_id="<id>"` trong công cụ tìm
  log / trace.
- **Lượng dùng.** `GET /api/v1/plans/tenants/{tenant_id}/usage` (module `plans`) cho biết khách dùng
  mỗi hạn mức tới đâu.
- **Tồn đọng.** `GET /api/v1/kernel/ops/backlog` (nhân sự nền tảng, `kernel.ops.read`) đếm job và event
  đang chờ của từng database và module; khách có database riêng là một dòng riêng.

Repo có sẵn hai file:

- [`deploy/prometheus/autoxide-alerts.yml`](../deploy/prometheus/autoxide-alerts.yml): luật cảnh báo —
  lỗi máy chủ trên 2%, p95 chậm, job chờ quá 10 phút, job vào dead-letter queue, event chưa giao quá 5
  phút, pool kết nối dùng 90%. Mỗi luật có chú thích `action` nói phải làm gì; các bước nằm trong sổ
  tay bên dưới. Thêm file vào `rule_files` trong `prometheus.yml` và chuyển cảnh báo qua Alertmanager.
- [`deploy/grafana/autoxide-dashboard.json`](../deploy/grafana/autoxide-dashboard.json): dashboard
  Grafana (Dashboards › New › Import) có request, lỗi, p95 theo route, tồn đọng, dead-letter queue và kết
  nối, lọc theo database.

Có nhiều tiến trình worker thì mỗi tiến trình đặt cùng gauge (mỗi `instance` một chuỗi); luật cảnh báo
và dashboard lấy giá trị lớn nhất, không cộng.

## Sổ tay xử lý sự cố

Sự cố cần quyết định và nhiều bước (lộ key, thu hồi hàng loạt, khôi phục một khách, mất dữ liệu) ở [`RUNBOOK.vi.md`](RUNBOOK.vi.md).

Mỗi tình huống: dấu hiệu, cách kiểm, cách sửa.

### Lỗi máy chủ

- **Dấu hiệu:** cảnh báo `AutoxideHigh5xxRate`; client nhận lỗi `500`.
- **Kiểm:** mọi response lỗi có `request_id`; tìm nó trong log. `autoxide doctor` kiểm một lượt cấu
  hình, database, migration, nơi lưu file và mail.
- **Sửa:** theo log (database mất kết nối, đầy đĩa, dịch vụ ngoài hỏng). Lỗi bên trong code là bug: báo
  kèm dòng log, không kèm secret hay dữ liệu cá nhân.

### Request chậm

- **Dấu hiệu:** cảnh báo `AutoxideSlowP95` nêu tên route.
- **Kiểm:** `database_connections` có gần mức tối đa không (request đứng chờ kết nối), và kế hoạch
  truy vấn của route đó (`EXPLAIN ANALYZE` trên PostgreSQL, `EXPLAIN QUERY PLAN` trên SQLite).
- **Sửa:** thiếu index thì thêm bằng migration mới của module sở hữu bảng; pool quá nhỏ thì xem mục "Kết
  nối database".

### Hàng đợi kẹt

- **Dấu hiệu:** cảnh báo `AutoxideJobsStuck` hay `AutoxideEventsBacklog`; mail, webhook, báo cáo tới
  muộn.
- **Kiểm:** có ít nhất một tiến trình chạy với `AUTOXIDE_PROCESS_ROLE` là `all` hay `worker` không
  (`/health/ready` của tiến trình đó, log của nó); `jobs_pending` theo module cho thấy module nào đang
  chậm; ở chế độ nhiều khách, database còn đang migration (`503 tenant_maintenance`) thì chờ tới khi
  `autoxide migrate up --all` xong.
- **Sửa:** chạy (hay khởi động lại) worker; tăng `AUTOXIDE_JOBS_CONCURRENCY` hay thêm tiến trình worker
  khi một module luôn có hàng đợi dài. Event đứng chờ một bên nhận cứ lỗi: lỗi của nó nằm trong log
  (`event delivery failed`).

### Dead-letter queue

- **Dấu hiệu:** cảnh báo `AutoxideDeadLetterGrowing`; `jobs_dead` tăng.
- **Kiểm:** `autoxide jobs failed` liệt kê job đã bỏ cuộc, mới nhất trước, kèm lỗi cuối (`--module` để
  lọc).
- **Sửa:** sửa nguyên nhân trước (sai mật khẩu SMTP, dịch vụ ngoài hỏng), rồi
  `autoxide jobs retry <module> <id>` cho từng job. Chạy lại trước khi sửa thì job chỉ vào hàng đợi để
  lỗi lần nữa.

### Quá nhiều kết nối

- **Dấu hiệu:** request phải chờ trước khi có byte trả lời nào; `/health/ready` chậm hay hết giờ, trong khi
  process vẫn khoẻ.
- **Kiểm:** số kết nối đang mở (`ss -tn state established '( sport = :8080 )' | wc -l`) so với
  `AUTOXIDE_HTTP_MAX_CONNECTIONS`; chạm giới hạn thì kết nối mới — kể cả health check cùng port — chờ tới khi
  một kết nối đóng.
- **Sửa:** giới hạn số kết nối theo từng client ở reverse proxy (phần lớn một đợt tràn đến từ ít client), rồi
  nâng setting cùng giới hạn file mở. Đặt timeout và ngưỡng lỗi của liveness probe đủ chịu một lúc quá tải ngắn
  (`/health/live`, vd 5 giây × 3): khởi động lại làm rơi mọi kết nối; readiness probe chậm chỉ đưa máy ra khỏi
  vòng phân tải, đúng điều mong muốn khi máy đang đầy.

### Kết nối database

- **Dấu hiệu:** cảnh báo `AutoxideDbPoolExhausted`; request chậm vì chờ kết nối.
- **Kiểm:** `database_connections{state="in_use"}` theo pool; trên PostgreSQL, truy vấn chạy lâu trong
  `pg_stat_activity`.
- **Sửa:** dừng thứ đang giữ kết nối (truy vấn chậm, job kẹt), rồi tăng
  `AUTOXIDE_DATABASE_READ_CONNECTIONS` / `AUTOXIDE_DATABASE_WRITE_CONNECTIONS` — mỗi tiến trình mở chừng
  ấy kết nối, nên giữ tổng dưới `max_connections` của máy chủ database. SQLite theo thiết kế chỉ có một
  kết nối ghi.

### Database đầy

- **Dấu hiệu:** lệnh ghi lỗi, log có "disk full" hay "no space left on device".
- **Kiểm:** SQLite: cỡ file database và file `-wal` của nó (một lần đọc kéo dài, như backup, làm file
  `-wal` lớn dần tới khi xong). PostgreSQL: các bảng lớn nhất,
  `SELECT relname, pg_size_pretty(pg_total_relation_size(relid)) FROM pg_statio_user_tables ORDER BY
  pg_total_relation_size(relid) DESC LIMIT 10;`.
- **Sửa:** giải phóng hay thêm dung lượng đĩa trước. Job đã xong và event đã giao tự bị xoá sau 7 ngày;
  trên PostgreSQL chạy `VACUUM` cho bảng lớn sau khi xoá nhiều dòng.

### Backup

- **Dấu hiệu:** cảnh báo `AutoxideBackupTooOld` hay `AutoxideBackupFailed`; `autoxide doctor` cảnh báo
  `kernel.backup_age`.
- **Kiểm:** các dòng mới nhất của `kernel_backups` cho database đó (`status`, `error`), dòng log
  `backup failed`, chỗ trống trên máy chủ (bản backup được dựng trước trong thư mục tạm, cỡ bằng
  database) và trên nơi lưu file.
- **Sửa:** theo lỗi (thiếu module `files`, đầy đĩa, thông tin đăng nhập nơi lưu). Dòng kẹt ở `running`
  do tiến trình đã chết được lượt sau đánh `failed` sau 15 phút. Backup tay bằng
  `autoxide backup <thư mục>` tới khi backup theo lịch chạy lại.

### Lộ khoá

- `AUTOXIDE_AUTH_TOKEN_SECRET`: đặt giá trị mới trên mọi máy chủ rồi khởi động lại, KHÔNG đưa khoá bị lộ
  vào `AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS`. Mọi access token hết dùng được; client lấy token mới bằng
  refresh token, vẫn còn hiệu lực. (Đổi định kỳ thì giữ khoá cũ ở đó một lúc để không ai nhận ra — xem
  "Lấy mã nguồn và đặt secret".)
- `AUTOXIDE_SECRET_KEY`: xoay như mục "Lấy mã nguồn và đặt secret" (giá trị mới, giá trị cũ vào
  `AUTOXIDE_SECRET_KEY_PREVIOUS`, `autoxide secrets reseal`). Bản backup lấy trước đó vẫn mở được bằng
  khoá cũ: xoá chúng, hay giữ chúng an toàn như chính khoá. Bộ đếm theo địa chỉ của người xem bắt đầu
  lại với khoá mới.
- API key hay secret của webhook: thu hồi key, xoay secret của webhook qua API.

### Mất quản trị viên cuối cùng

- **Dấu hiệu:** không ai đăng nhập được với role `admin` (tài khoản bị xoá, bị khoá, hay người đó đã
  nghỉ). `autoxide doctor` cảnh báo khi không còn quản trị viên nào đang hoạt động.
- **Sửa:** trên máy chủ, `autoxide user create --email … --name … --admin` tạo quản trị viên mới và in
  mật khẩu. Quản trị viên chỉ mất bước xác thực thứ hai hay passkey:
  `autoxide user reset-mfa` / `autoxide user remove-passkeys`.

### Khoá migration

Mỗi lúc chỉ một tiến trình migrate. Có hai khoá: một cho database chính (`migrate up`, máy chủ khởi
động với `AUTOXIDE_MIGRATE_ON_START`) và một cho database của khách (`migrate up --all` lấy nó sau khoá
đầu, lượt migrate chạy nền và việc chuyển khách chỉ lấy khoá này), nên một lượt dài qua database của
khách không bao giờ làm máy chủ đang khởi động phải chờ. Trên PostgreSQL mỗi khoá là một advisory lock
trên kết nối riêng; proxy hay failover cắt kết nối đó thì khoá mất, và lượt dừng trước database của
khách kế tiếp (chạy lại). Dùng PgBouncer ở chế độ session cho lệnh này, hoặc trỏ thẳng vào database.
Trên SQLite khoá là các file cạnh database (`.migrate-lock`, `.placements-lock`); SQLite trong bộ nhớ
không có khoá (chỉ một tiến trình). `migrate up --all` giữ khoá chính trong lúc chờ (tới 15 phút) khoá
database của khách, nên trong lúc đó máy chủ khởi động có việc lúc khởi động (`AUTOXIDE_MIGRATE_ON_START`,
quản trị viên đầu tiên, task khởi động) cũng phải chờ. Trong lúc cập nhật cuốn chiếu từ một bản cũ hơn khoá này, máy chủ
bản cũ còn có thể chạy lượt migrate mà không giữ khoá: chờ cập nhật xong rồi mới chạy lượt từ API.

### Cập nhật khi hai phiên bản cùng chạy

Lúc cập nhật cuốn chiếu, máy chủ cũ và mới cùng chạy trên một database, nên một bản phát hành đổi
database theo hai bước ("mở rộng trước, thu hẹp sau"):

1. **Mở rộng:** bản mới thêm thứ nó cần (cột mới cho phép trống hay có giá trị mặc định; bảng mới) và ghi
   cả dạng cũ lẫn dạng mới. Bản cũ vẫn chạy: nó không biết cột mới.
2. **Thu hẹp:** một bản phát hành sau, khi không còn máy chủ nào chạy bản cũ, thôi ghi dạng cũ và bỏ nó.

`autoxide serve` không khởi động khi còn thiếu migration, nên chạy `autoxide migrate up --all`
(database chính rồi mọi database riêng của khách; ở chế độ một khách giống `migrate up`) trước khi
bật máy chủ mới, rồi thay máy chủ cũ như mục "Cập nhật không làm rơi
request".

## Kiểm trước khi chạy thật

- [ ] `AUTOXIDE_AUTH_TOKEN_SECRET` và `AUTOXIDE_SECRET_KEY` là chuỗi ngẫu nhiên dài, giữ bí mật
      (`autoxide doctor` cảnh báo khi thiếu)
- [ ] Có TLS phía trước (reverse proxy), port 8080 không mở ra internet
- [ ] `AUTOXIDE_HTTP_CLIENT_IP_HEADER` đặt theo header của proxy
- [ ] Đã cấu hình SMTP, để đặt lại mật khẩu và xác minh email chạy được
- [ ] Backup theo lịch và chép sang máy khác; đã thử khôi phục một lần
- [ ] `/health/ready` được hệ thống giám sát theo dõi
- [ ] Đã ghi nước của placement (`autoxide placement country`) khi có khách buộc lưu dữ liệu tại Việt Nam, và
      đặt `incident.alert_emails` để nhận nhắc hạn 72 giờ của sự cố
