# Bước 31 — Hợp đồng API và chống hồi quy

> Lộ trình: `docs/ROADMAP.md` bước 31 · Quyết định: D-059 (endpoint deprecated), D-177 (SDK
> TypeScript trong repo, không npm — D-177 ghi "agent làm bước 32 quyết" theo số cũ; theo bảng
> D-178 đó là bước này), D-004 (OpenAPI là nguồn sinh SDK), D-097 (giao diện ở repo riêng), D-109
> (bản nháp AI chờ duyệt), D-164 / D-169 (`TestApp`), D-173 (gate hai mức), D-192 (bước 33 chờ bước này)

## Mục tiêu

Người viết giao diện, nhà phát triển ngoài và AI agent gọi API không sai, và không có gì âm thầm lệch:
gate chặn mọi thay đổi phá vỡ `/api/v1` so với bản phát hành gần nhất; SDK TypeScript luôn khớp
`docs/openapi.json`; hướng dẫn theo tình huống có ví dụ chạy thật trong gate; mỗi response trong test
HTTP được kiểm khớp schema của chính endpoint đó; database của bản trước nâng lên bản hiện tại được, và
migration của mọi module đi được up → down → up. Đây là lưới an toàn cho bước 33 (sửa mọi bảng của mọi
module) — nên bước 33 chỉ bắt đầu khi bước này xong.

## Bắt đầu khi

Ngay. Riêng test nâng cấp cần image Docker của bản trước: image đầu tiên do workflow của bước 30 đăng;
trước đó, lệnh test nâng cấp tự build image từ tag cũ (xem B1), nên không phải chờ.

File dùng chung với bước khác:

- `.github/workflows/ci.yml`, `gate.sh`, `docs/RELEASING.md` (bước 30 giữ) — bước này **thêm** bước
  `api-diff` vào gate, `fetch-depth: 0` cho job gate, job `upgrade`, job `sdk`, bước test nâng cấp trong
  quy trình phát hành. Rebase lên bản của bước 30 nếu nó đã merge.
- Property test chạm `modules/comments/src/text.rs`, `modules/forms/src/csv.rs`,
  `modules/content/src/entries/{body,slug}.rs` — bước 32 dời file của `comments`, `forms` (và thêm
  Markdown cho bình luận): test đặt **ngay cạnh hàm** (`#[cfg(test)] mod`), để dời file thì test đi theo.
- Bước 30 đổi `docs/openapi.json` (WebP `sources`, 503 `shutting_down`): không phá vỡ theo luật dưới đây.

## Phạm vi

Sửa / tạo: `xtask/src/` (lệnh `api-diff`, `sdk`, có thể `upgrade-test`), `xtask/Cargo.toml` (thêm
`chrono` từ workspace), `crates/kernel/src/http/deprecation.rs` (extension `x-sunset`),
`crates/kernel/src/http/openapi.rs` (media type của lỗi — xem B4), `crates/testkit/src/` (kiểm response
theo hợp đồng, ảnh chụp schema), `crates/app/src/e2e/` và `crates/app/tests/` (ví dụ tài liệu, migration,
nâng cấp), `sdk/typescript/` (mới), `docs/guides/` (mới, EN + VI), `README*.md`, `gate.sh`,
`.github/workflows/ci.yml`, `docs/RELEASING.md`, `STACK.md`, `CHANGELOG.md`, `docs/DECISIONS.md`.

Ngoài phạm vi: đăng npm (D-177), SDK ngôn ngữ khác, `/api/v2`, sửa hành vi endpoint (chỉ sửa **tài
liệu** khi tài liệu sai — và đó là thay đổi phải qua `api-diff`), `cargo semver-checks` cho hợp đồng Rust
(D-167 để sau).

## Thiết kế

### A1. `cargo xtask api-diff` — chặn thay đổi phá vỡ của `/api/v1`

Lệnh: `cargo xtask api-diff [--base <ref>]`. Viết bằng Rust trong `xtask` (file mới
`xtask/src/api_diff.rs`), chỉ dùng `serde_json` + `chrono` (đã có trong workspace) và `git` qua
`std::process::Command`.

**Chọn bản gốc:** `--base` nếu có; không thì `git describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' HEAD`.
Repo shallow (`git rev-parse --is-shallow-repository` = `true`) → lỗi kèm cách sửa (`fetch-depth: 0`).
Không có tag nào (fork chưa lấy tag) → in "no release tag found; nothing to compare" và thoát 0. Tag không
có `docs/openapi.json` → như trên. Đọc bản cũ bằng `git show <tag>:docs/openapi.json`, bản mới từ đĩa.

