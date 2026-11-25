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
- Bản thử trước một bản major: `X.0.0-rc.N` (N từ 1), xem "Bản thử trước bản major" bên dưới. Không dùng dạng
  pre-release khác (`-beta`, `-alpha.1`…): workflow `Release` từ chối.
- Sửa lỗi, không đổi hành vi: tăng `y` (`0.1.0` → `0.1.1`).
- Thêm tính năng, hoặc (khi còn `0.x`) thay đổi phá vỡ ở cấu hình, CLI hay hợp đồng
  module: tăng `x` (`0.1.1` → `0.2.0`) và ghi rõ ở mục **Changed** / **Removed**.
- API HTTP `/api/v1` không bao giờ phá vỡ: endpoint thay thế đi qua deprecation (D-059).
- **Từ `1.0.0`** theo [`docs/COMPATIBILITY.md`](COMPATIBILITY.md) (+ `.vi.md`, D-20261123-compatibility-policy):
  patch chỉ sửa lỗi; minor thêm, không phá vỡ; phá vỡ hợp đồng (REST, mã lỗi, cấu hình, migration, tool MCP,
  event, CLI, Rust API của kernel / testkit / `autoxide::Autoxide` / `api` của module core và official) chỉ ở
  bản major. Bản minor chạm hợp đồng thì kiểm theo bảng của trang đó trước khi merge.

## Kiểm hợp đồng Rust (`cargo-semver-checks`)

`cargo xtask semver` chạy `cargo semver-checks` (phiên bản ghim ở `STACK.md`) trên mọi crate thuộc hợp đồng:
`autoxide-kernel`, `autoxide-testkit`, `autoxide` (thư viện) và crate của module hạng core / official — danh sách
đọc từ bảng sinh tự động của `docs/MODULES.md`, module đổi hạng thì danh sách tự đổi. Baseline là tag git (chưa
đăng crates.io). Job `semver` của CI (`.github/workflows/ci.yml`, job riêng, không làm chậm gate):

- **PR phát hành** (đổi `version` của workspace): mọi crate hợp đồng so với tag phát hành trước; mục `### Breaking
  (Rust)` nằm ở tóm tắt của job để chép vào `CHANGELOG.md` (bước 3).
- **PR khác** chạm phần công khai (`crates/kernel/`, `crates/testkit/`, `crates/app/src/lib.rs`,
  `modules/*/src/lib.rs`, `modules/*/src/api.rs`, `modules/*/src/api/`): chỉ crate bị chạm, so với nhánh gốc
  của PR (`--base <sha> --changed <sha>`). PR sửa chính bộ kiểm (`xtask/src/semver.rs`) chạy như PR phát hành. PR
  không chạm thì job bỏ qua.
