# Bước 32 — Dọn bố cục và việc để lại

> Lộ trình: `docs/ROADMAP.md` bước 32 · Quy ước bố cục: `docs/MODULE_GUIDE.md` mục "File layout"
> (bước 28, commit `9e320c3` / `9456450`) · Quyết định: D-167 (hợp đồng module, giữ đường dẫn công
> khai), D-065 (đổi email — "báo địa chỉ cũ để sau"), D-156 / D-162 (mail qua job, thu hồi khi đổi mật
> khẩu), D-159 (`autoxide user reset-mfa`), D-112 (bình luận chữ thường, "Markdown + sanitizer để
> sau"), D-192 (thứ tự: `media` sau WebP của bước 30; bước 33 chờ nhóm A)

## Mục tiêu

Bốn module lớn (`newsletter`, `comments`, `media`, `forms`) và kernel đọc theo cùng một cách như
`content` / `auth`, để bước 33 (sửa mọi bảng của mọi module) và người viết module tìm code nhanh — không
đổi hành vi, không đổi đường dẫn công khai. Rồi đóng bốn việc nhật ký quyết định đã ghi "để sau": báo
địa chỉ email cũ khi tài khoản đổi email, quản trị viên gỡ passkey của người khác (và lệnh CLI cho admin
duy nhất), xuất CSV lớn bằng job, Markdown giới hạn cho bình luận.

## Bắt đầu khi

Ngay. Riêng dời file của `media` chờ WebP của bước 30 merge vào `main` (cùng sửa `variants.rs`,
`refresh.rs`, `jobs.rs`, `http/`). **Làm nhóm A trước nhóm B** (nhóm B sửa đúng các module nhóm A dời
file). Bước 33 bắt đầu khi nhóm A xong — merge nhóm A vào `main` ngay khi xong (gate đầy đủ xanh), đừng
giữ tới hết bước.

Phối hợp với bước 31: property test của bước 31 nằm cạnh hàm trong `comments/src/text.rs`,
`forms/src/csv.rs` — dời file thì test đi theo; bước 31 bật kiểm response theo hợp đồng trong
`TestApp` — endpoint mới của nhóm B phải khớp tài liệu ngay.

## Phạm vi

Sửa: `modules/{newsletter,comments,media,forms,auth}`, `crates/kernel/src/` (gom file),
`crates/app/src/commands/user.rs`, `crates/testkit` (`MemoryStorage` dùng chung), `.gitleaks.toml` (nếu
dời `antispam`), `xtask/src/boundaries.rs` (nếu dời `config`), `docs/MODULE_GUIDE.md` / `.vi.md`,
`docs/FORMS.md` / `.vi.md`, `docs/COMMENTS.md` / `.vi.md`, README (EN + VI, đoạn auth), `docs/openapi.json`,
`.env.example`, `STACK.md`, `CHANGELOG.md`, `docs/DECISIONS.md`, `docs/ROADMAP.md`.

Ngoài phạm vi: AVIF (chờ chủ dự án), đổi hành vi endpoint cũ, dời file của `auth` / `content` (đã theo
quy ước), cột `tenant_id` (bước 33).

## Thiết kế

### A1. Bố cục bốn module

Quy ước (MODULE_GUIDE): trên cùng giữ `lib.rs`, `api.rs` (chỉ `media` có), `config.rs`,
`permissions.rs`, `events.rs`, `entity/`, `migration/`, `http/`, `tools.rs`; service chia theo tính năng
— mỗi tính năng một file `<nhóm>.rs` khai `pub(crate) mod …;` cộng thư mục `<nhóm>/` (kiểu của `content`:
`entries.rs` + `entries/`); test dài thành `tests/` (mỗi tính năng một file, `tests/mod.rs` giữ đồ dùng
chung). Như `content`, được giữ ở trên cùng: `service.rs` (struct service dùng chung), `privacy.rs`, và
file trợ giúp dùng cho nhiều nhóm (vd `text.rs`).

Bố cục đề xuất (agent được chỉnh, ghi lý do trong quyết định):

| Module (dòng) | Nhóm → file hiện có | Test |
| --- | --- | --- |
| `newsletter` (9,7k) | `audience/`: `lists`, `subscribers`, `bounces` · `signup/`: `public` · `campaigns/`: `campaigns` (soạn), `sending`, `new_posts`, `compose` · trên cùng: `text`, `retention`, `privacy`, `service` | `tests/` đã có (giữ) |
| `comments` (8,7k) | `posting/`: `posting`, `authoring`, `targets` · `reading/`: `threads` · `moderation/`: `moderation`, `reports` · `notify.rs`, `retention.rs`, `text.rs`, `privacy.rs`, `service.rs` trên cùng | `tests/` đã có (giữ) |
| `media` (8k, **sau WebP**) | `library/`: `upload`, `sniff`, `folders`, `tags` · `images/`: `variants`, `refresh`, `alt` · `jobs.rs`, `service.rs` trên cùng | `tests.rs` (1.118 dòng) → `tests/` |
| `forms` (6,1k) | `definitions/`: `forms`, `fields` · `submissions/`: `public`, `inbox`, `csv`, `notify` · `retention.rs`, `privacy.rs`, `service.rs` trên cùng | `tests.rs` (904 dòng) → `tests/` (B3 thêm test xuất lớn) |

Cách làm, mỗi module một commit:

1. Trước khi dời: `cargo test -p autoxide-<m> -- --list --format terse 2>/dev/null | grep ': test$'`
   → lưu danh sách. So **số dòng** và **multiset tên lá** (phần sau `::` cuối — đường dẫn module của test
   đổi khi dời là bình thường). `cargo test -- --list` không chạy test, không cần database.
2. `git mv` (giữ lịch sử), sửa khai `mod`, sửa `use crate::…` (trình biên dịch chỉ chỗ), giữ
   `pub(crate)`; **không** đổi tên hàm, kiểu, mã lỗi, tên job / event / permission / tool.
3. Sửa chú thích đầu file kể tên file cùng cấp (vd `service.rs` của `newsletter`: "MỌI logic nằm ở đây và
   các file cùng cấp (`lists.rs`, `subscribers.rs`…)") — chú thích cũ thành sai.
4. Sau khi dời: cùng số test, cùng multiset tên lá; `docs/openapi.json` không đổi một byte (test sẵn có
   bắt); `cargo xtask check` xanh (luật handler chỉ nhìn `src/http/`, `src/tools.rs` — giữ hai chỗ này ở
   trên cùng).
5. CHANGELOG: một dòng "Internal: …" là đủ (người dùng không thấy gì) — hoặc không dòng, nếu agent quyết
   CHANGELOG chỉ cho thay đổi người dùng thấy (ROADMAP ghi "mỗi mục có dòng CHANGELOG": ghi dưới
   **Changed**, nói rõ không đổi hành vi).

### A2. Kernel: gom file phẳng thành nhóm, giữ đường dẫn công khai

`crates/kernel/src/` có 37 file phẳng; bước 33 sẽ thêm `tenancy`, `entitlement`, `payment`, `einvoice`,
`identity`, `sms`, `numbering`, `decimal` — nhóm phải có chỗ cho chúng. Đề xuất:

| Nhóm (riêng tư) | Module bên trong (đường dẫn công khai giữ nguyên `autoxide_kernel::<tên>`) | Bước 33+ thêm |
| --- | --- | --- |
| `contracts/` — trait module cung cấp / dùng qua `provide` / `require` | `ai`, `mail`, `storage`, `privacy`, `resource` | `payment`, `einvoice`, `identity`, `sms`, `entitlement` |
| `data/` — làm việc với database | `database`, `migration`, `pagination`, `rows`, `reference`, `version`, `search`, `transfer` | `tenancy`, `numbering` |
| `security/` | `access`, `secret`, `tokens`, `throttle`, `ip_hash`, `idempotency`, `tls`, `antispam/` | — |
| `ops/` — vận hành | `logging`, `metrics`, `telemetry`, `doctor`, `task`, `builtin` (riêng tư), `module_catalog` (riêng tư) | — |
| `values/` — kiểu giá trị | `money`, `email`, `public_url` | `decimal` |
| giữ trên cùng | `error`, `response`, `i18n`, `module` (+ `module/`), `config/`, `http/`, `event/`, `job/` | — |

Kỹ thuật (Rust): `lib.rs` khai `mod contracts;` (riêng tư) và `pub use contracts::{ai, mail, privacy,
resource, storage};` — re-export **module** giữ nguyên `autoxide_kernel::ai::TextModel` cho module ở repo
riêng (D-167) và giữ `crate::ai::…` bên trong kernel. Nhóm riêng tư nên không có đường thứ hai
(`autoxide_kernel::contracts::ai` không tồn tại) — "một cách cho mỗi việc". Module riêng tư (`builtin`,
`module_catalog`) dùng `pub(crate) use`.

Phải sửa theo khi dời (đã kiểm khi viết đặc tả):

- `include_str!("../locales/vi.json")` trong `i18n.rs` (nếu `i18n` dời) và
  `include_str!("fixtures/altcha_v2.json")` (đi theo thư mục `antispam/`).
- `.gitleaks.toml` có allowlist theo đường dẫn `crates/kernel/src/antispam/fixtures/altcha_v2.json` —
  dời `antispam` mà không sửa thì gitleaks trong gate đỏ.
- `xtask/src/boundaries.rs`: `ENV_ACCESS_FILE = "src/config/source.rs"` (nếu `config` dời — đề xuất
  giữ `config/` trên cùng).
- `super::` trong file bị dời giờ trỏ vào nhóm, không phải gốc crate.
- Liên kết doc nội bộ (`[`crate::module::Modules::new`]`) vẫn chạy qua re-export; chạy
  `cargo doc -p autoxide-kernel --no-deps` với `RUSTDOCFLAGS=-D warnings` để bắt link gãy.

Test mới giữ đường dẫn: `crates/kernel/tests/public_paths.rs` — `use` mọi module công khai hiện có
(`autoxide_kernel::{access, ai, antispam, config, database, doctor, email, error, event, http, i18n,
idempotency, ip_hash, job, logging, mail, mcp, metrics, migration, module, money, pagination, privacy,
public_url, reference, resource, response, rows, search, secret, storage, task, telemetry, throttle,
tls, tokens, transfer, version}`) và một kiểu tiêu biểu của mỗi module; mất đường dẫn nào là lỗi biên
dịch. Đếm test cả workspace trước / sau như A1. `lib.rs` thêm chú thích đầu file mô tả nhóm (module mới
của bước 33 vào nhóm nào). MODULE_GUIDE (EN + VI) không cần đổi nếu đường dẫn công khai không đổi.

### B1. `auth`: đổi email thì báo địa chỉ cũ

Hiện tại (`modules/auth/src/verify.rs`): `confirm(token)` đổi email trong một transaction, thu hồi token
đặt lại mật khẩu của địa chỉ cũ (D-156), ghi event `auth.email_verified` với `before.email`.

Thêm:

- Trong **cùng transaction** của `confirm`, khi `old != email`: xếp job `auth.send_email_changed`
  (`JobQueue::enqueue`, payload `{ user_id, old_email, new_email, language }` — payload bị xoá khi job
  xong, D-079). Request không chờ mail.
- Job: tạo token một lần qua `OneTimeTokens::new("auth")` mục đích `email_change_report`, `subject` =
  user id, `data` = địa chỉ cũ, sống 7 ngày (hằng số có lý do, hoặc Setting
  `AUTOXIDE_AUTH_EMAIL_CHANGE_REPORT_DAYS`); gửi mail tới **địa chỉ cũ**: tài khoản đổi sang địa chỉ mới
  (khuyến nghị che bớt: `n***@example.org`), lúc nào; nếu không phải bạn thì mở link / dùng token. Mẫu
  `auth.mail.email_changed_link` (khi đặt Setting mới `AUTOXIDE_AUTH_EMAIL_CHANGE_REPORT_URL` có
  `{token}`) và `auth.mail.email_changed_code` (token trong mail, như mẫu đặt lại mật khẩu). Bản dịch
  trong `modules/auth/locales/vi.json`.
- Endpoint mới `POST /api/v1/auth/email-change/report` (không cần đăng nhập; thân `{ "token": "…" }`;
  `ValidJson`; giới hạn lượt như `/password-reset`). Một transaction (`begin_write`, khoá dòng người dùng
  bằng `rows::lock_one` trước):
  1. `consume` token (sai / hết hạn / đã dùng → 400 `auth.invalid_email_change_report_token`);
  2. trả email về địa chỉ cũ **nếu** chưa ai khác lấy (index unique) — đã bị lấy thì giữ email hiện tại
     và vẫn làm các bước sau;
  3. thu hồi mọi phiên / refresh token, API key, passkey (`passkey::remove_all_in`), tắt TOTP + recovery
     code, xoá token xác minh email và token đặt lại đang chờ, đặt mật khẩu không dùng được — kẻ chiếm đã
     biết mật khẩu (đổi email cần mật khẩu) và có thể đã gắn MFA / passkey của nó;
  4. ghi event mới `auth.email_change_reported` (`before.email` = địa chỉ mới, `after.email` = địa chỉ
     sau khi xử lý);
  5. xếp job `auth.send_password_reset` (D-162) tới địa chỉ cũ.
  Trả 204. Khuyến nghị gom bước 3 vào hàm đã có cho đặt lại mật khẩu (`UserService::write_password` với
  `PasswordWrite::Recovery`, D-156) thay vì viết lại.
- Mã lỗi mới: `auth.invalid_email_change_report_token` — vi: "Mã báo đổi email không đúng, đã hết hạn hay
  đã dùng."
- Event mới khai qua `Module::events` (macro `events!` trong `modules/auth/src/events.rs`).
- Tài liệu: README (EN + VI) đoạn về email; mô tả OpenAPI của `POST /me/email` nói mail báo địa chỉ cũ.

### B2. `auth`: quản trị viên xem và gỡ passkey; lệnh CLI

- Permission mới `auth.passkeys.manage` ("See and remove the passkeys of other users. Give it only to
  people who handle account recovery.") trong `modules/auth/src/permissions.rs`, thêm vào `all()`.
- Endpoint mới (handler trong `modules/auth/src/http/passkeys.rs`, logic trong `passkey/service.rs`):
  - `GET /api/v1/auth/users/{id}/passkeys` — danh sách passkey của người đó (cùng dạng response như
    `GET /me/passkeys`; không bao giờ trả khoá công khai hay credential id thô nếu `/me` không trả).
  - `DELETE /api/v1/auth/users/{id}/passkeys/{passkey_id}` — gỡ một; 204.
  - `DELETE /api/v1/auth/users/{id}/passkeys` — gỡ hết; 204.
  Cả ba: `principal.require(&PASSKEYS_MANAGE)?`, rồi `state.roles.require_covers(&principal, id)` như
  `DELETE /users/{id}/mfa` (không tác động lên người quyền cao hơn, D-079). Lỗi: `auth.user_not_found`,
  `auth.passkey_not_found` (đã có, dùng lại), `permission_denied`,
  `auth.target_has_more_permissions`.
- Event: dùng lại `auth.passkey_removed` với actor là quản trị viên và thêm `after.by_administrator: true`
  (như `auth.mfa_disabled` của D-159). Audit tự ghi.
- Gỡ passkey không đổi mật khẩu, không đăng xuất phiên khác (giống `reset-mfa`) — ghi trong mô tả.
- CLI: `autoxide user remove-passkeys --email <email>` (`crates/app/src/commands/user.rs`, cạnh
  `reset-mfa`). Trait `autoxide_auth::api::AccountRecovery` thêm `async fn remove_passkeys(&self, email:
  &str) -> Result<(User, usize), Error>` (`Actor::system()`; `auth.account_not_found` khi không có tài
  khoản). In số passkey đã gỡ. `docs/DEPLOY.md` (EN + VI) cạnh `reset-mfa`: admin duy nhất mất thiết bị.
- Tool MCP: không thêm (việc khôi phục tài khoản không giao cho agent) — ghi trong quyết định.

### B3. `forms`: xuất CSV lớn bằng job

Giữ nguyên `GET /api/v1/forms/{id}/submissions/export` (đồng bộ) — **không** thêm giới hạn từ chối:
đó là thay đổi phá vỡ của `/api/v1` (D-059). Tài liệu khuyên dùng xuất qua job khi form có nhiều bài.

Bảng mới (migration `m0007_create_exports` của `forms`, có `down`):

| `forms_exports` | Kiểu (chạy cả hai database) |
| --- | --- |
| `id` | UUID, khoá chính (UUIDv7 sinh trong ứng dụng) |
| `form_id` | UUID (không foreign key xuyên module; cùng module thì theo cách `forms` đang làm) |
| `requested_by` | UUID, nullable (người dùng bị xoá → NULL) |
| `status_filter` | `string_len(16)`, nullable (như tham số `status` của export đồng bộ) |
| `state` | `string_len(16)`: `pending` / `running` / `ready` / `failed` |
| `row_count`, `size` | `big_integer`, nullable |
| `storage_key` | `string_len(255)`, nullable |
| `created_at`, `finished_at`, `expires_at` | timestamp UTC (`finished_at`, `expires_at` nullable) |

Index: (`form_id`, `id`) cho danh sách theo form; (`expires_at`) cho dọn dẹp.

- `POST /api/v1/forms/{id}/submissions/exports` (`forms.export`; thân `{ "status": "new" }` tuỳ chọn) →
  `Created` (201 + `Location: /api/v1/forms/exports/{export_id}`), `state: pending`. Ghi dòng + xếp job
  `forms.export_submissions` trong **cùng transaction**. Mỗi form tối đa một xuất đang chạy (unique theo
  logic, kiểm trong transaction ghi; trùng → 409 `forms.export_in_progress`, kèm id đang chạy).
- `GET /api/v1/forms/exports/{export_id}` (`forms.export`) → trạng thái, `row_count`, `size`,
  `download_url` (khi `ready`, chưa hết hạn), `expires_at`.
- `GET /api/v1/forms/exports/{export_id}/file?expires=<unix>&signature=<hex>` — không cần đăng nhập (link
  dán vào trình duyệt): chữ ký HMAC-SHA256 bằng khoá `SecretKey::derive("forms.export_download")` trên
  `export_id` + `expires`; so bằng hàm so sánh thời gian hằng; sai / hết hạn → 403
  `forms.export_link_invalid`; file đã xoá → 404 `forms.export_not_found`. `Content-Disposition:
  attachment`, `Cache-Control: no-store`, `text/csv; charset=utf-8`.
- Job: đọc theo trang 200 (như `export_csv` hiện có), ghi theo luồng vào `dyn Storage` (`Storage::put`
  nhận `ByteStream` — đừng dựng cả file trong bộ nhớ), khoá `forms/exports/<export_id>.csv`. Tách
  `csv::write` thành bộ ghi từng dòng dùng chung cho cả hai đường (cùng chặn CSV injection, cùng BOM, CRLF,
  ô trong ngoặc kép) — kết quả đồng bộ không đổi một byte (test so). Lỗi → `state = failed`.
- `forms` cần `dyn Storage`: `context.require::<dyn Storage>()` trong `start` (`files` đăng ký trước
  `forms` — đã đúng thứ tự ở `crates/app/src/modules.rs`). Test cần nơi lưu giả: chuyển `MemoryStorage`
  từ `modules/media/src/lib.rs` (mod `testing`) sang `autoxide_testkit` (API mới của testkit, công khai),
  `media` dùng lại.
- Setting mới: `AUTOXIDE_FORMS_EXPORT_LINK_MINUTES` (mặc định 60), `AUTOXIDE_FORMS_EXPORT_KEEP_HOURS`
  (mặc định 24). Sinh lại `.env.example`.
- Dọn: job `forms.cleanup` (`retention.rs`) xoá dòng quá `expires_at` và file của nó (xoá file trước, dòng
  sau; xoá file lỗi thì để dòng lại cho lần sau).
- Dữ liệu cá nhân: file xuất chứa email. `DELETE /api/v1/forms/data` (xoá theo email) và xoá tài khoản
  (`privacy.rs`) phải xoá mọi file xuất **của các form có bài bị xoá** (đơn giản và chắc: xoá mọi file xuất
  còn hạn của form đó). Ghi trong `docs/FORMS.md`.
- Event mới: `forms.submissions_exported` (subject là form; `after`: `export_id`, `row_count`; không có
  email) — audit biết ai lấy dữ liệu cá nhân ra.
- Mã lỗi mới (kèm vi): `forms.export_not_found` — "Không có bản xuất này, hoặc đã bị xoá.";
  `forms.export_in_progress` — "Form này đang có một bản xuất chưa xong."; `forms.export_link_invalid` —
  "Link tải không đúng hoặc đã hết hạn; hãy lấy link mới.".
- Tool MCP: không thêm (dữ liệu cá nhân hàng loạt) — ghi trong quyết định.
- `docs/FORMS.md` (EN + VI): mục xuất lớn.

### B4. `comments`: Markdown giới hạn

- Chữ cũ hiển thị như cũ: cột mới `format` trên `comments_comments` (migration `m0009_add_format`:
  `string_len(16)` NOT NULL DEFAULT `'text'`; `down` bỏ cột — kiểm `ALTER TABLE … DROP COLUMN` của
  sea-query trên SQLite bản đóng gói). Dòng cũ là `text`.
- Request tạo (`POST /api/v1/comments/public/comments`) và sửa (`PATCH …/{id}`) nhận `format`
  (`"text"` mặc định — client cũ gửi chữ có `*` không bị đổi nghĩa — hoặc `"markdown"`). Setting
  `AUTOXIDE_COMMENTS_MARKDOWN` (mặc định khuyến nghị `true`); tắt mà gửi `markdown` → 422 trường `format`.
  `GET …/public/settings` trả thêm `markdown: bool` để website biết hiện nút định dạng. Response có thêm
  `format`; `body_html` dựng theo `format`.
- Dựng HTML (`text.rs`): `pulldown-cmark` (tra phiên bản bằng `check-library-api`; tắt bảng, chú thích,
  gạch ngang…) và **bộ dựng theo danh sách cho phép** tự viết trên luồng event: chỉ ra `<p>`, `<br>`,
  `<strong>`, `<em>`, `<a href rel>`, `<blockquote>`, `<code>`, `<pre><code>`; mọi chữ đi qua `escape`
  sẵn có; HTML thô (`Event::Html`, `InlineHtml`) thành chữ đã escape; link chỉ `http://` / `https://` (như
  hiện nay), kèm `rel="nofollow ugc noopener"`, link khác thành chữ; ảnh thành chữ `alt` (không có `<img>`
  — chặn ảnh theo dõi); tiêu đề, danh sách, đường kẻ thành đoạn văn thường. Khi bộ dựng chỉ ra thẻ cố
  định thì không cần thêm `ammonia`; nếu chọn đường "Markdown → HTML → lọc" thì `ammonia` — đo kích
  thước binary và thời gian build, ghi `STACK.md` và quyết định.
- `link_count` đếm cả link Markdown (`[a](url)`) và địa chỉ trần — giới hạn link của chống spam không bị
  lách. Từ cấm so trên chữ gốc như cũ. Mail báo trả lời gửi chữ gốc.
- Tài liệu `docs/COMMENTS.md` (EN + VI): `format`, cú pháp được nhận, website vẫn chỉ chèn `body_html`.

## Thứ tự làm

| # | Commit | Mục ROADMAP |
| --- | --- | --- |
| 1 | `newsletter`: chia theo tính năng (đếm test trước / sau) | A.1 |
| 2 | `comments`: chia theo tính năng | A.1 |
| 3 | `forms`: chia theo tính năng, `tests.rs` → `tests/` | A.1 |
| 4 | `kernel`: gom nhóm, `pub use`, `public_paths.rs`, `.gitleaks.toml` nếu cần; quyết định | A.2 |
| 5 | `media` (sau khi WebP merge): chia theo tính năng, `tests.rs` → `tests/` | A.1 |
| — | gate đầy đủ → merge nhóm A vào `main` (mở đường cho bước 33) | — |
| 6 | testkit `MemoryStorage`; `forms`: bảng, job, ba endpoint, Setting, dọn, xoá theo email, tài liệu | B.3 |
| 7 | `auth`: job báo địa chỉ cũ + endpoint báo "không phải tôi", event, mẫu mail, tài liệu | B.1 |
| 8 | `auth`: permission + ba endpoint passkey của người khác; `AccountRecovery::remove_passkeys`; CLI; DEPLOY | B.2 |
| 9 | `comments`: cột `format`, bộ dựng Markdown, `link_count`, settings, tài liệu, STACK | B.4 |
| 10 | CHANGELOG, ROADMAP, phát hành (D-171) | — |

## Test bắt buộc

Mọi test chạm database chạy trên cả SQLite và Postgres (`TestDatabase::run`, HTTP qua
`TestApp::builder`).

- Nhóm A: số test và multiset tên lá trước = sau cho từng crate bị dời (ghi số vào thông điệp commit);
  `crates/kernel/tests/public_paths.rs`; test `docs/openapi.json` và `.env.example` sẵn có không đổi.
- B1: `doi_email_xep_mail_bao_dia_chi_cu` (job có trong cùng transaction — confirm rollback thì không có
  job); `bao_khong_phai_toi_thu_hoi_moi_thu_va_tra_email_cu` (phiên, refresh token, API key, passkey,
  TOTP đều hết; email trở lại; job đặt lại mật khẩu được xếp; event); `dia_chi_cu_da_bi_nguoi_khac_lay`
  (email giữ, vẫn khoá); token sai / hết hạn / dùng hai lần; **đồng thời**: hai request cùng token chỉ một
  thắng.
- B2: quản trị viên có / không có `auth.passkeys.manage`; người đích có quyền cao hơn → 403
  `auth.target_has_more_permissions`; gỡ một / gỡ hết; event `auth.passkey_removed` có
  `by_administrator`; CLI `user remove-passkeys` (như test `reset-mfa` hiện có); tài khoản không có.
- B3: xuất qua job ra file giống hệt từng byte bản đồng bộ cùng dữ liệu; link ký đúng tải được, sai chữ
  ký / hết hạn → 403, sau khi dọn → 404; hai yêu cầu xuất **đồng thời** cho một form → một 201, một 409;
  xoá theo email xoá file xuất; `forms.cleanup` xoá dòng và file quá hạn; job gặp lỗi nơi lưu → `failed`;
  không có `forms.export` → 403.
- B4: dòng `text` cũ (và mới mặc định) ra `body_html` **giống hệt** trước khi đổi (bộ mẫu cố định, so
  với đầu ra của hàm cũ); Markdown ra đúng thẻ cho phép; test XSS sẵn có (`xss_bi_chan`) giữ nguyên và
  thêm bản Markdown (`<script>`, `javascript:` trong link, HTML thô, link lồng, `[x](javascript:…)`,
  ảnh, thực thể HTML); `link_count` đếm link Markdown; tắt Setting → 422; migration up → down → up có dữ
  liệu. Property test của bước 31 cho `to_html` áp cả định dạng `markdown`.

## Tiêu chí xong

- [ ] Bốn module theo quy ước "File layout"; số test trước = sau; không đổi hành vi
- [ ] Kernel gom nhóm; mọi `autoxide_kernel::<tên>` cũ còn (test biên dịch); quyết định ghi nhóm cho
      module mới của bước 33
- [ ] B1–B4 xong như thiết kế, mỗi mục có test hai database và dòng CHANGELOG
- [ ] `docs/openapi.json`, `.env.example` sinh lại; tài liệu EN + VI (README / DEPLOY / FORMS / COMMENTS)
      sửa cùng commit; `STACK.md` nếu thêm thư viện
- [ ] `./gate.sh` đầy đủ xanh; quyết định mới; ROADMAP `[x]`; phát hành theo `docs/RELEASING.md`

## Chỗ dễ sai

- **Dời file mà đổi luôn tên / logic** — không; chỉ dời. Đổi gì khác thì commit riêng.
- **Đếm test bằng grep `#[test]`** sai (macro, test trong `mod` lồng) — dùng `cargo test -- --list`.
- **Kernel:** `include_str!` đường dẫn tương đối, allowlist đường dẫn của `.gitleaks.toml`,
  `ENV_ACCESS_FILE` của xtask, `super::` — bốn chỗ vỡ âm thầm hay vỡ ở bước khác của gate.
- **`media` trước WebP** gây xung đột lớn với bước 30 — chờ.
- **B1:** mail báo phải đi qua job xếp trong transaction của `confirm` (không gửi trực tiếp, không gửi
  khi transaction rollback); token "không phải tôi" gửi tới địa chỉ **cũ**, không phải mới; endpoint báo
  không được cho dò email (trả cùng lỗi cho mọi token sai); trả email cũ có thể đụng index unique.
- **B2:** thiếu `require_covers` thì người giữ `auth.passkeys.manage` khoá được admin.
- **B3:** link tải không có đăng nhập nên chữ ký phải gắn `export_id` **và** `expires`; so chữ ký thời
  gian hằng; job chạy lại (job queue retry) phải ghi đè cùng khoá và cập nhật dòng một cách idempotent;
  file chứa dữ liệu cá nhân — xoá theo email phải xoá file; `backup` chép cả `forms/exports/` (chấp nhận,
  ghi trong FORMS.md).
- **B4:** `pulldown-cmark` mặc định cho HTML thô đi qua — phải chặn; autolink `<http://…>` và link kiểu
  tham chiếu `[a][1]` cũng là link; `format` mặc định `text` để client cũ không đổi; `body_html` của dòng
  cũ phải giống từng byte.
- Mã lỗi mới thiếu bản dịch `locales/vi.json` thì `cargo xtask check` đỏ.

## Căn cứ pháp lý

- Xuất bài gửi chứa dữ liệu cá nhân, xoá theo yêu cầu chủ dữ liệu: Nghị định 13/2023/NĐ-CP về bảo vệ dữ
  liệu cá nhân (hiệu lực 01/7/2023) và Luật Bảo vệ dữ liệu cá nhân 2025 (hiệu lực 01/01/2026) — quyền xoá
  dữ liệu; file xuất phải xoá theo. Cần đối chiếu văn bản gốc cho thời hạn giữ file xuất (D-186).
- Báo chủ tài khoản khi email đổi: biện pháp bảo vệ tài khoản, không có điều khoản bắt buộc cụ thể —
  không cần căn cứ riêng.

## Agent làm bước này tự quyết

Ghi mỗi lựa chọn thành mục mới trong `docs/DECISIONS.md` (số kế tiếp lúc ghi):

- Bố cục cụ thể từng module (bảng A1 là đề xuất) và tên nhóm của kernel (bảng A2); nhóm cho module mới
  của bước 33.
- B1: thời hạn token báo (khuyến nghị 7 ngày), che địa chỉ mới trong mail, phạm vi thu hồi khi báo
  (khuyến nghị: như đặt lại mật khẩu + tắt TOTP + gỡ passkey).
- B2: một endpoint gỡ hết hay chỉ gỡ từng cái; không có tool MCP.
- B3: hai Setting và mặc định; xuất đang chạy trùng trả 409 hay trả bản đang chạy; không có tool MCP.
- B4: `pulldown-cmark` + bộ dựng tự viết hay thêm `ammonia` (đo rồi chọn); mặc định của
  `AUTOXIDE_COMMENTS_MARKDOWN`.