**Phạm vi so:** đường dẫn bắt đầu `/api/v1/` và `/health/`. Bỏ qua `info`, `servers`, mọi `description`,
`summary`, `example(s)`, `operationId`, `tags`.

**Ghép operation:** theo (method, đường dẫn đã chuẩn hoá — tên tham số đường dẫn thay bằng `{}`): đổi tên
`{id}` → `{item_id}` không phá client.

**Giải `$ref`:** cả hai bên giải về schema thật, so theo **hình dạng** chứ không theo tên component (utoipa
đổi tên component không phá client). Chống vòng lặp bằng tập cặp (ref cũ, ref mới, chiều). Chuẩn hoá
nullable: `type: ["string","null"]` và `oneOf: [X, {type: null}]` đều thành (X, nullable = true) —
`docs/openapi.json` hiện dùng cả hai (369 + 37 chỗ). `allOf` gộp thuộc tính.

**Hai chiều so schema:** *request* (bản mới phải nhận mọi thứ bản cũ nhận) và *response* (bản mới chỉ
được trả thứ bản cũ đã hứa). Luật, mỗi luật một mã ổn định:

| Mã | Phá vỡ khi |
| --- | --- |
| `operation.removed` | (method, path) có ở bản cũ, không có ở bản mới — **trừ** khi operation cũ `deprecated: true` và ngày sunset ≤ hôm nay (UTC) |
| `parameter.added_required` | thêm tham số path / query / header bắt buộc |
| `parameter.became_required` | tham số tuỳ chọn thành bắt buộc |
| `parameter.removed` | bỏ tham số query / header (server bỏ qua tham số lạ → client mất tác dụng mà không biết) |
| `request.body_required` | thêm thân bắt buộc, hay thân tuỳ chọn thành bắt buộc |
| `request.media_type_removed` | bỏ một media type của thân request |
| `request.property_required` | thêm trường bắt buộc, hay trường tuỳ chọn thành bắt buộc |
| `request.type_narrowed` | tập kiểu nhỏ đi (bỏ `null`, `integer` → `string`…), đổi `format` |
| `request.constraint_tightened` | `maxLength` / `maximum` / `maxItems` giảm, `minLength` / `minimum` tăng, `pattern` đổi |
| `response.status_removed` | mã 2xx / 3xx của bản cũ không còn (vd 200 → 201) |
| `response.media_type_removed` | bỏ media type của response 2xx |
| `response.property_removed` | bỏ trường của response |
| `response.property_optional` | trường bắt buộc thành tuỳ chọn hay thành nullable |
| `response.type_changed` | tập kiểu của response lớn ra hay khác, đổi `format` |
| `enum.value_removed` | bỏ giá trị enum (ở bất kỳ chiều nào — đúng chữ của ROADMAP) |
| `schema.composite_changed` | `oneOf` / `anyOf` khác nhau mà không xác định được an toàn → coi là phá vỡ |

Không phá vỡ (in dòng `note:` để người xem biết, không đỏ): thêm endpoint, thêm trường response, thêm
tham số / trường request tuỳ chọn, nới ràng buộc request, thêm giá trị enum **chỉ có trong request**.
Thêm giá trị enum có trong response: in `warning:` (client chặt có thể vỡ), không đỏ.

**Lỗi luôn là Problem:** response 4xx/5xx/`default` chỉ so schema, coi `application/json` và
`application/problem+json` là một (xem B4 — tài liệu hiện ghi sai media type của lỗi).

**Ngày sunset đọc được bằng máy:** `deprecation::apply` (kernel) thêm extension `x-sunset: "YYYY-MM-DD"`
(và `x-deprecated-since`, `x-successor` khi có) vào operation — `utoipa` 6 có `Operation::extensions`.
Hiện chưa endpoint nào deprecated, nên `docs/openapi.json` không đổi. `api-diff` đọc `x-sunset` của
**bản cũ**; bản cũ chưa có extension thì đọc câu "It will be removed on YYYY-MM-DD." trong `description`.

**Ngoại lệ có chủ đích** (tài liệu cũ sai so với hành vi thật, lỗ hổng như tiền lệ D-150 / D-156): file
`xtask/api-breaking-allow.json`, mỗi mục `{ "base": "vX.Y.Z", "rule": "<mã>", "at": "<method path
vị-trí>", "decision": "D-NNN" }`. Mục chỉ có hiệu lực khi `base` đúng bản gốc đang so — tự hết hạn khi có
bản phát hành mới. Mục thiếu `decision` là lỗi. Không có mục nào thì file không cần có.

