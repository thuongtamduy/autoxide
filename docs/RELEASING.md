# Phát hành một phiên bản

Quy trình cho agent và chủ dự án (D-078). **Agent tự phát hành khi xong trọn một bước của
`docs/ROADMAP.md`** — mọi mục của bước là `[x]`, gate xanh — không hỏi (D-171). Ngoài lúc đó
chỉ phát hành khi chủ dự án giao. Không phát hành giữa bước, không phát hành khi mục
`[Unreleased]` của `CHANGELOG.md` trống.

Cần quyền đẩy lên GitHub: `gh` đã đăng nhập, hoặc biến môi trường `GH_TOKEN` (token
fine-grained chỉ cho repo này, quyền Contents và Pull requests: Read and write). Thiếu quyền
thì dừng và báo chủ dự án, không tìm đường vòng.

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

1. Đưa nhánh lên `main` mới nhất: `git fetch origin`, rồi `git rebase origin/main`. Phiên
   khác có thể đã merge trong lúc mình làm — xem "Khi hai phiên đụng nhau" bên dưới.
2. Đổi `version` trong `Cargo.toml` gốc; chạy `cargo check` để cập nhật `Cargo.lock`. Đổi
   theo: `"version"` trong `docs/openapi.json`, bản phát hành mới nhất ở dòng "Trạng thái"
   của `docs/ROADMAP.md`.
3. Trong `CHANGELOG.md`: đổi `## [Unreleased]` thành `## [X.Y.Z] - YYYY-MM-DD`, thêm một
   mục `## [Unreleased]` trống ở trên, cập nhật hai dòng liên kết ở cuối file.
4. Commit `release: vX.Y.Z`, chạy gate (`./gate.sh`, cả SQLite lẫn Postgres) trên bản này.
5. Đẩy nhánh, mở PR vào `main`, merge kiểu rebase để commit `release: vX.Y.Z` nằm nguyên
   trên `main`:

   ```bash
   git push -u origin <nhánh>
   gh pr create --base main --title "…" --body "…"
   gh pr merge <số PR> --rebase --delete-branch
   ```

6. Gắn tag lên **đúng commit `release: vX.Y.Z`** của `main` — không phải commit mới nhất: người
   khác có thể vừa merge thêm sau commit release (v0.11.0 từng bị gắn nhầm như vậy). Kiểm commit
   tìm được đúng là commit release rồi mới đẩy:

   ```bash
   git fetch origin && git switch main && git merge --ff-only origin/main
   release=$(git log origin/main --format=%H --grep='^release: vX.Y.Z$' -n 1)
   git log -1 --format='%h %s' "$release"        # phải in ra "… release: vX.Y.Z"
   git tag -a vX.Y.Z -m "Autoxide vX.Y.Z" "$release"
   git push origin vX.Y.Z
   ```

7. Tạo bản phát hành trên GitHub, ghi chú là phần `## [X.Y.Z]` của `CHANGELOG.md`:

   ```bash
   awk '/^## \[X.Y.Z\]/{f=1;next} /^## \[/{f=0} f' CHANGELOG.md > notes.md
   gh release create vX.Y.Z --title vX.Y.Z --notes-file notes.md --verify-tag
   ```

8. `cargo clean`: mỗi lần đổi phiên bản, cargo build lại cả workspace thành một bộ file mới
   mà không xoá bộ cũ, `target/` phình thêm hàng chục GB.
9. Image Docker (khi chủ dự án muốn đăng): build trên máy rồi đẩy lên GitHub Container
   Registry. Cần một token GitHub có quyền `write:packages` của chủ dự án — thiếu thì bỏ qua
   bước này và báo lại, không tự tạo hay lưu token:

   ```bash
   docker build -t ghcr.io/<owner>/autoxide:X.Y.Z -t ghcr.io/<owner>/autoxide:X.Y .
   echo "$TOKEN" | docker login ghcr.io -u <owner> --password-stdin
   docker push ghcr.io/<owner>/autoxide:X.Y.Z
   docker push ghcr.io/<owner>/autoxide:X.Y
   ```

   Lần đầu đăng image, gói mặc định là riêng tư: chủ repo vào trang của gói (Packages →
   autoxide → Package settings) đổi thành Public để người lạ tải được.

Workflow `Release` (`.github/workflows/release.yml`) vẫn còn trong repo nhưng chỉ chạy khi
bấm tay; bật lại khi có runner dùng được thì các bước 7 và 9 lại tự động.

## Khi hai phiên đụng nhau

Nhiều phiên agent có thể làm song song trên các nhánh khác nhau. Lúc rebase ở bước 1:

- **Số quyết định trùng** (`D-NNN` của mình đã có trên `main` với nội dung khác): đổi số các
  mục của mình thành số kế tiếp sau mục cuối của `main`, sửa theo mọi chỗ nhắc tới số đó
  (code, tài liệu, thông điệp commit). Mục đã lên `main` thì không đổi số.
- **`CHANGELOG.md`**: dòng mới của mình phải nằm trong `## [Unreleased]`. Git hay tự đặt chúng
  vào mục của bản vừa phát hành — kiểm lại bằng mắt, mục đã phát hành không được thêm dòng.
- **Tag đã có** (`git ls-remote --tags origin vX.Y.Z` trả về kết quả): phiên khác vừa phát
  hành số đó. Rebase lại lên `main`, lấy số kế tiếp, làm lại từ bước 2.

## Nâng cấp thư viện

Dependabot (`.github/dependabot.yml`) mở pull request mỗi tuần, gom bản minor/patch của
thư viện Rust thành một PR. CI không chạy trên PR (D-107): người duyệt hoặc agent lấy nhánh
của PR về, chạy `./gate.sh`, xanh mới merge. Bản lớn (major) đến từng PR riêng: đọc
CHANGELOG của thư viện (skill `check-library-api`) trước khi merge. Lỗ hổng thư viện được
`cargo deny check` trong gate bắt; đỏ thì nâng thư viện hoặc ghi ngoại lệ có lý do trong
`deny.toml` và `docs/DECISIONS.md`.
