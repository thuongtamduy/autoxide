# Phát hành một phiên bản

Quy trình cho agent và chủ dự án (D-078). **Phát hành gom nhiều bước thành một bản lớn**
(D-248, thay D-171): xong một bước thì chỉ merge vào `main` (gate nhanh xanh, PR merge kiểu
rebase), không phát hành. Gate đầy đủ (SQLite và Postgres) chỉ bắt buộc ở bước phát hành bên dưới
(D-300): đỏ thì sửa trên nhánh làm việc, merge bản sửa, rồi mới phát hành. Agent tự phát hành, không hỏi, khi xong trọn một **giai đoạn** của
`docs/ROADMAP.md` (A, B, C…: mọi bước của giai đoạn đã `[x]`). Ngoài lúc đó chỉ phát hành khi chủ
dự án giao. Không phát hành giữa bước, không phát hành khi mục `[Unreleased]` của `CHANGELOG.md`
trống sau khi gộp mảnh (bước 3).

Cần quyền đẩy lên GitHub: `gh` đã đăng nhập, hoặc biến môi trường `GH_TOKEN` (token
fine-grained chỉ cho repo này, quyền Contents và Pull requests: Read and write). Thiếu quyền
thì dừng và báo chủ dự án, không tìm đường vòng. Image Docker không cần token cá nhân nào:
workflow `Release` đăng bằng `GITHUB_TOKEN` của chính nó.

## Đánh số phiên bản

Theo Semantic Versioning, một số cho cả workspace (`[workspace.package] version` trong
`Cargo.toml`, mọi crate dùng chung):

- `0.x.y` cho tới khi chủ dự án tuyên bố API ổn định (`1.0.0`).
- Sửa lỗi, không đổi hành vi: tăng `y` (`0.1.0` → `0.1.1`).
- Thêm tính năng, hoặc (khi còn `0.x`) thay đổi phá vỡ ở cấu hình, CLI hay hợp đồng
  module: tăng `x` (`0.1.1` → `0.2.0`) và ghi rõ ở mục **Changed** / **Removed**.
- API HTTP `/api/v1` không bao giờ phá vỡ: endpoint thay thế đi qua deprecation (D-059).

## Khi làm việc hằng ngày

Mỗi thay đổi hành vi mà người dùng thấy (API, cấu hình, CLI, migration, Docker) thêm một **file
mảnh** vào `changelog.d/`, **không sửa `CHANGELOG.md`** (D-20261119-changelog-fragments): tên
`<tên>.<mục>.md` — `<tên>` chữ thường, số, `-` (vd `step45-checkout`); `<mục>` là `added`, `changed`,
`deprecated`, `removed`, `fixed` hoặc `security` — nội dung là các gạch đầu dòng tiếng Anh đúng như sẽ
nằm trong changelog. `cargo xtask check` (trong gate) kiểm tên và nội dung. Nhiều phiên song song
không còn xung đột ở `CHANGELOG.md`; xem `changelog.d/README.md`.

## Các bước phát hành

Agent chỉ chuẩn bị bản phát hành và đẩy tag; workflow `Release` (`.github/workflows/release.yml`,
D-175, D-197) chạy lại gate (chia nhiều máy như CI, chung `gate.yml`, D-20261120-release-gate-sharded), build và đăng image Docker (`linux/amd64` và `linux/arm64`) lên
`ghcr.io/<owner>/autoxide:X.Y.Z` và `:X.Y` bằng `GITHUB_TOKEN`, đính kèm chứng thực nguồn gốc build
và SBOM đã ký, rồi tạo bản phát hành GitHub với ghi chú từ `CHANGELOG.md`. Workflow chỉ chạy khi
repo công khai; `./gate.sh` trên máy vẫn là gate bắt buộc trước khi đẩy tag. **Repo đang riêng
tư (D-199): workflow bị bỏ qua, làm bước 1–6 rồi theo "Khi repo còn riêng tư" bên dưới thay cho
bước 7.**

1. Đưa nhánh lên `main` mới nhất: `git fetch origin`, rồi `git rebase origin/main`. Phiên
   khác có thể đã merge trong lúc mình làm — xem "Khi hai phiên đụng nhau" bên dưới.