**Thông báo** (tiếng Anh, như mọi thông báo của gate), vd:

```text
[api-breaking] operation.removed: DELETE /api/v1/media/{id} (in v0.12.0) is gone.
  Keep it working; declare it in Module::deprecations with a sunset date and add the replacement as
  a new endpoint (CLAUDE.md, architecture rule 6; D-059). It may be removed after the sunset date.
[api-breaking] response.property_removed: GET /api/v1/content/entries/{id} 200 $.data.slug
  Clients read this field. Keep returning it (mark it deprecated in its description) and add the new field.
```

**Gate:** `gate.sh` thêm bước "api contract (cargo xtask api-diff)" ngay sau `cargo xtask check`, ở cả
hai mức (nhanh, không cần database); sửa danh sách bước ở đầu `gate.sh`. CI: `actions/checkout` của job
gate thêm `fetch-depth: 0` (lấy cả tag).

**Test** (`#[cfg(test)]` trong `api_diff.rs`, JSON nhỏ viết tay): một test cho mỗi mã ở bảng trên, cộng:
đổi tên tham số đường dẫn không phá; đổi tên component cùng hình dạng không phá; nullable hai cách viết
tương đương; operation deprecated quá sunset được bỏ, chưa tới sunset thì đỏ; mục allowlist khớp `base`
thì bỏ qua, khác `base` thì vô hiệu; `$ref` vòng lặp không treo; so `docs/openapi.json` với chính nó →
không có gì.

### A2. SDK TypeScript (D-177)

Thư mục `sdk/typescript/`:

- `src/schema.ts` — **sinh ra**, đầu file `// Generated by \`cargo xtask sdk\` from docs/openapi.json — do not edit.`
  Kiểu cho mọi `components.schemas`, và `paths` / `operations` (tham số path / query / header, thân
  request, response theo mã). Thứ tự ổn định (sắp theo tên) để diff đọc được.
- `src/client.ts` — viết tay, không phụ thuộc thư viện: `createClient({ baseUrl, token?, language?, fetch? })`;
  `request(method, path, { path, query, body, headers, idempotencyKey, ifNoneMatch })` có kiểu suy từ
  `paths`; thân `{data}` trả `data`; lỗi ném `AutoxideError` (`status`, `code`, `detail`, `errors`,
  `retryAfter` từ header, `problem` nguyên vẹn); `paginate()` là async generator đi theo
  `pagination.next_cursor`; 304 trả `{ notModified: true }`.
- `src/index.ts`, `package.json` (`"private": true`, `"type": "module"`, `typescript` ghim trong
  `devDependencies` + `package-lock.json`), `tsconfig.json` (`strict`), `README.md` + `README.vi.md` (cách
  dùng: chép thư mục, hoặc tải gói `.tgz` đính kèm bản phát hành — xem dưới).

Khuyến nghị công cụ sinh: **viết bằng Rust trong `xtask`** (`cargo xtask sdk` ghi file,
`cargo xtask sdk --check` so với file đã commit) — gate không cần Node, chạy được cả ở `--quick`; tập
JSON Schema mà utoipa sinh ra nhỏ và biết trước (`type`, `properties`, `required`, `items`, `enum`,
`oneOf`, `allOf`, `$ref`, `additionalProperties`, nullable kiểu danh sách). Gate chạy `cargo xtask sdk
--check`; lệch thì thông báo nêu lệnh sinh lại (như test `docs/openapi.json` hiện nay,
`crates/app/src/commands/openapi.rs`). Kiểm kiểu bằng `tsc --noEmit` cho SDK và ví dụ TypeScript chỉ chạy
**trong CI** (job `sdk`, `actions/setup-node` ghim phiên bản) — người lạ chạy `./gate.sh` không cần Node.
Phương án khác (để so trong quyết định): `openapi-typescript` chạy qua image Docker `node` ghim phiên bản,
như gitleaks.

Phát hành: workflow `Release` (bước 30) thêm bước `npm pack` trong `sdk/typescript/` và đính kèm
`autoxide-sdk-X.Y.Z.tgz` vào bản phát hành — client cài bằng URL của file đó, không cần npm registry.
Phiên bản trong `package.json` = phiên bản workspace (bước đổi phiên bản trong `docs/RELEASING.md` thêm
dòng này; `cargo xtask sdk --check` kiểm luôn).

