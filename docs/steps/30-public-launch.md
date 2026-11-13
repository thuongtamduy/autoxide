# Bước 30 — Ra mắt công khai

> Lộ trình: `docs/ROADMAP.md` bước 30 · Quyết định: D-175 (công khai, bật lại CI và phát hành tự
> động; thay D-107), D-176 (WebP), D-078 (phát hành, image GHCR, Dependabot), D-074 / D-108
> (gitleaks), D-168 (số đo ảnh thu nhỏ), D-171 (agent tự phát hành), D-173 (gate hai mức),
> D-178 (đánh số lại bước), D-192 (thứ tự từ bước 30)

## Mục tiêu

Người lạ tìm thấy repo, đọc README tiếng Anh, chạy được Autoxide bằng image Docker đã đăng
(`ghcr.io/<owner>/autoxide`, cả `linux/amd64` lẫn `linux/arm64`, kèm chứng thực nguồn gốc build và
SBOM), trên SQLite hay Postgres, mà không cần toolchain Rust và không phải hỏi ai. Mỗi push và pull
request chạy lại gate trên GitHub (lưới thứ hai sau `./gate.sh` trên máy); đẩy tag `vX.Y.Z` là đủ để
có bản phát hành và image. Repo có đủ file của một dự án mã nguồn mở (bảo mật, đóng góp, quy tắc ứng
xử, mẫu issue / PR). Sản phẩm thêm hai thứ: bản nhỏ WebP cho ảnh (website nhẹ hơn 60–70%) và deploy
không gián đoạn (`/health/ready` báo 503 trước khi máy chủ ngừng nhận kết nối).

## Bắt đầu khi

Ngay (bước 29 đã merge). Song song với 31 và 32, nhưng có file dùng chung — giữ thứ tự sau:

- **WebP (C1) làm và merge vào `main` trước tiên** (gate đầy đủ xanh): bước 32 chờ nó mới dời file
  của `media` (D-192).
- `.github/workflows/ci.yml`, `release.yml`, `gate.sh`, `docs/RELEASING.md` là của bước này. Bước 31
  sẽ thêm vào chúng (bước `api-diff` trong gate, `fetch-depth: 0` cho job gate, job nâng cấp, job SDK,
  bước test nâng cấp trong RELEASING): giữ cấu trúc file dễ thêm, không gộp job tuỳ tiện.
- Image đầu tiên được workflow đăng là đầu vào cho test nâng cấp của bước 31.

## Việc chỉ chủ dự án làm được — agent KHÔNG tự làm

Agent chuẩn bị, kiểm và báo bằng tiếng Việt; chủ dự án bấm. Agent được dùng lệnh **chỉ đọc**
(`gh api repos/<owner>/autoxide --jq .security_and_analysis`, `gh api …/private-vulnerability-reporting`)
để xác nhận sau khi chủ dự án làm.

| Việc | Ở đâu | Agent làm trước |
| --- | --- | --- |
| Đổi repo sang công khai | Settings → General → Danger Zone → Change visibility | Báo cáo kiểm A1 (bên dưới) xanh |
| Bật Private vulnerability reporting | Settings → Code security → Private vulnerability reporting | `SECURITY.md` đã trỏ tới nút "Report a vulnerability" |
| Bật Secret scanning, Push protection, Dependabot alerts | Settings → Code security | — (repo công khai có thể bật sẵn; agent kiểm bằng `gh api`) |
| Địa chỉ liên hệ trong `CODE_OF_CONDUCT.md` | chủ dự án đưa địa chỉ | Agent hỏi; chưa có thì mục B3 để `[ ]`, không bịa địa chỉ, không để chỗ trống trong file đã commit |
| Gói image thành Public (nếu sau lần đăng đầu vẫn Private) | Packages → autoxide → Package settings → Change visibility | Agent kiểm bằng `docker pull` khi chưa đăng nhập |
| Bật Actions nếu bị tắt ở cấp repo | Settings → Actions → General | — |
| ~~Quyết về lịch sử git~~ — **đã quyết (chủ dự án, 13/11/2026, D-198): giữ nguyên lịch sử git**, kể cả email tác giả commit và dòng `Claude-Session:` | — | Không viết lại lịch sử, không hỏi lại; kiểm gitleaks toàn lịch sử vẫn phải xanh |

## Phạm vi

Sửa / tạo:

- `.github/workflows/{ci,audit,release}.yml`, `.github/dependabot.yml`
- `.github/ISSUE_TEMPLATE/{bug_report.yml,feature_request.yml,config.yml}`,
  `.github/pull_request_template.md`
- `SECURITY.md`, `SECURITY.vi.md`, `CONTRIBUTING.md`, `CONTRIBUTING.vi.md`, `CODE_OF_CONDUCT.md`
  (bản `.vi.md` xem mục "Agent tự quyết")
