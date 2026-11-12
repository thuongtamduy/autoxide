# Phát hành một phiên bản

Quy trình cho agent và chủ dự án (D-078). Phát hành là việc ra bên ngoài máy này (đẩy
tag lên GitHub, đăng image): agent làm khi chủ dự án đã giao, như bước 13.

## Đánh số phiên bản

Theo Semantic Versioning, một số cho cả workspace (`[workspace.package] version` trong
`Cargo.toml`, mọi crate dùng chung):

- `0.x.y` cho tới khi chủ dự án tuyên bố API ổn định (`1.0.0`).
- Sửa lỗi, không đổi hành vi: tăng `y` (`0.1.0` → `0.1.1`).
- Thêm tính năng, hoặc (khi còn `0.x`) thay đổi phá vỡ ở cấu hình, CLI hay hợp đồng
  module: tăng `x` (`0.1.1` → `0.2.0`) và ghi rõ ở mục **Changed** / **Removed**.
- API HTTP `/api/v1` không bao giờ phá vỡ: endpoint thay thế đi qua deprecation (D-059).

## Khi làm việc hằng ngày

Mỗi commit đổi hành vi mà người dùng thấy (API, cấu hình, CLI, migration, Docker) thêm một
dòng tiếng Anh vào mục `## [Unreleased]` của `CHANGELOG.md`, dưới `Added`, `Changed`,
`Deprecated`, `Removed`, `Fixed` hoặc `Security`.

## Các bước phát hành

CI trên GitHub đã tắt (D-107): mọi bước chạy trên máy của người phát hành.

1. Gate xanh trên `main` (`./gate.sh`, cả SQLite lẫn Postgres).
2. Đổi `version` trong `Cargo.toml` gốc; chạy `cargo check` để cập nhật `Cargo.lock`.
3. Trong `CHANGELOG.md`: đổi `## [Unreleased]` thành `## [X.Y.Z] - YYYY-MM-DD`, thêm một
   mục `## [Unreleased]` trống ở trên, cập nhật hai dòng liên kết ở cuối file.
4. Chạy gate, commit `release: vX.Y.Z`, merge vào `main`.
5. Gắn tag trên `main` và đẩy lên:

   ```bash
   git tag -a vX.Y.Z -m "Autoxide vX.Y.Z"
   git push origin vX.Y.Z
   ```

6. Tạo bản phát hành trên GitHub (Releases → Draft a new release → chọn tag), dán phần
   `## [X.Y.Z]` của `CHANGELOG.md` làm ghi chú.
7. Image Docker (khi chủ dự án muốn đăng): build trên máy rồi đẩy lên GitHub Container
   Registry. Cần một token GitHub có quyền `write:packages` của chủ dự án — agent dừng lại
   hỏi, không tự tạo hay lưu token:

   ```bash
   docker build -t ghcr.io/<owner>/autoxide:X.Y.Z -t ghcr.io/<owner>/autoxide:X.Y .
   echo "$TOKEN" | docker login ghcr.io -u <owner> --password-stdin
   docker push ghcr.io/<owner>/autoxide:X.Y.Z
   docker push ghcr.io/<owner>/autoxide:X.Y
   ```

   Lần đầu đăng image, gói mặc định là riêng tư: chủ repo vào trang của gói (Packages →
   autoxide → Package settings) đổi thành Public để người lạ tải được.

Workflow `Release` (`.github/workflows/release.yml`) vẫn còn trong repo nhưng chỉ chạy khi
bấm tay; bật lại khi có runner dùng được thì các bước 6–7 lại tự động.

## Nâng cấp thư viện

Dependabot (`.github/dependabot.yml`) mở pull request mỗi tuần, gom bản minor/patch của
thư viện Rust thành một PR. CI không chạy trên PR (D-107): người duyệt hoặc agent lấy nhánh
của PR về, chạy `./gate.sh`, xanh mới merge. Bản lớn (major) đến từng PR riêng: đọc
CHANGELOG của thư viện (skill `check-library-api`) trước khi merge. Lỗ hổng thư viện được
`cargo deny check` trong gate bắt; đỏ thì nâng thư viện hoặc ghi ngoại lệ có lý do trong
`deny.toml` và `docs/DECISIONS.md`.