- Crate chia ra 4 máy (`semver 1/4` … `4/4`, `cargo xtask semver --shard k/4`); job **"Rust API compatibility
  (cargo-semver-checks)"** gộp kết quả thành một mục và là tên check ổn định. Mỗi máy dựng mọi crate của nó trong
  một target dir chung (`target/semver-checks`) thay vì mỗi crate một thư mục riêng như công cụ tự làm
  (D-20261125-semver-job-time). Đủ 28 crate: khoảng 23 phút thời gian chờ thực, phần chậm nhất 22 phút (đo ở
  #506); giới hạn mỗi phần 45 phút.

Chặn hay chỉ cảnh báo do `version` của workspace quyết, không cần sửa workflow:

- **Còn `0.x`**: chỉ cảnh báo (dòng `::warning` và tóm tắt của job), job vẫn xanh; so như bản patch để mọi phá vỡ đều hiện.
- **Từ `1.0.0`**: có phá vỡ thì `cargo xtask semver` thoát 1, job đỏ; so như bản minor (thêm mới và
  `#[deprecated]` là hợp lệ). Ngoại lệ duy nhất: PR phát hành tăng major (`2.0.0` so với tag `v1.x`) chỉ báo cáo.
- Khi phát hành `1.0.0`, chủ dự án thêm check **"Rust API compatibility (cargo-semver-checks)"** vào danh sách
  check bắt buộc của nhánh `main` (Settings → Branches → branch protection; agent không có quyền đổi). Thiếu bước
  này thì job đỏ vẫn hiện trên PR nhưng không ngăn được merge.
- Chạy tay trên máy: `cargo xtask semver` (so với tag trước), `--changed origin/main` (chỉ crate bị chạm),
  `--deny` (chặn cả khi còn `0.x`), `--shard k/n` (chỉ phần thứ k trong n). Mọi crate trên máy 4 nhân: khoảng 1
  giờ và ~5 GB đĩa ở `target/semver-checks` (lệnh xoá thư mục này ở đầu mỗi lần chạy; `rm -rf` sau đó để lấy lại
  đĩa) — nên đọc mục ở tóm tắt của job CI thay vì chạy tay.

## Dòng hỗ trợ dài hạn

Ghi cho người ngoài ở [`SECURITY.md`](../SECURITY.md#supported-versions) (+ `.vi.md`); dự án cộng đồng, không SLA.

- Trước `1.0`: chỉ bản minor mới nhất được vá.
- Từ `1.0`: bản minor mới nhất nhận mọi bản sửa; bản minor trước nhận vá bảo mật **3 tháng** sau khi minor mới
  ra; khi có major mới, bản minor cuối của major trước nhận vá bảo mật **6 tháng**.
- Vá cho dòng cũ: nhánh `release/X.Y` cắt từ tag `vX.Y.Z` mới nhất của dòng đó, cherry-pick bản sửa đã vào
  `main`, đổi `version` thành `X.Y.Z+1`, gom mảnh changelog của bản sửa vào mục `## [X.Y.Z+1]`, chạy gate đầy đủ,
  rồi đẩy tag `vX.Y.Z+1` tay lên commit của nhánh đó (workflow `Release` chạy theo push tag). `Auto release` chỉ
  nhìn `main`, không gắn tag cho nhánh `release/*`. Bản sửa trên `main` cũng ra trong bản phát hành kế tiếp của
  `main`. Job `verify` của `Release` thấy phiên bản thấp hơn bản chính thức cao nhất đã có nên không gắn image
  `latest` và không đánh dấu "Latest release" của GitHub (D-20261123-supply-chain); thẻ `X.Y` của dòng cũ vẫn được
  cập nhật.
- Hết hạn hỗ trợ của một dòng thì xoá nó khỏi bảng trong `SECURITY.md`.

## Khi làm việc hằng ngày

Mỗi thay đổi hành vi mà người dùng thấy (API, cấu hình, CLI, migration, Docker) thêm một **file
mảnh** vào `changelog.d/`, **không sửa `CHANGELOG.md`** (D-20261119-changelog-fragments): tên
`<tên>.<mục>.md` — `<tên>` chữ thường, số, `-` (vd `step45-checkout`); `<mục>` là `added`, `changed`,
`deprecated`, `removed`, `fixed` hoặc `security` — nội dung là các gạch đầu dòng tiếng Anh đúng như sẽ
nằm trong changelog. `cargo xtask check` (trong gate) kiểm tên và nội dung. Nhiều phiên song song
không còn xung đột ở `CHANGELOG.md`; xem `changelog.d/README.md`.

## Các bước phát hành

Agent chuẩn bị bản phát hành và merge PR phát hành; khi CI trên `main` của commit đó xanh, workflow `Auto release`
(`.github/workflows/auto-release.yml`, D-20261121-auto-release) tự gắn tag `vX.Y.Z` và gọi workflow `Release`. Workflow `Release` (`.github/workflows/release.yml`,
D-175, D-197) chạy lại gate (chia nhiều máy như CI, chung `gate.yml`, D-20261120-release-gate-sharded) — **trừ khi** workflow CI trên `main` của đúng SHA của tag đã `success` (gate SQLite + Postgres 16 và 18, upgrade-test, quét secret): khi đó job `ci-result` cho bỏ qua gate (D-20261121-ci-batch-release-faster); chưa có lần chạy, đang chạy hay đỏ thì gate chạy như cũ —, build và đăng image Docker (`linux/amd64` và `linux/arm64`) lên
`ghcr.io/<owner>/autoxide:X.Y.Z` và `:X.Y` bằng `GITHUB_TOKEN`, đính kèm chứng thực nguồn gốc build
và SBOM đã ký, rồi tạo bản phát hành GitHub với ghi chú từ `CHANGELOG.md`. Workflow chỉ chạy khi
repo công khai; `./gate.sh` trên máy vẫn là gate bắt buộc trước khi đẩy tag. **Repo đang riêng
tư (D-199): workflow bị bỏ qua, làm bước 1–6 rồi theo "Khi repo còn riêng tư" bên dưới thay cho
bước 7.**

1. Đưa nhánh lên `main` mới nhất: `git fetch origin`, rồi `git rebase origin/main`. Phiên
   khác có thể đã merge trong lúc mình làm — xem "Khi hai phiên đụng nhau" bên dưới.
2. Đổi `version` trong `Cargo.toml` gốc — cả dòng `version` của `[workspace.package]` lẫn `version = "=X.Y.Z"`
   của từng crate nội bộ ở `[workspace.dependencies]` (một lệnh: `sed -i 's/version = "=A.B.C"/version = "=X.Y.Z"/'
   Cargo.toml`; quên thì luật `publish-ready` của `cargo xtask check` đỏ); chạy `cargo check` để cập nhật `Cargo.lock`. Đổi
   theo: `"version"` trong `docs/openapi.json`, `"version"` trong `sdk/typescript/package.json` và
   `sdk/typescript/package-lock.json`, image mặc định trong `compose.quickstart.yml` — `cargo xtask sdk --check`
   của gate kiểm mấy chỗ này; bản phát hành mới
   nhất ở dòng "Trạng thái" của `docs/ROADMAP.md`.
3. Chạy `cargo xtask changelog`: gộp mọi mảnh trong `changelog.d/` vào `## [Unreleased]` (mỗi mảnh vào
   cuối mục của nó) và xoá các mảnh. Rồi trong `CHANGELOG.md`: đổi `## [Unreleased]` thành `## [X.Y.Z] - YYYY-MM-DD`, thêm một
   mục `## [Unreleased]` trống ở trên, cập nhật hai dòng liên kết ở cuối file. Workflow kiểm
   có mục `## [X.Y.Z]` và tag khớp `version`, sai thì không phát hành.
   Rồi lấy mục `### Breaking (Rust)` từ tóm tắt của job "Rust API compatibility (cargo-semver-checks)" ở PR phát
   hành, hay chạy tay `cargo xtask semver --markdown` (cần `cargo-semver-checks`, phiên bản ghim ở `STACK.md`;
   khoảng 1 giờ trên máy 4 nhân và ~5 GB đĩa — `rm -rf target/semver-checks` sau đó): nó in mục `### Breaking (Rust)`
   gồm thay đổi phá vỡ của mọi crate hợp đồng (xem "Kiểm hợp đồng Rust") so với tag trước — job `semver`
   của PR phát hành in sẵn mục này ở tóm tắt; chép vào mục `## [X.Y.Z]`. Không in gì thì không thêm mục. Còn
   `0.x` thì chỉ báo cáo, không chặn (D-167, D-242); từ `1.0` có phá vỡ ở bản không tăng major thì không
   phát hành.
4. Commit `release: vX.Y.Z`, chạy gate (`./gate.sh`, cả SQLite lẫn Postgres) trên bản này, rồi
   `cargo xtask publish-check` (các crate vẫn đóng gói và build được như trên crates.io, xem "Đăng lên crates.io"). Rồi
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

6. **Tag tự gắn** (D-20261121-auto-release): khi workflow CI của push vào `main` trên commit chứa bản phát hành
   (merge commit của PR, có `version = "X.Y.Z"` mới và mục `## [X.Y.Z]` trong `CHANGELOG.md`) kết thúc `success`,
   workflow `Auto release` tạo tag có chú thích `vX.Y.Z` trên **đúng commit đó** rồi gọi `Release` trên tag. Không
   đẩy tag tay, không nhờ chủ dự án bấm Publish. Tag `vX.Y.Z` đã có thì `Auto release` không làm gì, nên các lần
   push sau (cùng phiên bản) không phát hành lại. CI của commit đó đỏ thì không có tag: sửa, merge bản sửa — CI của
   commit mới xanh thì tag gắn lên commit mới (vẫn cùng `version`).

   Vẫn đẩy tag tay được khi cần (vd `Auto release` bị tắt): gắn lên đúng commit phát hành của `main` (commit merge của
   PR phát hành, hay commit `release: vX.Y.Z` nếu merge kiểu rebase), không phải commit mới nhất:

   ```bash
   git fetch origin && git switch main && git merge --ff-only origin/main
   release=$(git log origin/main --format=%H --grep='release: vX.Y.Z' -n 1)
   git log -1 --format='%h %s' "$release"        # phải là commit phát hành vX.Y.Z
   git tag -a vX.Y.Z -m "Autoxide vX.Y.Z" "$release"
   git push origin vX.Y.Z
   ```

   Workflow `Release` thấy trang Release đã có thì cập nhật ghi chú và đính kèm file chứ không báo lỗi. Ghi chú quá
   giới hạn của GitHub (125 000 ký tự) thì workflow thay bằng đoạn ngắn dẫn tới `CHANGELOG.md`.

7. Theo dõi workflow `Auto release` rồi `Release` của tag (Actions) tới khi xanh, rồi kiểm image và file đã đăng
   đúng như người dùng kiểm ([`docs/VERIFYING.md`](VERIFYING.md)):

   ```bash
   gh attestation verify oci://ghcr.io/<owner>/autoxide:X.Y.Z --owner <owner>
   docker run --rm ghcr.io/<owner>/autoxide:X.Y.Z --version
   gh release download vX.Y.Z --dir /tmp/autoxide-vX.Y.Z
   gh attestation verify /tmp/autoxide-vX.Y.Z/autoxide-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz --owner <owner>
   ```

   Workflow ký không cần khoá nào (D-20261123-supply-chain): image có chứng thực nguồn gốc build, chứng thực SBOM và
   chữ ký cosign; binary Linux (amd64, arm64 — chép từ image), gói SDK và SBOM có chứng thực cho từng file, cộng
   `SHA256SUMS` ký bằng cosign (`SHA256SUMS.sigstore.json`).

   Workflow đỏ thì sửa nguyên nhân rồi chạy lại trên cùng tag (Actions → Release → Run workflow,
   chọn tag); không xoá tag đã đẩy. Lỗi nằm ở code thì phát hành bản vá `X.Y.Z+1`.
8. `cargo clean`: mỗi lần đổi phiên bản, cargo build lại cả workspace thành một bộ file mới
   mà không xoá bộ cũ, `target/` phình thêm hàng chục GB.

## Bản thử trước bản major (release candidate)

Bản major là cam kết dài hạn ([`docs/COMPATIBILITY.md`](COMPATIBILITY.md)), nên ra qua một hay nhiều bản thử
`X.0.0-rc.N` trước — lần đầu là `1.0.0-rc.1` (D-20261123-supply-chain). Mở `rc.1` và lên bản chính thức đều do **chủ
dự án quyết** (`1.0` là lúc chủ dự án tuyên bố API ổn định); agent kiểm tiêu chí bên dưới và báo khi đạt.

**Phát hành một bản thử** — đúng các bước 1–8 ở trên, với phiên bản `X.0.0-rc.N`:

- `version` của workspace, ghim `=X.0.0-rc.N` của crate nội bộ, `docs/openapi.json`, `sdk/typescript/package.json`
  (+ lock), image của `compose.quickstart.yml`: đều `X.0.0-rc.N`.
- `CHANGELOG.md`: gộp mảnh như mọi lần, mục mang tên `## [X.0.0-rc.N] - YYYY-MM-DD`.
- Workflow tự nhận ra bản thử (có `-`): trang Release là **pre-release**, ghi chú mở đầu bằng dòng cảnh báo "Release
  candidate…"; image chỉ có thẻ `X.0.0-rc.N` — không `X.0`, không `latest` (người đang dùng `latest` hay `0.x` không bị
  kéo lên bản thử). `Auto release` gắn tag `vX.0.0-rc.N` như mọi bản.
- `rc.1` so với bản `0.x` cuối: job `semver` chỉ báo cáo (tăng major). Từ `rc.2`: so với bản thử trước và **chặn** mọi
  phá vỡ hợp đồng Rust — giữa các bản thử chỉ sửa lỗi. Phá vỡ là bắt buộc thì đó là lỗi nghiêm trọng của bản thử: dừng
  lại hỏi chủ dự án.

**Trong giai đoạn bản thử** (từ `rc.1` tới bản chính thức), `main` đóng băng tính năng: chỉ merge sửa lỗi, tài liệu,
test, và nâng thư viện bản vá. Mảnh changelog chỉ thuộc mục `fixed`, `security`, hay `changed` khi là sửa lỗi. Tính
năng mới nằm chờ ở nhánh của nó, merge sau khi bản chính thức ra.

**Lỗi nghiêm trọng** (chặn bản chính thức): lỗ hổng bảo mật mức cao hay nghiêm trọng; mất hay hỏng dữ liệu; migration
hay nâng cấp hỏng trên SQLite hoặc PostgreSQL; máy chủ không khởi động với cấu hình mặc định; phá vỡ một hợp đồng của
`docs/COMPATIBILITY.md`. Ghi chúng thành issue có nhãn `release-blocker`.

**Tiêu chí từ bản thử lên bản chính thức** — tất cả phải đạt:

1. Bản thử mới nhất đã ra ít nhất **14 ngày**, và trong 14 ngày đó không có lỗi nghiêm trọng nào mở. Có lỗi nghiêm
   trọng thì sửa, ra `rc.N+1`, đếm lại từ đầu.
2. Gate đầy đủ (`./gate.sh`, SQLite và PostgreSQL) xanh trên commit sẽ phát hành, và CI trên `main` của commit đó xanh
   — kể cả test nâng cấp từ bản thử trước (job `upgrade`). Đã chạy `cargo xtask upgrade-test --from <tag 0.x cuối>`
   trên cả hai database: người dùng `0.x` nâng thẳng lên bản chính thức được.
3. Độ phủ end-to-end đạt mục tiêu của P11 bước 70: mọi endpoint trong `docs/openapi.json` và mọi tool trong
   `docs/mcp-tools.json` được gọi thành công ít nhất một lần (`cargo xtask coverage-api --run` không còn mục chưa phủ),
   trên cả hai database.
4. Báo cáo rà soát bảo mật của P6 bước 70 đã đăng, và không còn phát hiện mức cao hay nghiêm trọng nào chưa sửa.
5. `cargo xtask semver` không thấy phá vỡ nào so với bản thử cuối; `cargo deny check` xanh; không có cảnh báo
   Dependabot mức cao trở lên đang mở.
6. Chữ ký và chứng thực của bản thử cuối kiểm được bằng đúng các lệnh của [`docs/VERIFYING.md`](VERIFYING.md).
7. Chủ dự án đồng ý.

**Phát hành bản chính thức**: bước 1–8 với `X.0.0`. Mục `## [X.0.0]` của `CHANGELOG.md` gồm thay đổi sau bản thử cuối,
kèm dòng đầu `Includes every change of X.0.0-rc.1 to X.0.0-rc.N, listed below.`; các mục bản thử giữ nguyên bên dưới.
Khi ra `1.0.0`, chủ dự án thêm check **"Rust API compatibility (cargo-semver-checks)"** vào danh sách check bắt buộc
(mục "Kiểm hợp đồng Rust").

## Đăng lên crates.io (chưa bật — chờ chủ dự án)

Mọi crate trừ `xtask` và module mẫu `notes` đã **sẵn sàng** cho crates.io nhưng **chưa đăng**
(D-20261121-external-modules): đăng là việc không hoàn tác được (một phiên bản đã đăng chỉ "yank" được, tên crate
giữ vĩnh viễn) và cần tài khoản của chủ dự án. Agent không tự đăng, không đổi `publish = false`.

Giữ cho sẵn sàng (đã chạy mỗi lần gate và mỗi lần phát hành):

- `cargo xtask check` — luật `publish-ready`: metadata đủ, phụ thuộc nội bộ ghim `version = "=X.Y.Z"`.
- `cargo xtask publish-check` (lúc phát hành, bước 4; lâu như một lần build sạch, ghi vào `target/package`) —
  `cargo package` cho từng crate rồi build từ gói đã đóng, với phiên bản thư viện mới nhất hợp lệ: đúng thứ người
  dùng crates.io nhận. Đỏ thì sửa trước khi phát hành.

Khi chủ dự án bảo đăng, lần đầu (làm tay, một lần):

1. Chủ dự án tạo tài khoản crates.io (đăng nhập bằng GitHub), xác nhận email, tạo một API token có quyền
   `publish-new` và `publish-update` (https://crates.io/settings/tokens). Token là secret: không vào git, không dán
   vào trò chuyện.
2. Kiểm lại tên còn trống: 36 tên `autoxide`, `autoxide-kernel`, `autoxide-testkit`, `autoxide-<module>` (ngày
   2026-11-21 đều trống). Tên đã bị người khác lấy thì dừng lại hỏi chủ dự án.
3. Đổi `publish = false` ở `[workspace.package]` của `Cargo.toml` gốc thành `publish = true` (`xtask` tự khai `publish = false`;
   module mẫu `notes` luôn bị loại bằng `--exclude autoxide-notes`), commit.
4. Trên đúng commit của tag phát hành, gate xanh: `cargo publish --workspace --exclude xtask --exclude
   autoxide-notes --dry-run`, rồi bỏ `--dry-run`. Cargo tự xếp thứ tự theo phụ thuộc: `autoxide-kernel` →
   `autoxide-testkit` → các module (module nền trước module dùng nó) → `autoxide`. crates.io giới hạn số crate MỚI
   mỗi tài khoản (một đợt vài crate, sau đó chậm lại): lệnh dừng giữa chừng vì giới hạn thì chờ rồi chạy lại — crate
   đã đăng được bỏ qua; có thể xin nâng giới hạn qua help@crates.io trước.
5. Với từng crate, trên crates.io: Settings → Trusted Publishing → thêm repo GitHub này, workflow `release.yml`
   (Trusted Publishing chỉ cấu hình được sau khi crate đã có, nên lần đầu phải dùng token). Xong thì thu hồi token.

Từ lần sau (tự động, không secret lâu dài): thêm vào `release.yml` một job `crates` chạy sau `publish`, quyền
`id-token: write`, dùng action `rust-lang/crates-io-auth-action` lấy token ngắn hạn qua OIDC rồi chạy
`cargo publish --workspace --exclude xtask --exclude autoxide-notes`. Job này CHƯA có trong workflow: thêm nó cùng
lúc với bước 3, ghim action theo SHA như mọi action khác (D-20261121-pin-ci-deps). Cách cấu hình Trusted Publishing
và tên action nên đối chiếu lại tài liệu crates.io lúc làm (https://crates.io/docs/trusted-publishing).

Sau khi đăng: sửa `docs/guides/writing-a-module.md` (+ bản `.vi.md`) và `examples/external-module/Cargo.toml` để
hướng dẫn phụ thuộc bằng phiên bản crates.io thay cho git tag.

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
thư viện Rust thành một PR; npm của `sdk/typescript`, image Docker và action của GitHub mỗi loại một PR. Chỉ đề xuất
bản đã ra ít nhất 7 ngày (`cooldown`, D-20261123-supply-chain); bản vá lỗ hổng thì không chờ. CI chạy gate trên PR (khi repo công khai); người duyệt hoặc agent
vẫn chạy `./gate.sh` trên máy, xanh mới merge. Bản lớn (major) đến từng PR riêng: đọc
CHANGELOG của thư viện (skill `check-library-api`) trước khi merge. Lỗ hổng thư viện được
`cargo deny check` trong gate bắt; đỏ thì nâng thư viện hoặc ghi ngoại lệ có lý do trong
`deny.toml` và `docs/DECISIONS.md`.