- `xtask/src/` — luật mới giữ file dự án và cặp EN / VI (file mới, vd `xtask/src/docs.rs`)
- `modules/media` (WebP), `modules/content/src/http/public.rs` (`PublicMedia`), `docs/openapi.json`
- `crates/kernel/src/http/health.rs`, `crates/kernel/src/config/http.rs`, `crates/kernel/locales/vi.json`,
  `crates/app/src/runtime.rs`, `crates/app/src/commands/serve.rs`, `.env.example`, `compose.yml`
- `README.md` / `.vi.md`, `docs/DEPLOY.md` / `.vi.md`, `docs/MEDIA.md` / `.vi.md`, `docs/RELEASING.md`,
  `gate.sh` (chỉ chú thích D-107 / D-108), `STACK.md`, `CHANGELOG.md`, `docs/DECISIONS.md`, `docs/ROADMAP.md`

Ngoài phạm vi: đăng npm, binary cho từng hệ điều hành, AVIF (chờ chủ dự án), Helm / Kubernetes
manifest, kiểm hợp đồng API và test nâng cấp (bước 31), dời file của `media` (bước 32), mọi bảng
có `tenant_id` (bước 33).

## Thiết kế

### A1. Kiểm trước khi công khai (không commit, chỉ báo cáo)

1. gitleaks toàn lịch sử: bước "secrets" của `./gate.sh` (hoặc `gitleaks git . --no-banner --redact
   --config .gitleaks.toml`) xanh.
2. File đang theo dõi: `git ls-files -z | xargs -0 grep -nIE '<mẫu>'` với các mẫu: đường dẫn riêng
   (`/home/`, `/Users/`, `C:\\Users`), tên máy (`\.local\b`, `\.lan\b`, `ssh `), địa chỉ IPv4 không
   thuộc dải tài liệu / riêng dùng trong test (bỏ `127.0.0.1`, `0.0.0.0`, `10.`, `192.168.`,
   `192.0.2.`, `198.51.100.`, `203.0.113.`, `169.254.169.254` của test SSRF), email khác
   `example.com` / `localhost` / `noreply`. Lần quét sơ bộ khi viết đặc tả: chỉ thấy IP mẫu trong test
   (`modules/webhooks/src/network.rs`, `crates/kernel/src/http/client.rs`) và đường dẫn repo GitHub
   trong `CHANGELOG.md` — cái sau là địa chỉ công khai của dự án, giữ.
3. Toàn lịch sử, kể cả file đã xoá: `git log -p --all | grep -nE '<cùng mẫu>'`.
4. Siêu dữ liệu commit: `git log --format='%an <%ae>' | sort | uniq -c` và trailer trong thông điệp —
   báo chủ dự án (bảng trên), không tự sửa.
5. `.claude/settings.json` và `.claude/skills/` được theo dõi: đọc lại, không có đường dẫn riêng.

Báo cáo tiếng Việt: lệnh đã chạy, kết quả, việc chủ dự án cần làm.

### A2. Ba workflow tự chạy lại (D-175 thay D-107)

Trả đúng trigger cũ (commit `429f8d9` là commit tắt; bản trước nó có trigger gốc):

| File | `on:` |
| --- | --- |
| `ci.yml` | `push: branches: [main]`, `pull_request:`, `workflow_dispatch:` |
| `audit.yml` | `schedule: - cron: "17 3 * * *"`, `workflow_dispatch:` |
| `release.yml` | `push: tags: ["v*.*.*"]`, `workflow_dispatch:` (chạy thử, xem A3) |

- Bỏ ba dòng chú thích "TẮT tự chạy (D-107)…" ở đầu cả ba file.
- `dependabot.yml`: trả mục `package-ecosystem: github-actions` (D-107 bỏ vì workflow không chạy) và
  sửa chú thích đầu file ("CI chạy gate trên từng đề xuất").
- Kiểm bản mới nhất của từng action (`gh api repos/<action>/releases/latest`), kiểm cú pháp bằng
  `actionlint` (image Docker ghim phiên bản, như D-074).
- Job `secrets` của CI dùng `ghcr.io/gitleaks/gitleaks:v8.30.0`, gate dùng
  `zricethezav/gitleaks:v8.30.0` (D-108): đổi CI sang cùng image với gate để một chỗ ghim.
- Sửa chữ "CI đã tắt" ở: chú thích bước 5 trong `gate.sh` (D-107 / D-108: giữ gitleaks trong gate,
  bỏ câu "repo riêng tư"), `docs/RELEASING.md`, dòng "Trạng thái" của `docs/ROADMAP.md`, `STACK.md`
  (cụm "chạy trong gate" thêm "và CI").
- **Thứ tự:** commit trigger được làm trước khi repo công khai, nhưng mục này chỉ `[x]` khi cả ba
  workflow đã chạy xanh trên GitHub (CI trên push lên `main`; Audit bấm tay một lần; Release qua A3).

### A3. Workflow `Release`: image hai kiến trúc, chứng thực, SBOM

Viết lại job `publish` của `release.yml` thành bốn job:

1. `gate` — như hiện có (kiểm tag khớp `Cargo.toml`, mục CHANGELOG, chạy `./gate.sh`). Khi chạy bằng
   `workflow_dispatch` (chạy thử) thì bỏ bước kiểm tag.
2. `image` — ma trận `{platform: linux/amd64, runner: ubuntu-24.04}`,
   `{platform: linux/arm64, runner: ubuntu-24.04-arm}` (runner ARM thật, miễn phí cho repo công khai;
   không dùng QEMU — D-078 bỏ ARM vì giả lập quá lâu). `docker/build-push-action` với
   `platforms: ${{ matrix.platform }}`, `outputs: type=image,name=ghcr.io/<repo>,push-by-digest=true,name-canonical=true,push=true`,
   cache `type=gha,scope=<platform>`, `provenance: false`, `sbom: false` (chứng thực làm ở bước 3,
   tránh manifest "unknown/unknown" của BuildKit). Ghi digest ra artifact.
3. `manifest` — tải các digest, `docker buildx imagetools create` gắn tag `X.Y.Z` và `X.Y`
   (`docker/metadata-action`, `type=semver`), lấy digest của manifest list, rồi:
   - `actions/attest-build-provenance` với `subject-name: ghcr.io/<repo>`, `subject-digest`,
     `push-to-registry: true`;
   - SBOM SPDX của image (vd `anchore/sbom-action`, định dạng `spdx-json`) rồi `actions/attest-sbom`
     cùng subject, `push-to-registry: true`; đính kèm file SBOM vào bản phát hành.
   - Quyền job: `contents: read`, `packages: write`, `id-token: write`, `attestations: write`.
4. `release` — ghi chú từ CHANGELOG (awk sẵn có), `gh release create` (quyền `contents: write`).
   Bỏ qua khi chạy thử.

Chạy thử (`workflow_dispatch`): đăng tag `sha-<7 ký tự>` (metadata-action `type=sha`), không đăng
`X.Y.Z` / `X.Y`, không tạo bản phát hành — để kiểm build hai kiến trúc, chứng thực và cài như người lạ
(C3) **trước** khi phát hành thật, mà không phải phát hành giữa bước (D-171).

Tên image: GHCR đòi chữ thường — lấy từ `docker/metadata-action` hay `${GITHUB_REPOSITORY,,}`, không
dùng thẳng `github.repository`.