### A3. Hướng dẫn theo tình huống (`docs/guides/`, EN + VI)

Mỗi file một tình huống, mỗi ví dụ có `curl` và TypeScript dùng SDK (khuyến nghị tên file):

- `website.md` — đọc bài và ảnh: `GET /api/v1/content/public/posts`, `…/resolve`, `featured_media` /
  `body_media` với `variants` và `sources` WebP (bước 30) → `<picture>` / `srcset`; ETag / `If-None-Match`
  → 304; `Cache-Control` của URL có phiên bản.
- `authentication.md` — `POST /api/v1/auth/login` (và `…/login/mfa` khi cần mã), làm mới bằng
  `POST /api/v1/auth/refresh` (refresh token xoay vòng, gửi lại token cũ → cả phiên bị thu hồi),
  đăng xuất.
- `api-keys.md` — tạo API key quyền tối thiểu (`/api/v1/auth/api-keys`, cần mật khẩu hiện tại), gửi
  `Authorization: Bearer axk_…`, rate limit theo key (`AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE`).
- `retries.md` — `Idempotency-Key` (24 giờ, `Idempotent-Replayed: true`, 409
  `idempotency_result_not_kept` với kết quả chứa secret), cộng mã tham chiếu `reference` cho lệnh nghiệp
  vụ (D-184).
- `pagination.md` — `limit` / `cursor` / `order`, `pagination.next_cursor`, `paginate()` của SDK.
- `errors.md` — `code` (nhánh theo nó), `detail` theo `Accept-Language`, `errors[]` (`field`, `code`),
  `Retry-After` của 429 / 503.
- `mcp.md` — A4.

Viết theo hợp đồng thật (`docs/openapi.json`), không theo trí nhớ. README (EN + VI) thêm mục trỏ tới
`docs/guides/`.

### A4. Nối AI agent qua MCP (`docs/guides/mcp.md` + `.vi.md`)

Mở rộng đoạn MCP trong README: tạo API key cho agent với đúng permission cần; cấu hình Claude Code
(`claude mcp add --transport http autoxide https://<host>/mcp --header "Authorization: Bearer <key>"`)
và client MCP khác (cấu hình JSON Streamable HTTP chung); bảng tool theo module — tên tool, permission
(và permission thay thế), chỉ đọc / phá huỷ; giới hạn: rate limit theo API key, tool chỉ hiện khi có
quyền, mọi ghi vào audit log, kết quả AI chỉ là gợi ý / bản nháp chờ người duyệt (D-109), tool không làm
được gì REST cùng permission không làm được (luật kiến trúc 8).

Bảng tool **không viết tay rồi để lệch**: test (A5) so bảng trong `mcp.md` với `modules::all()` →
`Modules::tools()` (tên, permission) — thiếu hay thừa dòng thì đỏ, thông báo in dòng bảng đúng.

### A5. Ví dụ trong tài liệu được test

- Ví dụ `curl` viết theo dạng hẹp mà test đọc được: `curl -sS [-X METHOD] "$API/<đường dẫn>" [-H 'Tên:
  giá trị']… [--json '<JSON>' | --data-binary @<file mẫu>]`. Biến lấy từ ví dụ trước khai bằng dòng chú
  thích ngay dưới lệnh: `# save: TOKEN=.data.access_token`. Mỗi khối `bash` có dòng
  `<!-- example: <tên> -->` ngay trước là một bước; khối không đánh dấu không chạy.
- Test `crates/app/src/e2e/guides.rs`: với mỗi file `docs/guides/*.md` (bản EN), dựng app bằng
  `TestApp::builder(test)` với `crate::modules::all()` (đăng nhập thật của `auth`), tạo tài khoản mẫu,
  chạy các bước theo thứ tự, kiểm mã trạng thái kỳ vọng ghi trong dòng `# expect: 201` (mặc định 2xx).
  Trên cả SQLite và Postgres (`TestDatabase::run`).
- Khối code của bản VI phải **giống hệt** bản EN (chú thích trong code viết tiếng Anh) — test kiểm, để
  bản dịch không giữ ví dụ cũ.
- Ví dụ TypeScript: `cargo xtask sdk --extract-examples <thư mục>` chép các khối `ts` ra file; job `sdk`
  của CI chạy `tsc --noEmit` trên chúng cùng SDK. Kiểu sai so với SDK thì CI đỏ.
- Bảng tool của `mcp.md`: test như A4, cùng file.

### B1. Test nâng cấp từ bản phát hành trước

