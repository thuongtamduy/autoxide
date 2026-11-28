# Bắt đầu

Ba cách thử Autoxide, từ ít việc nhất đến nhiều việc nhất. Cả ba cho cùng một máy chủ: REST API, tài liệu
OpenAPI, endpoint MCP cho AI agent và một tài khoản quản trị.

## 1. Demo (khoảng hai phút)

`autoxide demo` là máy chủ dùng xong bỏ để xem thử, không cần cấu hình gì. Lần đầu nó tạo thư mục
`autoxide-demo` (đổi bằng `--dir`) với database SQLite, secret ngẫu nhiên, quản trị viên
`admin@example.com` và dữ liệu tham chiếu (địa chỉ, thuế, giới hạn khuyến mãi), rồi in mật khẩu ra màn
hình. Chạy lại thì tiếp tục với dữ liệu cũ. Dừng bằng Ctrl+C.

Bằng Docker:

```bash
docker run --rm -it -p 127.0.0.1:8080:8080 ghcr.io/thuongtamduy/autoxide:latest demo
```

Container dừng là dữ liệu mất. Muốn giữ, thêm một volume và đặt thư mục demo vào đó:

```bash
docker run --rm -it -p 127.0.0.1:8080:8080 -v autoxide-demo:/var/lib/autoxide/data \
  ghcr.io/thuongtamduy/autoxide:latest demo --dir data
```

Bằng binary tải về: lấy file đúng hệ điều hành ở [trang phát hành](https://github.com/thuongtamduy/autoxide/releases/latest),
giải nén và chạy `./autoxide demo` (`autoxide.exe demo` trên Windows). File phát hành có checksum và chứng
thực nguồn gốc build: [`VERIFYING.vi.md`](VERIFYING.vi.md). File macOS và Windows chưa ký bằng chứng chỉ của
Apple hay Microsoft, nên hệ điều hành có thể cảnh báo lần đầu:

- macOS: `xattr -d com.apple.quarantine ./autoxide`, rồi chạy lại.
- Windows: SmartScreen, "More info", "Run anyway".

Rồi mở `http://127.0.0.1:8080/api/docs` xem tài liệu API và đăng nhập bằng tài khoản quản trị đã in
(`POST /api/v1/auth/login`).

### Các request đầu tiên

Đăng nhập, giữ token, tạo một bài viết rồi liệt kê lại (`jq` chỉ để lấy token ra; `--json` cần curl 7.82 trở lên).
Dùng mật khẩu demo đã in:

```bash
API=http://127.0.0.1:8080/api/v1
TOKEN=$(curl -sS $API/auth/login --json '{"email":"admin@example.com","password":"<mật khẩu đã in>"}' | jq -r .data.access_token)

curl -sS $API/content/entries -H "Authorization: Bearer $TOKEN" --json '{
  "kind": "post",
  "title": "Hello from Autoxide",
  "body": {"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"My first post."}]}]}
}'

curl -sS "$API/content/entries?kind=post" -H "Authorization: Bearer $TOKEN"
```

Bài mới là bản nháp (`status: draft`) và hiện trong danh sách. Mọi endpoint, kèm trường và permission cần có, nằm ở
trang tra cứu API `/api/docs`. Module mẫu `notes` (ví dụ dạy cho người viết module) không có trong file phát hành hay
image Docker: module cho website ở đây là `content`.

### Khi cổng đang bận

Demo kiểm cổng trước khi in bất cứ gì. Nếu `127.0.0.1:8080` đã bị chiếm, nó dừng với lỗi nêu cách sửa:
`AUTOXIDE_HTTP_ADDRESS=127.0.0.1:8081 ./autoxide demo` (tên biến đúng là vậy; không có `AUTOXIDE_PORT`). Với Docker thì
publish cổng khác của máy chủ: `-p 127.0.0.1:8081:8080`.

Demo luôn dùng file SQLite riêng trong thư mục demo: nó bỏ qua `DATABASE_URL` trong môi trường và `.env`, nên không thể chạm vào database thật. `--database-url <url>` trỏ nó tới database khác khi bạn chủ ý.

Demo cũng tắt các dịch vụ bên ngoài lấy từ môi trường (mail SMTP, kho file S3, OpenTelemetry, nhà cung cấp AI, module bộ nối): thư được ghi ra file và file tải lên nằm trong thư mục demo. `--keep-external-services` giữ chúng khi bạn chủ ý.

Không dùng cho production: secret nằm trong một file thường ở thư mục demo, và trang tài liệu API nạp
script xem từ một CDN công cộng.

## 2. Quickstart (bản cài một máy chủ thật)

Docker Compose, volume cho dữ liệu, secret của riêng bạn: xem mục Chạy nhanh trong
[README](../README.vi.md#chạy-nhanh). PostgreSQL, nhiều máy chủ và HTTPS: [`DEPLOY.vi.md`](DEPLOY.vi.md).

## 3. Phát triển chính Autoxide

Mở repo trong Codespace hay dev container (đã có `.devcontainer/`), hoặc kiểm máy của bạn còn thiếu gì:

```bash
cargo xtask doctor      # chỉ đọc: liệt kê công cụ cần có, kèm một lệnh cài cho từng thứ
./gate.sh --quick       # "xong" nghĩa là gì trước mỗi commit
```

Khi `cargo xtask check` báo vi phạm, nó in thêm một dòng `how to fix:` dưới đó. Luật cho người đóng góp và
AI agent: [`AGENTS.md`](../AGENTS.md) và [`CONTRIBUTING.vi.md`](../CONTRIBUTING.vi.md).
