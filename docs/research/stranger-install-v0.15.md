# Thử cài như người lạ — v0.15.0 (bước 50)

Người thử đóng vai người lạ lần đầu gặp Autoxide: chỉ dùng README (EN + VI), `docs/`,
`.env.example`, trang Release v0.15.0 và image `ghcr.io/thuongtamduy/autoxide:0.15.0`. Không đọc
mã nguồn. Mọi secret là giá trị tự sinh dùng một lần. Máy thử: Linux, 4 CPU, 15 GB RAM, Docker 29.

Mức độ: **chặn** (người lạ không đi tiếp được nếu không đoán), **khó chịu** (đi tiếp được nhưng
mất thời gian hoặc có nguy cơ mất dữ liệu), **nhỏ**.

## Kết quả tóm tắt

| Đường | Kết quả |
| --- | --- |
| Image + SQLite (`docker run`) | Chạy được, **sau khi tự đoán** thứ tự `migrate up` → `user create` → `serve`: README không có đường này (vấp 1) |
| Image + PostgreSQL (Compose, theo `docs/DEPLOY.md` mục A) | Chạy đúng từng bước, không vấp; `postgres:18-alpine` kéo được, không gặp 429 |
| Từ mã nguồn (`cargo run`) | Xem mục cuối |
| Đăng nhập, `/auth/me`, API key, MCP `tools/list`, `Idempotency-Key`, lỗi tiếng Việt | Đúng như docs, cả hai database |
| Backup + restore | Chạy đúng trên cả hai database (có hai bẫy: vấp 3, 7) |

Số đo (SQLite, image 0.15.0): kéo image 6 giây (mạng tốt), **627 MB** trên đĩa (binary 377 MB);
`serve` sẵn sàng sau **1,25 giây**; RAM nghỉ khoảng **200 MB**; `migrate up` 3 giây (407 migration);
Compose với PostgreSQL từ lúc `up` tới khi healthy: 17 giây (gồm kéo `postgres:18-alpine`). Số bảng
462, `openapi.json` 4,2 MB.

## Từng bước đã làm

1. **Đọc README.** Dòng đầu nói Autoxide là gì (rõ), nhưng phần "Getting started" chỉ có `git clone
   <repository-url>` + `cargo run` + `./gate.sh`: dành cho người phát triển. Không có một dòng nào
   về chạy image, dù trang Release chỉ nêu tên image. → vấp 1.
2. **`docker pull ghcr.io/thuongtamduy/autoxide:0.15.0`**: được, không cần đăng nhập.
3. **`docker run … serve` (đoán, vì README không nói)**: thoát ngay với một dòng lỗi dài khoảng 4 KB
   liệt kê 407 migration. Cuối dòng có `run autoxide migrate up first`, nhưng phải kéo hết dòng
   mới thấy. → vấp 2.
4. **`migrate up`, `user create --admin`** (volume `autoxide-data`): được. Mật khẩu in ở dòng cuối,
   sau mấy dòng log JSON và hai cảnh báo thiếu secret. → vấp 6.
5. **`doctor`**: rất hữu ích; nói rõ còn thiếu gì và sửa thế nào. Báo ba thứ chưa nhập
   (`address.import`, `tax.import_pack`, `promotions.load_legal_data`). README nhắc hai thứ đầu,
   không nhắc thứ ba. → vấp 5 (đã sửa).
6. **Chạy ba lệnh `task …`**: được. `address.import` không cần mạng ra ngoài (danh mục đi kèm).
7. **`serve` với `AUTOXIDE_AUTH_TOKEN_SECRET` và `AUTOXIDE_SECRET_KEY` tự sinh**: sẵn sàng sau 1,25 s.
   `/health/live`, `/health/ready`, `/api/v1/openapi.json`, `/llms.txt`, `/mcp`: đều đúng.
8. **Đăng nhập** `POST /api/v1/auth/login` → `access_token`; `GET /auth/me` đúng; sai mật khẩu → 401
   `auth.invalid_credentials`; `Accept-Language: vi` → thông báo tiếng Việt.
9. **Tạo API key**: lần đầu tôi gửi `password` thay vì `current_password` và nhận 422 chỉ đúng trường
   (`current_password: must not be empty`) — thông báo tốt. Có key rồi: gọi `/auth/users` được;
   `/mcp` không có token → 401 `www-authenticate: Bearer`; có key → `tools/list` trả danh sách tool.
10. **Ghi dữ liệu**: `POST /api/v1/content/entries` (tạo bài nháp) → 201; gửi lại cùng
    `Idempotency-Key` → 201 kèm `idempotent-replayed: true`. `GET /api/v1/notes` → 404: module mẫu
    `notes` không đăng ký mặc định (README có nói "not registered by default" ở mục tài liệu) nên
    người lạ không có "ghi chú mẫu" để thử; bài nháp `content` là ví dụ ghi đơn giản nhất.