`docs/RELEASING.md` viết lại: agent làm bước 1–6 (rebase, đổi phiên bản, CHANGELOG, commit
`release: vX.Y.Z`, gate, PR + merge, gắn tag lên đúng commit release, đẩy tag); **workflow** làm bản
phát hành GitHub và image (bỏ bước 7 và 9 tay của agent, bỏ đoạn "token `write:packages` của chủ dự
án"). Thêm: theo dõi `gh run watch` tới khi workflow xanh; workflow đỏ thì sửa, phát hành bản vá
`X.Y.(Z+1)` — **không bao giờ dời hay xoá tag đã đẩy**. Mục "Nâng cấp thư viện": CI chạy trên PR của
Dependabot, xanh mới merge. D-171 sửa theo bằng một mục mới (bỏ "image chỉ đăng khi có token").

`docs/DEPLOY.md` (EN + VI), mục mới "Verify the image" trước bước "Start":

```bash
gh attestation verify oci://ghcr.io/<owner>/autoxide:X.Y.Z --repo <owner>/autoxide
gh attestation verify oci://ghcr.io/<owner>/autoxide:X.Y.Z --repo <owner>/autoxide \
  --predicate-type https://spdx.dev/Document/v2.3
```

kèm cách ghim image theo digest (`AUTOXIDE_IMAGE=ghcr.io/…/autoxide@sha256:…`). Kiểm lệnh thật trên
bản đã đăng trước khi ghi (cờ của `gh attestation` đổi theo phiên bản `gh`).

### A4. Bảo mật của GitHub

Secret scanning, push protection, Dependabot alerts: chủ dự án bật (bảng trên), agent xác nhận bằng
`gh api`. `./gate.sh` giữ nguyên bước gitleaks (D-108, D-175).

### B1–B3. File của dự án mã nguồn mở

`SECURITY.md` (+ `SECURITY.vi.md` cùng mục), ở gốc repo (GitHub hiện ở tab Security):

- **Supported versions**: bảng — bản minor mới nhất của `0.x` nhận bản vá; bản cũ hơn thì nâng cấp.
- **Reporting a vulnerability**: chỉ qua "Report a vulnerability" (Security → Advisories, Private
  vulnerability reporting); không mở issue công khai; nên gửi kèm phiên bản, database, cách tái hiện,
  ảnh hưởng.
- **What to expect**: thời gian phản hồi dự kiến (khuyến nghị: xác nhận đã nhận trong 7 ngày, đánh giá
  trong 14 ngày, công bố phối hợp sau khi có bản vá) — dự án do một người giữ, đừng hứa con số không
  giữ được.
- **Not a vulnerability**: người có `auth.roles.manage` cấp được mọi quyền (thiết kế, có ghi trong mô
  tả permission); tự XSS; tấn công cần quyền quản trị máy chủ; bản cài bỏ qua checklist của
  `docs/DEPLOY.md` (không TLS, mở `/metrics`, thiếu `AUTOXIDE_AUTH_TOKEN_SECRET`); báo cáo máy quét tự
  động không có cách khai thác; lỗ hổng của thư viện mà `cargo deny` đã theo dõi và không khai thác được
  qua Autoxide. Giao diện (repo riêng, D-096) ngoài phạm vi file này.
- README (EN + VI) thêm mục "Security" trỏ tới `SECURITY.md`.

`CONTRIBUTING.md` (+ `.vi.md`) cho người đóng góp là người: dựng môi trường (rustup, C compiler,
Docker, `cargo-deny`, `git config core.hooksPath .githooks`); hai mức gate (D-173: `--quick` lúc
làm, `./gate.sh` đầy đủ trước PR); luật quan trọng nhất tóm từ `CLAUDE.md` — chạy trên cả SQLite và
Postgres, module sở hữu bảng và chỉ nói qua trait, lỗi có `code` ổn định + bản dịch `locales/vi.json`,
code mới có test chạy trên cả hai database, `/api/v1` không phá vỡ (D-059), sinh lại
`docs/openapi.json` / `.env.example`, tài liệu EN + VI cùng commit, dòng `CHANGELOG.md`; cách viết commit
(tiếng Việt hay tiếng Anh đều được, một việc một commit) và PR (mẫu PR); license phần đóng góp
`MIT OR Apache-2.0` (như đoạn cuối README); người dùng AI agent đọc `CLAUDE.md`. README mục
"Contributing" rút gọn và trỏ tới file này.

`CODE_OF_CONDUCT.md`: chép nguyên văn Contributor Covenant **bản mới nhất** (tra
contributor-covenant.org lúc làm; giữ đoạn ghi nguồn theo license CC BY 4.0 của văn bản), điền địa chỉ
liên hệ chủ dự án đưa.

`.github/ISSUE_TEMPLATE/` (issue forms YAML, tiếng Anh):

- `bug_report.yml`: phiên bản (`autoxide --version` hay tag image), database (dropdown SQLite /
  PostgreSQL + phiên bản), cách cài (image Docker, Compose, binary, mã nguồn), các bước tái hiện, kết
  quả mong đợi / thực tế, log (`render: shell`), ô tick "đã tìm issue cũ" và "không phải lỗ hổng bảo
  mật (xem SECURITY.md)".
- `feature_request.yml`: vấn đề, đề xuất, cách khác đã nghĩ, có chạy trên cả hai database không.
- `config.yml`: `blank_issues_enabled: false`; `contact_links` trỏ báo lỗ hổng tới
  `https://github.com/<owner>/autoxide/security/advisories/new`.

`.github/pull_request_template.md`: checklist — `./gate.sh` xanh (SQLite và Postgres); test mới chạy
trên cả hai; EN + VI sửa cùng commit; dòng `CHANGELOG.md` dưới `[Unreleased]`; sinh lại
`docs/openapi.json` / `.env.example` nếu đổi; mục `docs/DECISIONS.md` nếu là quyết định lâu dài; không
có secret.

### B4. Bản phát hành đáng tin

Là A3 (chứng thực + SBOM) cộng mục "Verify the image" của `docs/DEPLOY.md`. Xong khi
`gh attestation verify` xanh trên image của bản phát hành thật.

### B5. Luật giữ file dự án và cặp EN / VI

Luật mới trong `cargo xtask check` (gate gọi sẵn), file mới `xtask/src/docs.rs`, dùng `Violation` của
`boundaries.rs`, tên luật ổn định:

- `project-files`: các file sau phải có — `README.md`, `README.vi.md`, `SECURITY.md`, `SECURITY.vi.md`,
  `CONTRIBUTING.md`, `CONTRIBUTING.vi.md`, `CODE_OF_CONDUCT.md`, `CHANGELOG.md`, `LICENSE-MIT`,
  `LICENSE-APACHE`, `.github/pull_request_template.md`, ba file của `.github/ISSUE_TEMPLATE/`.
- `docs-pair`: ở gốc repo và `docs/` (đệ quy, **trừ** `docs/research/`, `docs/prompts/`, `docs/steps/`
  và file nội bộ tiếng Việt `ROADMAP.md`, `DECISIONS.md`, `RELEASING.md`, `BENCHMARKS.md`): mỗi
  `X.vi.md` phải có `X.md` và ngược lại với danh sách file cho người ngoài; hai bản phải cùng **dãy cấp
  tiêu đề** (`#`…`######`, theo thứ tự) và cùng **số khối code**. Thông báo nêu vị trí lệch đầu tiên:
  `[docs-pair] docs/DEPLOY.vi.md: heading 7 is level 3, docs/DEPLOY.md has level 2 ("…")`.
- Bỏ qua dòng nằm trong khối code khi tìm tiêu đề (README có `# Build and run…` trong khối `bash`).
  Lúc viết đặc tả, chín cặp hiện có đều khớp theo luật này.
- Test đơn vị trong `docs.rs` cho từng luật (thiếu file, thiếu bản dịch, lệch cấp, lệch số khối,
  tiêu đề trong khối code).

### C1. `media`: bản nhỏ WebP (D-176)

Chọn bộ mã hoá (skill `check-library-api`, đúng bản ghim): `image` 0.25.10 đang ghim chỉ mã hoá WebP
**không mất dữ liệu** (đã kiểm `src/codecs/webp/encoder.rs`: "Right now only lossless encoding is
supported… use libwebp"), không đủ cho số đo của D-168. Ứng viên: crate `webp` (bọc `libwebp` qua
`libwebp-sys`, build C bằng `cc` — Dockerfile đã có trình biên dịch C vì SQLite đóng gói sẵn; license
libwebp BSD-3-Clause có trong `deny.toml`). Kiểm: nén mất dữ liệu có alpha, build trên `linux/arm64`,
`cargo deny` xanh, không cần thư viện hệ thống. Thêm vào `[workspace.dependencies]` và `STACK.md`.

Dữ liệu — migration mới `m0006_variant_formats` của `media` (số thật theo thư mục, migration cũ không sửa):

- `media_variant` giữ cột; một ảnh có thể có hai dòng cùng `width` (JPEG/PNG và WebP).
- `up`: bỏ index `media_variant_unique` (`item_id, version_id, width`), tạo
  `media_variant_format_unique` (`item_id, version_id, width, content_type`).
- `down`: xoá dòng `content_type = 'image/webp'` **trước**, rồi bỏ index mới, tạo lại index cũ (file
  WebP mồ côi do job `media.clean_orphans` dọn).
- Khoá nơi lưu của WebP: `media/<item_id>/<version_id>_w<width>.webp` (bản JPEG/PNG giữ khoá cũ).
  Hàm `storage_key` nhận thêm định dạng; `jobs.rs::parse_key` phải nhận đuôi `.webp`.

Sinh (`variants.rs`): mỗi chiều rộng làm thêm một bản WebP nén mất dữ liệu (ảnh có alpha: WebP có
alpha). Cùng job `media.make_variants`, cùng lần giải mã, trên `spawn_blocking`. Chất lượng là hằng
số có lý do (khuyến nghị bắt đầu 80, đo rồi chốt) hoặc Setting mới `AUTOXIDE_MEDIA_WEBP_QUALITY`
(thêm thì sinh lại `.env.example`).

API — khuyến nghị (D-176 để agent quyết; ghi quyết định):

- `variants` **giữ nguyên** (chỉ JPEG / PNG): client cũ dựng `srcset` từ đó không đổi gì. Trộn WebP vào
  sẽ làm `srcset` có hai ứng viên cùng chiều rộng.
- Trường mới `sources: [{ "content_type": "image/webp", "variants": [{width, height, url}] }]` trong
  `MediaItemResponse` (`modules/media/src/http/mod.rs`), `PublicMedia` (`modules/content/src/http/public.rs`
  — dùng cho `featured_media`, `og_image`, `body_media`) và `MediaRef` của `autoxide_media::api` (kiểu
  mới `MediaSource`). Dạng danh sách để AVIF sau này chỉ thêm phần tử, không thêm trường. Website dùng
  `<picture><source type="image/webp" srcset="…"><img srcset="…"></picture>`.
- URL: `url` của item + `&w=<width>&format=webp`. `GET /api/v1/media/{id}/content` nhận tham số query
  `format` (chỉ `webp`, chỉ đi cùng `w`; sai thì 422 với `errors`). Bản WebP chưa làm xong thì trả bản
  gốc và không cache lâu, như `w` hiện nay. ETag của bản WebP khác bản JPEG cùng cỡ
  (`"<version_id>-w<width>-webp"`).
- Không thương lượng theo header `Accept` (cache / CDN phải `Vary: Accept`, URL có phiên bản mất tác dụng).

Làm lại ảnh đã có: `refresh_variants` (`refresh.rs`) coi ảnh là lệch khi thiếu bản WebP ở một cỡ chắc
chắn làm được (cùng luật `needs_refresh` hiện có, xét riêng từng định dạng). `unique_key` của job giữ
như cũ. Mô tả task sửa theo ("…or that have no WebP copies yet").

Đo (ghi vào quyết định): ảnh chụp cỡ của D-168 (khoảng 6000 × 4000, license tự do, **không commit**
vào repo — ghi nguồn) và một PNG có vùng trong suốt; với mỗi chiều rộng mặc định của
`AUTOXIDE_MEDIA_IMAGE_WIDTHS`: dung lượng JPEG/PNG so với WebP, thời gian mã hoá, trên bản build
release.

`docs/MEDIA.md` (EN + VI), mục "Smaller copies of images": `sources`, ví dụ `<picture>`, `format=webp`,
chạy `autoxide task media.refresh_variants` sau khi nâng cấp.

### C2. Deploy không gián đoạn

- Setting mới (kernel, `config/http.rs`): `AUTOXIDE_HTTP_SHUTDOWN_DRAIN_SECONDS` — "On shutdown, how
  long `/health/ready` answers 503 while the server keeps serving, so load balancers stop sending new
  requests before connections close. Then requests in progress get up to
  `AUTOXIDE_HTTP_SHUTDOWN_SECONDS`." Khoảng `0..=300`; mặc định khuyến nghị `0` (giữ hành vi hiện
  tại cho máy đơn, test, Compose). `HttpConfig` (đã `#[non_exhaustive]`) thêm `shutdown_drain: Duration`.
  Sinh lại `.env.example`.