Khuyến nghị cấu trúc:

- Integration test `crates/app/tests/upgrade.rs` (cần binary thật: `env!("CARGO_BIN_EXE_autoxide")` là
  bản hiện tại). Đọc `TEST_UPGRADE_IMAGE` (vd `ghcr.io/<owner>/autoxide:0.12`); chưa đặt thì test in
  "skipped: set TEST_UPGRADE_IMAGE (cargo xtask upgrade-test does it)" và qua — giống cách test S3 phụ
  thuộc `TEST_S3_ENDPOINT`; ghi điều này trong quyết định. Database lấy qua `TestDatabase::run` như mọi
  test (SQLite hay Postgres theo `TEST_DATABASE_URL`).
- `cargo xtask upgrade-test [--from vX.Y.Z]`: chọn tag trước (như A1), lấy image
  `ghcr.io/<owner>/autoxide:X.Y.Z` (`docker pull`); không có thì build từ tag
  (`git worktree add <tạm> vX.Y.Z` → `docker build -t autoxide:vX.Y.Z`); rồi chạy
  `cargo test -p autoxide --test upgrade` hai lần: không `TEST_DATABASE_URL` (SQLite), và với Postgres
  (dựng container như `gate.sh`, hay `GATE_POSTGRES_URL`).

Kịch bản trong test:

1. **Bản cũ** (`docker run` image cũ): SQLite — gắn thư mục tạm của máy vào `/var/lib/autoxide/data`,
   chạy với `--user <uid>:<gid>` của máy (image mặc định uid 10001), `DATABASE_URL=sqlite://data/autoxide.sqlite`;
   Postgres — `--network host` (Linux; ghi giới hạn này), `DATABASE_URL` của database test.
   `migrate up`; `user create --email … --name … --admin --password-stdin`; `serve` (port do test chọn,
   `AUTOXIDE_AUTH_TOKEN_SECRET` cố định cho cả hai bản).
2. Ghi dữ liệu mẫu qua API của bản cũ (HTTP thật, `reqwest` đã có trong workspace — chỉ thêm ở
   `[dev-dependencies]`): đăng nhập; API key; role mới; media (ảnh PNG thật, chờ job ảnh nhỏ); content
   (bài nháp, bài đã đăng có ảnh đại diện và ảnh trong thân); form (định nghĩa qua API quản trị);
   newsletter list; webhook. Tránh endpoint công khai cần token chống spam. Giữ lại ID, access token,
   refresh token.
3. Dừng bản cũ bằng `docker stop` (SIGTERM, tắt êm — SQLite đóng WAL sạch).
4. **Bản hiện tại** trên cùng database: `autoxide migrate up` (thành công), `autoxide doctor` (thoát 0,
   gồm kiểm chuỗi băm audit — D-183), `autoxide serve`.
5. Đọc lại mọi bản ghi đã ghi (cùng trường, cùng giá trị), access token / refresh token cũ còn dùng được,
   đăng nhập bằng mật khẩu cũ, vài lệnh ghi (sửa bài, tải ảnh, tạo role), `autoxide task
   media.refresh_variants` chạy được. Bản cũ chưa có tính năng nào thì bỏ phần đó (đọc `--version` của
   image).

Chạy: job `upgrade` của `ci.yml` (ma trận `sqlite` / `postgres`, gọi `cargo xtask upgrade-test`) và một
bước trong `docs/RELEASING.md` trước khi gắn tag. **Không** vào `./gate.sh` (cần tải image, build bản
release, lâu) — ghi quyết định.

### B2. Migration thật: up → down → up

Test `crates/app/src/e2e/migrations.rs` với mọi module của `modules::all()` **cộng** `notes` (không đăng
ký mặc định) và module nội bộ `kernel`, trên cả hai database:

1. Database trống: `Migrator::up` → ảnh chụp schema S1.
2. Với từng module theo **thứ tự ngược** đăng ký: `Migrator::down(database, module, u32::MAX)`. Sau
   cùng chỉ còn bảng lịch sử `<module>_migrations` (sea-orm-migration không xoá chúng — kiểm thật rồi
   ghi vào test).
3. `up` lại → ảnh chụp S2 == S1.
4. Có dữ liệu: dựng app (`TestApp`), ghi dữ liệu mẫu qua REST (dùng lại bộ ghi mẫu của B1 nếu được),
   rồi với từng module: `down(1)` → `up` (migration mới nhất của module đi lại được khi có dữ liệu), và
   `down(u32::MAX)` → `up` không lỗi.