11. **Backup** bằng `docker run … backup backups/t1` (đúng như README, chỉ đổi lệnh): in
    "Backed up 3541 rows … (backups/t1)" nhưng thư mục nằm **trong container**, mất khi container
    bị xoá. → vấp 3. Làm lại với thư mục mount: lần đầu `Permission denied (os error 13)` (vì thư
    mục thuộc root, image chạy user 10001), sau `chown 10001` thì được. **Restore** vào volume mới:
    `407 applied, 0 pending`, `doctor` thấy quản trị viên và danh mục địa chỉ.
12. **Postgres bằng Compose** (`.env` theo DEPLOY A.1, `AUTOXIDE_IMAGE=…:0.15.0`,
    `docker compose up -d --no-build`): migrate chạy xong rồi app lên; `user create`, các `task`,
    `doctor` (3 cảnh báo, đều có lý do), login, tạo bài, API key, MCP: đều đúng. Backup theo DEPLOY
    A.6 (đã `chown`) và restore (`down -v` → `up -d postgres` → `restore --no-deps` → `up -d`):
    đăng nhập lại được bằng mật khẩu cũ. → vấp 7.
13. **Thử cấu hình sai**: `DATABASE_URL=mysql://…` → "must start with `sqlite:` or `postgres://`";
    nhiều biến sai cùng lúc → liệt kê một lần, kèm lệnh sinh secret; thiếu `.env` khi `compose up` →
    "set POSTGRES_PASSWORD in .env"; mật khẩu `--password-stdin` ngắn → "between 12 and 128".
    Tất cả rõ ràng.
14. **Kiểm EN và VI**: README, DEPLOY có cùng số tiêu đề và số khối code; các biến và lệnh trong
    README khớp nhau từng cái một. Không thấy lệch nội dung (DEPLOY.vi ngắn hơn 38 dòng, do cách
    ngắt dòng). Ghi chú: hai bản đều có cùng lỗ ở vấp 1.

## Chỗ vấp

| # | Mức | Chỗ vấp | Đã sửa? | Đề xuất |
| --- | --- | --- | --- | --- |
| 1 | **chặn** | README không có cách chạy image đã đăng; "Getting started" chỉ cho người phát triển (`cargo`, `gate.sh`). `docker run` chỉ có trong comment của `Dockerfile`. Người lạ với SQLite phải tự đoán thứ tự `migrate up` → `user create` → `serve`, `-v` volume, secret | **Có** (README mục "Try it with Docker", DEPLOY mục C, EN + VI; đã chạy thử đúng từng dòng) | — |
| 2 | khó chịu | `serve` / `user create` trên database chưa migrate in một dòng lỗi ~4 KB liệt kê 407 tên migration; lệnh sửa nằm ở cuối | Không (sửa code) | In "407 migrations have not been applied (first: kernel/m0001…); run `autoxide migrate up`" và chỉ liệt kê đầy đủ khi `--verbose` |
| 3 | **khó chịu (mất dữ liệu)** | `backup backups/x` trong `docker run` báo thành công nhưng ghi vào container; mất khi container bị xoá. Thông báo in đường dẫn tương đối nên không lộ ra chuyện đó | Một phần: docs nói rõ (README, DEPLOY C) | Code: in đường dẫn tuyệt đối đầy đủ, và nếu thư mục đích không nằm trên volume/mount (so `st_dev` với `/`) thì cảnh báo ở stderr |
| 4 | khó chịu | Image 627 MB: binary 377 MB. `[profile.release]` trong `Cargo.toml` chỉ có `overflow-checks`, không `strip`, không `lto`, không `codegen-units` | Không (sửa build) | Đo `strip = "symbols"` (thường giảm phần lớn), thử `lto = "thin"`; cân nhắc tách thông tin gỡ lỗi. Ghi số đo trước/sau vào quyết định |
| 5 | khó chịu | `doctor` cảnh báo thiếu `promotions.load_legal_data` nhưng README chỉ nhắc `address.import` và `tax.import_pack` | **Có** (README EN + VI) | Cân nhắc một lệnh gộp, vd `autoxide task --all-reference-data`, hoặc để `serve` tự nhập như thuế |
| 6 | nhỏ | Image mặc định `AUTOXIDE_LOG_FORMAT=json`: người thử đọc log JSON khó; mọi lệnh CLI (`migrate`, `user create`…) in log JSON và 2 cảnh báo thiếu secret trước kết quả thật, mật khẩu admin nằm lẫn cuối | Không | Lệnh CLI chạy tay nên log `text` mặc định, hoặc đưa log CLI xuống mức WARN; để kết quả lên đầu |
| 7 | nhỏ | DEPLOY A.6 restore: `docker compose up -d postgres` không chờ Postgres sẵn sàng, `restore` ngay sau đó có thể lỗi kết nối | **Có** (`up -d --wait postgres`, đã chạy thử) | — |
| 8 | nhỏ | `docs/DEPLOY.md` nói SBOM "attached to each GitHub release as `sbom.spdx.json`"; Release v0.15.0 **không có asset nào** (`assets: []`). Trang Release chỉ có tên image + highlights, không có lệnh chạy | Không (sửa workflow) | Sửa `release.yml` để đính SBOM lại sau khi cập nhật trang Release, hoặc sửa DEPLOY bỏ câu đó; thêm vào ghi chú Release một dòng trỏ README mục Docker |
| 9 | nhỏ | `gh attestation verify` (DEPLOY A.2): người lạ cần `gh auth login` trước; docs không nói. **Không kiểm được trong phiên này** (môi trường chặn API orgs) | Không | Thêm "cần `gh auth login`" vào DEPLOY; kiểm lại trên máy thật |
| 10 | nhỏ | Không có trang xem API cho người: `/docs`, `/api/v1/docs`, `/scalar` đều 404; chỉ có `openapi.json` 4,2 MB. README không nói mở nó bằng gì | Không | Một dòng trong README ("mở bằng Swagger Editor / Scalar / Insomnia"), hoặc phục vụ một trang xem tài liệu sinh tự động (luật của dự án cho phép) |
| 11 | nhỏ | README bản `git clone <repository-url>` (cả README, CONTRIBUTING, DEPLOY) là chỗ giữ chỗ, người lạ không biết điền gì | **Có** (URL repo công khai, EN + VI, 4 file) | — |
| 12 | nhỏ | README "Project layout" liệt kê 6 module trong khi `modules/` có 33; đoạn "Status" dài ~40 dòng một khối, vẫn nói "Business modules come next" trong khi danh sách bên dưới đã có module nghiệp vụ | Một phần: sửa dòng `modules/<name>` (EN + VI); "Status" chưa viết lại | Viết lại "Status" thành 5–6 dòng + trỏ tới ROADMAP và CHANGELOG |
| 13 | nhỏ | Tạo API key cần trường `current_password`; README chỉ nói "asks for the user's current password", không nêu tên trường (guide `api-keys.md` thì đúng) | Không | Trỏ README tới guide `api-keys.md` ở câu đó |
| 14 | nhỏ | `docker compose run …` in thêm dòng `Container … Created/Started` lẫn vào đầu ra (stderr của Compose); với `user create` làm mật khẩu khó thấy | Không | Docs dùng `docker compose run --rm -T … 2>/dev/null` hoặc `--quiet-pull`; hoặc in mật khẩu giữa hai dòng trống |
| 15 | nhỏ | Mặc định `docker run -p 8080:8080` (như comment `Dockerfile`) mở cổng ra mọi giao diện mạng; image nghe `0.0.0.0` trong container | **Có** (docs dùng `127.0.0.1:8080:8080`, DEPLOY C giải thích) | Sửa comment trong `Dockerfile` cho khớp |