- Kiểu mới `autoxide_kernel::http::Draining` (phác thảo, tra API thật):
  `#[derive(Clone, Default)] pub struct Draining(Arc<AtomicBool>)`, `fn start(&self)`,
  `fn is_draining(&self) -> bool`. `health_routes(database)` **giữ chữ ký** (testkit và test đang
  gọi); thêm `health_routes_with(database, draining: Draining)`.
- `/health/ready` khi đang drain: kiểm cờ **trước** cache 1 giây (`READY_CACHE`), trả 503 mã mới
  `shutting_down` ("The server is shutting down; send requests to another instance."). `/health/live`
  vẫn 200. Thêm 503 này vào mô tả OpenAPI của `ready`, sinh lại `docs/openapi.json`.
- `crates/app`: `runtime::start` tạo `Draining`, giữ trong `Runtime`; `run_server` bọc tương lai
  `shutdown`: tín hiệu đến → `draining.start()`, log `draining`, chờ `shutdown_drain` **hoặc tín hiệu
  dừng thứ hai** (tạo thêm một `shutdown_signal()`; Ctrl-C lần hai bỏ qua phần chờ) → rồi mới tới
  graceful shutdown của `http::serve` với `shutdown_grace` như cũ. Không cần sửa `http::serve`.
- Áp cho cả vai `worker` (chỉ có health check).
- `docs/DEPLOY.md` (EN + VI): bảng "Settings worth knowing" và mục "Running several servers": đặt drain
  lớn hơn chu kỳ hỏi readiness của bộ cân bằng tải; thời gian chờ dừng của hệ thống vận hành
  (`stop_grace_period` của Compose — mặc định Docker 10 giây —, `TimeoutStopSec` của systemd,
  `terminationGracePeriodSeconds` của Kubernetes) phải lớn hơn drain + `AUTOXIDE_HTTP_SHUTDOWN_SECONDS`.
  `compose.yml` thêm `stop_grace_period` cho `app` khớp mặc định.