Ảnh chụp schema là API mới của testkit (phác thảo, cạnh `schema_ownership_violations` trong
`ownership.rs` hay file mới): `pub async fn schema_snapshot(connection: &DatabaseConnection) ->
Result<SchemaSnapshot, TestDatabaseError>` — bảng, cột (tên, kiểu, nullable, mặc định), index (tên, cột,
unique); hai nhánh truy vấn (`sqlite_master` + `pragma_table_info` / `pragma_index_list`;
`information_schema.columns` + `pg_indexes`), sắp xếp ổn định, `#[non_exhaustive]`, có `Debug` để diff
dễ đọc. Testkit là hợp đồng module (D-167): thêm, không đổi cái cũ.

### B3. Property test cho bộ kiểm đầu vào

Thư viện: `proptest` (MIT / Apache-2.0), chỉ ở `[dev-dependencies]` của crate cần; ghi `STACK.md`.
Mặc định 256 ca mỗi test (gate nhanh); CI có thể đặt `PROPTEST_CASES` lớn hơn. File
`proptest-regressions/` do proptest ghi: commit (ca hỏng cũ chạy lại mãi). Hàm thuần, không chạm
database, nên luật "cả hai database" không áp.

| Đích | Bất biến tối thiểu |
| --- | --- |
| `autoxide_kernel::email::{normalize, is_valid, checked}` | không panic; `normalize` idempotent; `is_valid(x)` ⇒ ≤ 254 byte, phần trước `@` 1–64 byte, tên miền có dấu chấm, nhãn 1–63 byte (D-174); `checked` trả `Some` đúng khi `is_valid(normalize(x))` |
| con trỏ pagination (`crates/kernel/src/pagination.rs`, `encode` / `decode`) | `decode(encode(c)) == c`; `decode` chuỗi bất kỳ không panic; con trỏ rác qua HTTP ra 422, không 500 (một test qua `TestApp`) |
| thân ProseMirror (`modules/content/src/entries/body.rs::check`) | JSON bất kỳ (giới hạn độ sâu) không panic; thân đã nhận thì kiểm lại vẫn nhận; `stored_image_ids` chỉ trả UUID có trong thân |
| slug (`entries/slug.rs`) | `from_title(chữ bất kỳ, id)` luôn `is_valid`; `with_suffix(slug hợp lệ, n)` hợp lệ |
| CSV (`modules/forms/src/csv.rs::write`) | đọc lại bằng bộ đọc RFC 4180 tối giản trong test ra đúng số dòng / ô; không ô nào (sau khi bỏ ngoặc kép) bắt đầu bằng `=`, `+`, `-`, `@`, tab, CR mà thiếu `'` |
| bình luận (`modules/comments/src/text.rs::{clean_body, to_html, escape, link_count}`) | HTML ra không có thẻ nào ngoài `<a …>`, `</a>`, `<br>`; mọi `<a` có `href` `http(s)://` và `rel="nofollow ugc noopener"`; `escape` không để lọt `<`, `>`, `&`, `"`, `'` thô; `link_count` = số `<a` |

Bước 32 thêm Markdown cho bình luận: bộ bất biến của dòng cuối áp cho cả định dạng mới (ghi nhắc trong
đặc tả bước 32).

### B4. Response thật khớp hợp đồng

Trong testkit: mỗi `TestRequest::send` (`crates/testkit/src/app.rs`) kiểm response với `TestApp::openapi()`:

1. Tìm operation theo method + đường dẫn (khớp mẫu `{…}` của `paths`). Không khớp (route lạ → 404) thì
   không kiểm.
2. Chọn response theo mã chính xác, không có thì `default`. Mã không có và không có `default` → lỗi.
3. Media type: `Content-Type` thật phải là một khoá trong `content` (bỏ `; charset=…`). Không có thân
   (204, 304, `HEAD`) → bỏ qua phần thân.
4. Thân JSON: kiểm theo schema, giải `$ref` trong `components`. Thân không phải JSON (CSV, nội dung
   media): chỉ kiểm media type.
5. Lệch → `send()` trả `Err(TestAppError)` với thông báo `GET /api/v1/x 200: $.data.items[0].id: expected
   string (uuid)` — mọi test HTTP đang có thành test hợp đồng mà không phải sửa.
6. Lối thoát cho test cố ý gửi thứ sai: `TestRequest::unchecked()` (phương thức mới).