## Mặc định có an toàn không?

- Không đặt `AUTOXIDE_AUTH_TOKEN_SECRET`: server sinh khoá tạm và cảnh báo (token mất hiệu lực khi
  restart). Không đặt `AUTOXIDE_SECRET_KEY`: khoá gốc nằm **trong database**, cảnh báo rõ. Cả hai
  cảnh báo đều nói đúng việc cần làm. Chấp nhận được cho dùng thử, và README/DEPLOY đã nói. `doctor`
  nhắc lại.
- Mật khẩu sinh tự động mạnh (24 ký tự); `--password-stdin` đòi 12–128 ký tự.
- Compose chỉ mở `127.0.0.1:8080`; `docker run` mà người lạ tự viết thì không (vấp 15, đã sửa docs).
- Không có tài khoản mặc định, không có secret mặc định đi kèm image.

## Từ mã nguồn

Kết quả ghi ở cuối file này (mục "Build từ mã nguồn").

## Việc đã sửa trong PR này (chỉ tài liệu)

- `README.md` + `README.vi.md`: mục mới "Try it with Docker (no build)" / "Chạy thử bằng Docker (không
  cần build)"; thêm `task promotions.load_legal_data`; "Requirements" nói rõ chỉ cần khi build từ
  mã nguồn; `git clone` có URL thật; dòng `modules/<name>` trong "Project layout".
- `docs/DEPLOY.md` + `docs/DEPLOY.vi.md`: mục C "One container with SQLite" (luật migrate, volume,
  cổng, cập nhật, backup/restore, bẫy `chown 10001`); `up -d --wait postgres` khi restore; URL repo.
- `CONTRIBUTING.md` + `CONTRIBUTING.vi.md`: URL repo thay chỗ giữ chỗ.

## Cần sửa code (không làm trong PR này)

Vấp 2 (thông báo migration quá dài), 3 (cảnh báo backup ra ngoài volume), 4 (cỡ binary/image), 6 (log
của lệnh CLI), 8 (SBOM trên trang Release), 10 (trang xem API), 14 (đầu ra Compose).

## Build từ mã nguồn

Đang đo (`cargo build -p autoxide`, bản dev, 4 CPU, toolchain tự tải lần đầu); kết quả sẽ bổ sung
sau khi build xong. README chỉ dẫn `cargo run -p autoxide -- <lệnh>` nên lần chạy đầu phải đợi biên
dịch toàn bộ workspace: README chưa nói việc này mất bao lâu.