### C3. Cài như người lạ

Trên container sạch (`docker run --rm -it debian:trixie-slim` có Docker-in-Docker, hay máy ảo mới),
chỉ đọc README tiếng Anh, dùng image đã đăng (bản chạy thử `sha-…` của A3, rồi bản phát hành thật):
SQLite bằng `docker run` (một volume), Postgres bằng Compose với `AUTOXIDE_IMAGE`. Ghi mọi chỗ vấp
vào báo cáo, sửa README / `docs/DEPLOY.md` (EN + VI cùng commit). Chỗ đã biết sẽ vấp, kiểm và sửa:

- README "Getting started" chỉ có đường build bằng `cargo`; cần mục "Quick start with Docker" ở đầu.
- `serve` từ chối chạy khi còn migration (CMD của image là `serve`): hướng dẫn phải chạy
  `docker run … migrate up`, rồi `user create --admin`, rồi `serve`. Không tự đổi để `serve` migrate
  (quyết định cũ: tách hai việc khi chạy nhiều bản).
- `compose.yml` có `build: .`: người chỉ tải `compose.yml` (không clone) dùng `--no-build` được không —
  thử; nếu không, hướng dẫn tải đúng file theo tag.
- `<repository-url>`, `ghcr.io/<owner>/autoxide:0.1` trong README / DEPLOY: thay bằng đường dẫn thật
  của repo (như liên kết cuối `CHANGELOG.md`) và tag thật sau khi công khai.
- Volume SQLite thuộc uid 10001 trong image; backup ra thư mục máy chủ cần `chown 10001` (DEPLOY đã ghi
  cho Compose — kiểm cho đường SQLite).

### Mã lỗi, event, permission, job

- Mã lỗi mới: `shutting_down` (kernel) — vi: "Máy chủ đang tắt; hãy gửi request tới máy chủ khác.";
  lỗi 422 của `format` dùng `errors` của `validation` sẵn có (không cần mã mới).
- Không thêm event, permission, tool MCP. Job `media.make_variants` giữ tên, làm thêm WebP.

## Thứ tự làm