Bộ kiểm JSON Schema: thư viện `jsonschema` (draft 2020-12, mà OpenAPI 3.1 dùng; tắt feature tải schema
qua mạng) hoặc tự viết cho tập con utoipa sinh ra — đo thời gian build và test của cả workspace rồi chọn,
ghi `STACK.md` nếu thêm.

**Lệch đã biết sẽ lộ ra ngay:** 779 response lỗi trong `docs/openapi.json` khai media type
`application/json`, nhưng lỗi thật trả `application/problem+json` (`crates/kernel/src/response.rs`,
`PROBLEM_CONTENT_TYPE`; response `default` đã khai đúng). Sửa ở kernel (`http/openapi.rs`: response có
schema `Problem` khai `application/problem+json`), sinh lại `docs/openapi.json` + SDK. `api-diff` đã coi
hai media type này là một cho lỗi (A1) — làm A1 trước B4.

## Thứ tự làm

| # | Commit | Mục ROADMAP |
| --- | --- | --- |
| 1 | kernel: `x-sunset` / `x-deprecated-since` / `x-successor` trong OpenAPI (+ test) | A.1 |
| 2 | `xtask api-diff` + test từng luật; `gate.sh`; `ci.yml` `fetch-depth: 0`; quyết định | A.1 |
| 3 | testkit: kiểm response theo hợp đồng + `unchecked()`; kernel: media type của lỗi; sinh lại `docs/openapi.json`; sửa lệch khác test tìm ra (mỗi loại lệch một commit) | B.4 |
| 4 | `xtask sdk` + `sdk/typescript/` + `--check` trong gate + job `sdk` trong CI; quyết định (D-177) | A.2 |
| 5 | `docs/guides/` (EN + VI) các tình huống; README trỏ tới | A.3 |
| 6 | `docs/guides/mcp.md` (EN + VI) + test bảng tool | A.4 |
| 7 | test ví dụ tài liệu (`e2e/guides.rs`), trích ví dụ TS cho CI | A.5 |
| 8 | testkit `schema_snapshot`; test migration up → down → up (trống và có dữ liệu); sửa `down` hỏng nếu có (migration mới, không sửa migration đã commit) | B.2 |
| 9 | `proptest`: từng nhóm đích một commit; sửa lỗi tìm ra (kèm ca hồi quy) | B.3 |
| 10 | test nâng cấp + `xtask upgrade-test` + job `upgrade` + bước trong RELEASING | B.1 |
| 11 | CHANGELOG, ROADMAP, phát hành (D-171) | — |

## Test bắt buộc

- `xtask`: test của `api_diff` (danh sách ở A1); test của bộ sinh SDK (schema mẫu → TypeScript mong đợi:
  nullable, enum, `oneOf`, `allOf`, mảng, map `additionalProperties`, tên có ký tự lạ); `sdk --check` đỏ
  khi file lệch.
- `crates/kernel`: `deprecation_ghi_x_sunset_vao_tai_lieu`; response lỗi khai `application/problem+json`.
- `crates/testkit`: `response_lech_hop_dong_thi_send_bao_loi`, `unchecked_bo_qua_kiem`,
  `schema_snapshot` trên cả hai database (bảng, cột, index, unique).
- `crates/app/src/e2e/guides.rs`: mọi ví dụ `curl` của `docs/guides/*.md` chạy được; khối code EN = VI;
  bảng tool MCP khớp `Modules::tools()`. Cả hai database.
- `crates/app/src/e2e/migrations.rs`: `moi_module_up_down_up_tren_database_trong`,
  `moi_module_down_mot_buoc_roi_up_khi_co_du_lieu`, `down_het_roi_up_khi_co_du_lieu`. Cả hai database.
- `crates/app/tests/upgrade.rs`: kịch bản B1 (CI và trước phát hành, cả hai database).
- Property test (B3) trong `crates/kernel`, `content`, `forms`, `comments`.
- Mọi test HTTP sẵn có xanh **với** kiểm hợp đồng bật.

## Tiêu chí xong

- [ ] `cargo xtask api-diff` trong gate (cả hai mức), test cho từng loại thay đổi; thông báo nêu cách
      làm đúng; `x-sunset` trong tài liệu khi có deprecation
- [ ] `sdk/typescript/` sinh từ `docs/openapi.json`, gate kiểm khớp, CI kiểm kiểu; gói `.tgz` theo bản
      phát hành; README SDK (EN + VI); quyết định cho công cụ sinh