2. Đổi `version` trong `Cargo.toml` gốc; chạy `cargo check` để cập nhật `Cargo.lock`. Đổi
   theo: `"version"` trong `docs/openapi.json`, `"version"` trong `sdk/typescript/package.json` và
   `sdk/typescript/package-lock.json`, `artifactVersion` / `pubVersion` trong `sdk/openapitools.json`
   (SDK Kotlin, Dart), rồi `cargo xtask sdk` (chép hợp đồng cho SDK Swift) — `cargo xtask sdk --check`
   của gate kiểm cả mấy chỗ này; bản phát hành mới
   nhất ở dòng "Trạng thái" của `docs/ROADMAP.md`.
3. Chạy `cargo xtask changelog`: gộp mọi mảnh trong `changelog.d/` vào `## [Unreleased]` (mỗi mảnh vào
   cuối mục của nó) và xoá các mảnh. Rồi trong `CHANGELOG.md`: đổi `## [Unreleased]` thành `## [X.Y.Z] - YYYY-MM-DD`, thêm một
   mục `## [Unreleased]` trống ở trên, cập nhật hai dòng liên kết ở cuối file. Workflow kiểm
   có mục `## [X.Y.Z]` và tag khớp `version`, sai thì không phát hành.
   Rồi chạy `cargo xtask semver --markdown` (cần `cargo-semver-checks`, phiên bản ghim ở `STACK.md`;
   mất vài phút và vài GB đĩa — `rm -rf target/semver-checks` sau đó): nó in mục `### Breaking (Rust)`
   gồm thay đổi phá vỡ của `autoxide-kernel` và `autoxide-testkit` so với tag trước; chép vào mục
   `## [X.Y.Z]`. Không in gì thì không thêm mục. Còn `0.x` thì chỉ báo cáo, không chặn (D-167, D-242).
4. Commit `release: vX.Y.Z`, chạy gate (`./gate.sh`, cả SQLite lẫn Postgres) trên bản này. Chạy CI bằng tay trên
   nhánh release (Actions → CI → Run workflow) để build ba SDK di động Swift, Kotlin, Dart — PR và push thường không
   build chúng (D-20261121-mobile-sdk-on-demand); đỏ thì sửa trước khi phát hành. Rồi
   chạy test nâng cấp: `cargo xtask upgrade-test` — bản phát hành trước (image Docker của nó; tải
   không được thì build từ tag, lâu) ghi dữ liệu, bản này đọc lại trên cùng database, cả SQLite lẫn
   Postgres (D-246). Cần Docker; đỏ thì không phát hành. Job `upgrade` của CI chạy cùng lệnh.
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

   Môi trường của agent không đẩy được tag (chỉ được đẩy nhánh làm việc) thì nhờ chủ dự án tạo trên
   GitHub: Releases → Draft a new release → tag `vX.Y.Z` (Create new tag), **Target** là commit
   release ở trên (hay commit merge của nó, cùng nội dung) → Publish. Workflow `Release` chạy theo tag
   đó; thấy trang Release đã có thì cập nhật ghi chú và đính kèm file chứ không báo lỗi. Ghi chú quá
   giới hạn của GitHub (125 000 ký tự) thì workflow thay bằng đoạn ngắn dẫn tới `CHANGELOG.md`.

7. Theo dõi workflow `Release` của tag (Actions → Release) tới khi xanh, rồi kiểm image đã đăng:

   ```bash
   gh attestation verify oci://ghcr.io/<owner>/autoxide:X.Y.Z --owner <owner>
   docker run --rm ghcr.io/<owner>/autoxide:X.Y.Z --version
   ```

   Workflow đỏ thì sửa nguyên nhân rồi chạy lại trên cùng tag (Actions → Release → Run workflow,
   chọn tag); không xoá tag đã đẩy. Lỗi nằm ở code thì phát hành bản vá `X.Y.Z+1`.
8. `cargo clean`: mỗi lần đổi phiên bản, cargo build lại cả workspace thành một bộ file mới
   mà không xoá bộ cũ, `target/` phình thêm hàng chục GB.

## Khi repo còn riêng tư

Chủ dự án giữ repo riêng tư (D-199), nên đẩy tag không kích hoạt gì. Sau bước 6:

1. Tạo bản phát hành trên GitHub, ghi chú là phần `## [X.Y.Z]` của `CHANGELOG.md`:

   ```bash
   awk '/^## \[X.Y.Z\]/{f=1;next} /^## \[/{f=0} f' CHANGELOG.md > notes.md
   gh release create vX.Y.Z --title "Autoxide vX.Y.Z" --notes-file notes.md --verify-tag
   ```

   Không có `gh` hay quyền tạo bản phát hành thì dừng ở tag đã đẩy và báo chủ dự án tạo bản phát
   hành trên trang Releases (chọn tag, dán phần `## [X.Y.Z]` của `CHANGELOG.md`).
   Đính kèm gói SDK TypeScript: `(cd sdk/typescript && npm pack)` tạo `autoxide-sdk-X.Y.Z.tgz`;
   `gh release upload vX.Y.Z sdk/typescript/autoxide-sdk-X.Y.Z.tgz` (hoặc kéo file vào trang phát hành).
2. Image Docker chỉ khi chủ dự án muốn và đưa token GitHub có quyền `write:packages` — thiếu thì
   bỏ qua và báo lại, không tự tạo hay lưu token. Chỉ build được `linux/amd64`, không kèm chứng
   thực:

   ```bash
   docker build -t ghcr.io/<owner>/autoxide:X.Y.Z -t ghcr.io/<owner>/autoxide:X.Y .
   echo "$TOKEN" | docker login ghcr.io -u <owner> --password-stdin
   docker push ghcr.io/<owner>/autoxide:X.Y.Z
   docker push ghcr.io/<owner>/autoxide:X.Y
   ```

3. Bước 8 (`cargo clean`) như trên.

Khi chủ dự án công khai repo (bước 50 của lộ trình), bỏ phần này.

## Image cho một tag đã phát hành

Tag đã đẩy mà chưa có image trên ghcr.io (Release bị bỏ qua lúc repo riêng tư, hay chạy lại thì gate đỏ vì
thứ mới sau ngày phát hành): Actions → "Release image (existing tag)" → Run workflow trên `main`, nhập tag
(vd `v0.14.0`). Workflow build mã nguồn của tag và đăng `X.Y.Z`, `X.Y` như Release, không chạy gate, không
tạo bản phát hành (D-20261120-upgrade-image). Job `upgrade` của CI kéo image này thay vì build lại bản cũ.

Lần đầu workflow đăng image, gói trên GitHub Container Registry mặc định là riêng tư: chủ repo
vào trang của gói (Packages → autoxide → Package settings → Change visibility) đổi thành Public
một lần để người lạ tải được (GitHub không cho đổi bằng `GITHUB_TOKEN`).

## Khi hai phiên đụng nhau

Nhiều phiên agent có thể làm song song trên các nhánh khác nhau. Lúc rebase ở bước 1:

- **Số quyết định trùng** (`D-NNN` của mình đã có trên `main` với nội dung khác): đổi số các
  mục của mình thành số kế tiếp sau mục cuối của `main`, sửa theo mọi chỗ nhắc tới số đó
  (code, tài liệu, thông điệp commit). Mục đã lên `main` thì không đổi số.
- **Thay đổi cho changelog**: viết bằng file mảnh trong `changelog.d/`, nên không có gì để gộp tay.
  Mảnh nào còn nằm ở `changelog.d/` sau khi phiên khác phát hành thì sẽ vào bản sau — đúng ý.
- **Tag đã có** (`git ls-remote --tags origin vX.Y.Z` trả về kết quả): phiên khác vừa phát
  hành số đó. Rebase lại lên `main`, lấy số kế tiếp, làm lại từ bước 2.

## Nâng cấp thư viện

Dependabot (`.github/dependabot.yml`) mở pull request mỗi tuần, gom bản minor/patch của
thư viện Rust thành một PR. CI chạy gate trên PR (khi repo công khai); người duyệt hoặc agent
vẫn chạy `./gate.sh` trên máy, xanh mới merge. Bản lớn (major) đến từng PR riêng: đọc
CHANGELOG của thư viện (skill `check-library-api`) trước khi merge. Lỗ hổng thư viện được
`cargo deny check` trong gate bắt; đỏ thì nâng thư viện hoặc ghi ngoại lệ có lý do trong
`deny.toml` và `docs/DECISIONS.md`.