Cột "Mục ROADMAP": `A.2` là mục thứ hai của nhóm A trong bước 30 của `docs/ROADMAP.md` (A.5 là
image ARM64; B.4 là chứng thực + SBOM; C.3 là cài như người lạ).

| # | Commit | Mục ROADMAP |
| --- | --- | --- |
| 1 | (không commit) A1: báo cáo kiểm, gửi chủ dự án danh sách việc của họ, hỏi địa chỉ CoC | A.1 |
| 2 | `media`: chọn bộ mã hoá WebP, đo, quyết định + `STACK.md` | C.1 |
| 3 | `media`: migration, sinh WebP, khoá nơi lưu, `parse_key`, `refresh_variants` | C.1 |
| 4 | `media` + `content`: `sources`, `format=webp`, ETag, `docs/openapi.json`, `docs/MEDIA*.md`, CHANGELOG → gate đầy đủ, **merge vào `main`** | C.1 |
| 5 | kernel + app: drain, Setting, `shutting_down`, `.env.example`, `compose.yml`, DEPLOY, CHANGELOG | C.2 |
| 6 | `SECURITY*`, `CONTRIBUTING*`, mẫu issue / PR, README trỏ tới | B.1, B.2, B.3 (trừ CoC) |
| 7 | `CODE_OF_CONDUCT.md` khi có địa chỉ | B.3 |
| 8 | `xtask`: `project-files`, `docs-pair` | B.5 |
| 9 | trigger ba workflow, Dependabot, chú thích D-107, STACK / gate.sh (sau khi repo công khai) | A.2 |
| 10 | `release.yml` bốn job + ARM64 + chứng thực + SBOM; RELEASING; DEPLOY "Verify the image"; chạy thử `workflow_dispatch` | A.3, A.5, B.4 |
| 11 | (không commit) xác nhận cài đặt bảo mật của GitHub | A.4 |
| 12 | C3 trên image chạy thử; sửa README / DEPLOY | C.3 |
| 13 | quyết định mới ghi D-175 đã làm thế nào, D-171 bỏ "token của chủ dự án"; đánh dấu ROADMAP; phát hành (D-171) — workflow đăng image; kiểm lại C3 + `gh attestation verify` trên bản thật | tất cả |

## Test bắt buộc

Chạy trên cả SQLite và Postgres (`TestDatabase::run`, `TestApp::builder`):

- `media`: `lam_anh_thu_nho_co_ca_webp` (render: mỗi cỡ có JPEG/PNG + WebP, WebP giải mã được, đúng
  kích thước, ảnh có alpha ra WebP có alpha); `variant_webp_qua_tham_so_format` (HTTP: `&w=…&format=webp`
  trả `image/webp`, ETag khác bản JPEG; chưa làm xong thì bản gốc + cache ngắn; `format=gif` → 422;
  `format` không có `w` → 422); `sources_trong_item_va_ban_cong_khai` (`media` và `content`:
  `featured_media.sources`, `body_media[].sources`; `variants` không có WebP); `refresh_variants_xep_job_cho_anh_thieu_webp`
  (ảnh chỉ có JPEG cũ → bị coi là lệch; chạy hai lần không xếp trùng); `parse_key` nhận `_w640.webp`;
  `clean_orphans` xoá file WebP mồ côi; migration `m0006` up → down → up có dòng WebP (down không vỡ
  index unique); thay nội dung ảnh xoá cả bản WebP cũ.
- Kernel (`crates/kernel/tests/health.rs`): `ready_tra_503_khi_dang_drain` (mã `shutting_down`, `live`
  vẫn 200, cờ thắng cache); `crates/kernel/src/config/http.rs`: đọc / mặc định / ngoài khoảng của
  `AUTOXIDE_HTTP_SHUTDOWN_DRAIN_SECONDS`.
- App (`crates/app/src/commands/serve.rs`): `drain_tra_503_roi_moi_dung` (drain 300 ms: sau tín hiệu,
  `/health/ready` 503, `/health/live` 200, request thường vẫn 200; máy chủ dừng sau ≥ 300 ms);
  `tin_hieu_thu_hai_bo_qua_drain`.
- `xtask`: test đơn vị của `project-files` và `docs-pair` (B5).
- Test `.env.example` và `docs/openapi.json` sẵn có xanh sau khi sinh lại.

## Tiêu chí xong

- [ ] Báo cáo A1 gửi chủ dự án; repo công khai (chủ dự án làm)
- [ ] CI (push `main`, PR), Security audit (lịch), Release (tag) chạy xanh trên GitHub; không còn chú
      thích D-107; Dependabot có lại `github-actions`
- [ ] Image `ghcr.io/<owner>/autoxide:X.Y.Z` và `:X.Y`, hai kiến trúc, tải được khi chưa đăng nhập;
      `gh attestation verify` (provenance và SBOM) xanh; `docs/RELEASING.md` chỉ còn agent đẩy tag