- [ ] `docs/guides/` (EN + VI): website, xác thực, API key, gọi lại an toàn, pagination, lỗi, MCP; mỗi
      tình huống có `curl` và TypeScript; ví dụ chạy trong gate
- [ ] Test nâng cấp chạy trong CI (SQLite, Postgres) và có trong `docs/RELEASING.md`
- [ ] Migration mọi module up → down → up, trống và có dữ liệu, hai database
- [ ] Property test cho sáu nhóm đích; `proptest` trong `STACK.md`
- [ ] Mọi response trong test HTTP khớp `docs/openapi.json`; tài liệu lỗi khai đúng media type
- [ ] `./gate.sh` đầy đủ xanh; CHANGELOG (SDK, hướng dẫn, media type lỗi trong tài liệu, `x-sunset`);
      quyết định mới; ROADMAP `[x]`; phát hành theo `docs/RELEASING.md`

## Chỗ dễ sai

- **Shallow clone / thiếu tag** trong CI làm `git describe` hỏng hay so nhầm bản — kiểm và báo rõ, đừng
  im lặng qua.
- **Tag gắn ở commit `release:`** chứ không phải `HEAD` của `main` (RELEASING bước 6): `git describe`
  từ `HEAD` lấy đúng tag tổ tiên gần nhất — đừng dùng `git tag --sort` (lấy tag mới nhất kể cả không phải
  tổ tiên của nhánh đang làm).
- **Đổi tên component** của utoipa (vd generic `Page_X`) không được báo là phá vỡ — so theo hình dạng.
- **Nullable hai cách viết** và `allOf` bọc `$ref` (utoipa dùng khi có `description` cạnh `$ref`).
- **Lỗi media type đã biết** (B4) — làm `api-diff` trước, không thì sửa tài liệu lỗi bị chặn oan.
- **Ngày sunset:** so theo UTC, "≤ hôm nay"; operation mới deprecated mà sunset trong tương lai vẫn phải
  giữ.
- **Kiểm hợp đồng trong `send()`** có thể làm đỏ hàng loạt test sẵn có — mỗi lệch là lỗi thật (tài liệu
  hay code); sửa từng loại, đừng thêm `unchecked()` để cho qua.
- **Test nâng cấp:** container ghi file SQLite bằng uid khác máy (chạy `--user`); `--network host` chỉ
  có trên Linux; dừng bản cũ phải êm (SIGTERM) trước khi bản mới mở file; secret ký token phải giống nhau
  hai bản, không thì token cũ "hỏng" oan.
- **Migration down:** SQLite đổi bảng bằng cách tạo lại bảng — `down` của migration có `ALTER` cần
  kiểm thật; `down` tạo lại index unique có thể vỡ vì dữ liệu mà bản mới cho phép (vd WebP của bước 30:
  `down` phải xoá dòng WebP trước). Migration đã commit không sửa — `down` hỏng thì ghi quyết định và
  thêm migration mới, hoặc ghi ngoại lệ có lý do.
- **Ví dụ tài liệu** chứa secret giả: dùng `$TOKEN`, `axk_…` rút gọn, không chép token thật từ test ra
  tài liệu (gitleaks).
- `proptest` sinh chuỗi Unicode lạ: bộ kiểm đếm byte hay ký tự phải đúng như D-174 ghi.

## Căn cứ pháp lý

Không có yêu cầu pháp lý riêng. Ví dụ trong hướng dẫn không dùng dữ liệu cá nhân thật (Nghị định
13/2023/NĐ-CP về bảo vệ dữ liệu cá nhân — chỉ dùng `example.com`, tên giả).

## Agent làm bước này tự quyết

Ghi mỗi lựa chọn thành mục mới trong `docs/DECISIONS.md` (số kế tiếp lúc ghi):

- Công cụ sinh SDK và chỗ kiểm (khuyến nghị: Rust trong `xtask`, kiểm trong gate; `tsc` chỉ trong CI) —
  đây là quyết định D-177 giao.
- Luật ngoài danh sách của ROADMAP (`parameter.removed`, `request.constraint_tightened`,
  `response.status_removed`…) có bật không; cách xử lý enum mới trong response.
- Thư viện kiểm JSON Schema (đo rồi chọn).
- Test nâng cấp: integration test có điều kiện theo biến môi trường + lệnh `xtask`, ngoài `gate.sh`.
- Tên và cách chia file trong `docs/guides/`; dạng hẹp của ví dụ `curl`.
- Có kiểm hợp đồng cho MCP (tool `input_schema` / kết quả) không — ngoài ROADMAP; khuyến nghị để sau.
