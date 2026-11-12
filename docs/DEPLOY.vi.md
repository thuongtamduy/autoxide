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

Tạo file `.env` cạnh `compose.yml` chứa hai secret. Không bao giờ commit file này.

```bash
printf 'POSTGRES_PASSWORD=%s\nAUTOXIDE_AUTH_TOKEN_SECRET=%s\n' \
  "$(openssl rand -hex 24)" "$(openssl rand -base64 48 | tr -d '\n')" > .env
chmod 600 .env
```

- `POSTGRES_PASSWORD` bảo vệ database.
- `AUTOXIDE_AUTH_TOKEN_SECRET` ký access token. Giữ nguyên giá trị: đổi nó thì mọi người
  bị đăng xuất.

### 2. Khởi động

```bash
docker compose up -d --build
```

Hoặc khỏi build, dùng image đã phát hành (xem các bản phát hành trên GitHub của dự án):

```bash
echo 'AUTOXIDE_IMAGE=ghcr.io/<owner>/autoxide:0.1' >> .env
docker compose up -d --no-build
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
`rust-toolchain.toml`), rồi:

```bash
cargo build --release --locked -p autoxide
sudo install -m 755 target/release/autoxide /usr/local/bin/autoxide
```

### 2. Chuẩn bị thư mục

```bash
sudo useradd --system --home-dir /var/lib/autoxide --create-home autoxide
sudo -u autoxide sh -c 'cd /var/lib/autoxide && printf "AUTOXIDE_AUTH_TOKEN_SECRET=%s\n" "$(openssl rand -base64 48 | tr -d "\n")" > .env && chmod 600 .env'
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
| `AUTOXIDE_HTTP_CLIENT_IP_HEADER` | Sau reverse proxy: header chứa địa chỉ client |
| `AUTOXIDE_HTTP_CORS_ORIGINS` | Trang web ở origin khác được gọi API |
| `AUTOXIDE_MAIL_SMTP_URL`, `AUTOXIDE_MAIL_FROM` | Gửi mail (đặt lại mật khẩu, xác minh email) |
| `AUTOXIDE_FILES_S3_BUCKET` và `AWS_…` | Lưu file trên S3 hay dịch vụ tương thích S3 |
| `AUTOXIDE_AUTH_PASSWORD_RESET_URL`, `AUTOXIDE_AUTH_EMAIL_VERIFICATION_URL` | Mail dẫn tới trang của bạn |
| `AUTOXIDE_AUTH_PASSKEY_RP_ID`, `AUTOXIDE_AUTH_PASSKEY_ORIGINS` | Bật đăng nhập bằng passkey cho tên miền của bạn |
| `AUTOXIDE_PROCESS_ROLE` | `api` / `worker` để chạy API và việc nền ở tiến trình riêng |
| `AUTOXIDE_HTTP_METRICS` | Metrics Prometheus ở `/metrics` |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Gửi trace tới một OpenTelemetry collector |

Kiểm cấu hình mà không khởi động gì: `autoxide config`. Lệnh liệt kê mọi biến kèm giá
trị (secret được giấu) và báo mọi vấn đề cùng một lượt.

## Chạy nhiều máy chủ

Với PostgreSQL, chạy bao nhiêu tiến trình `app` cũng được sau reverse proxy: chúng chỉ
dùng chung database (và S3 cho file), và job nền được nhận sao cho mỗi job chạy đúng một
lần. Đặt cùng `AUTOXIDE_AUTH_TOKEN_SECRET` ở mọi nơi. Muốn tăng API và việc nền riêng
rẽ, chạy vài tiến trình với `AUTOXIDE_PROCESS_ROLE=api` và ít nhất một tiến trình
`worker`. SQLite chỉ cho một tiến trình máy chủ.

## Kiểm trước khi chạy thật

- [ ] `AUTOXIDE_AUTH_TOKEN_SECRET` là một chuỗi ngẫu nhiên dài, giữ bí mật
- [ ] Có TLS phía trước (reverse proxy), port 8080 không mở ra internet
- [ ] `AUTOXIDE_HTTP_CLIENT_IP_HEADER` đặt theo header của proxy
- [ ] Đã cấu hình SMTP, để đặt lại mật khẩu và xác minh email chạy được
- [ ] Backup theo lịch và chép sang máy khác; đã thử khôi phục một lần
- [ ] `/health/ready` được hệ thống giám sát theo dõi