- [ ] Secret scanning, push protection, Dependabot alerts bật (đã kiểm bằng `gh api`); gitleaks còn trong gate
- [ ] `SECURITY`, `CONTRIBUTING` (EN + VI), `CODE_OF_CONDUCT.md` có địa chỉ thật, mẫu issue / PR; README trỏ tới
- [ ] `cargo xtask check` chặn thiếu file và lệch cặp EN / VI
- [ ] WebP: số đo trong quyết định; `docs/MEDIA*.md`; client cũ không đổi (`variants` như cũ)
- [ ] Drain: Setting, test, `docs/DEPLOY*.md`, `.env.example`
- [ ] Cài như người lạ trên bản phát hành thật, SQLite và Postgres, README / DEPLOY đã sửa theo
- [ ] `./gate.sh` đầy đủ xanh; CHANGELOG có dòng cho WebP, drain, image ARM64 / chứng thực, file dự án;
      quyết định mới trong `docs/DECISIONS.md`; mọi mục bước 30 `[x]`; phát hành theo `docs/RELEASING.md`

## Chỗ dễ sai

- **Thứ tự công khai:** workflow không chạy được khi repo còn riêng tư (lý do của D-107) — đừng đánh dấu
  A2 trước khi thấy chạy xanh thật.
- **GHCR chữ thường**, và gói lần đầu có thể Private dù repo công khai.
- **Manifest list:** chứng thực phải gắn với digest của manifest list (thứ người dùng pull theo tag),
  không phải digest từng kiến trúc; `provenance` mặc định của BuildKit thêm mục "unknown/unknown" làm
  rối manifest.
- **Tag đã đẩy là bất biến**: Release đỏ thì phát hành bản vá, không xoá / gắn lại tag (RELEASING đã
  có tiền lệ v0.11.0 gắn nhầm).
- **Shallow clone:** bước 31 cần `fetch-depth: 0` để so với tag — đừng thêm `fetch-depth: 1` tường minh.
- **WebP:** index unique cũ chặn dòng WebP cùng cỡ — migration phải đổi index trước khi job ghi;
  `down` phải xoá dòng WebP trước khi tạo lại index cũ; `parse_key` không nhận `.webp` thì file mồ côi
  không bao giờ được dọn; ETag trùng giữa hai định dạng làm trình duyệt dùng nhầm bản đã cache;
  `MediaVariant` / `MediaRef` không `#[non_exhaustive]` — thêm trường làm hỏng chỗ dựng bằng literal
  (chỉ trong `media`, kiểm bằng `cargo check --workspace`). Ảnh gốc lớn hơn mọi cỡ: `srcset` của
  `<source type="image/webp">` không có ứng viên cỡ gốc — tài liệu khuyên thêm URL bản gốc làm ứng viên
  lớn nhất.
- **Drain:** kiểm cờ trước cache 1 giây của `ready`; tổng drain + grace vượt thời gian chờ dừng của
  Docker (10 giây mặc định) thì tiến trình bị `SIGKILL` giữa chừng.
- **docs-pair:** tiêu đề trong khối code; file nội bộ tiếng Việt không có bản EN — phải nằm trong danh
  sách loại trừ, không thì luật đỏ oan.
- Thời gian gate trong CI ~ một giờ cho mỗi lần Release (gate chạy lại) — giữ `timeout-minutes: 60`,
  đừng chạy gate hai lần trong một workflow.

## Căn cứ pháp lý

Không có yêu cầu pháp lý Việt Nam riêng cho bước này. Văn bản Contributor Covenant theo CC BY 4.0
(giữ đoạn ghi nguồn). License đóng góp theo Apache-2.0 điều 5 (đã ghi ở cuối README).

## Agent làm bước này tự quyết

Mỗi lựa chọn dưới đây ghi một mục mới trong `docs/DECISIONS.md` (skill `record-decision`, số kế tiếp
sau mục cuối của `main` lúc ghi):

- Bộ mã hoá WebP và chất lượng (khuyến nghị: crate `webp` / `libwebp`, chất lượng 80 sau khi đo); có
  Setting chất lượng hay không.
- Hình dạng API WebP (khuyến nghị `sources` + `format=webp` như trên; ghi lý do không dùng `Accept`).
- Mặc định của drain (khuyến nghị `0`) và tên Setting.
- Công cụ SBOM (syft qua `anchore/sbom-action` hay khác) và có tắt attestation của BuildKit không.
- Cách chạy thử Release trước khi phát hành thật (khuyến nghị `workflow_dispatch` → tag `sha-…`).
- `CODE_OF_CONDUCT.vi.md`: chỉ thêm nếu có bản dịch chính thức tiếng Việt của **cùng phiên bản**
  Contributor Covenant; không thì chỉ bản EN, ghi vào danh sách loại trừ của `docs-pair` và quyết định.
- Ngôn ngữ của mẫu issue / PR (khuyến nghị tiếng Anh, vì GitHub hiển thị cho người lạ).
- Thời gian phản hồi hứa trong `SECURITY.md`.
