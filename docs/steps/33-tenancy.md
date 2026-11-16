# Bước 33 — Nền đa khách: khách hàng, vị trí dữ liệu, mặt điều khiển

> Lộ trình: `docs/ROADMAP.md` bước 33 · Thiết kế: `docs/research/business-foundation.md` trụ 1, mục 6, mục 7 ·
> Quyết định: D-185, D-190, D-192 (đường nâng cấp), D-006 (phần bị D-185 thay), D-009, D-038, D-070,
> D-113, D-116, D-117, D-119, D-122, D-164, D-167, D-181, D-182, D-183, D-184

## Mục tiêu

Xong bước này, một bản cài Autoxide phục vụ được **nhiều khách hàng (tenant)** mà dữ liệu không bao giờ lẫn
nhau: mỗi request biết mình thuộc khách nào (tên miền, API key, access token, header), kernel chọn đúng
database của khách (chung `shared` hay riêng `dedicated`, SQLite một file mỗi khách hoặc Postgres), mọi bảng
cấp khách hàng của 13 module mang `tenant_id`, secret được mã hoá khi lưu, mỗi khách có cấu hình và secret
riêng, tên miền riêng có xác minh. Đây là nền cho cả bốn mô hình kinh doanh và sàn (D-185); bước 34–38 dựng
trên nó.

Người cài một cửa hàng **không thấy gì đổi**: chế độ `single` là mặc định, `/api/v1` không đổi (gate chặn
thay đổi phá vỡ của bước 31 xanh), bản cài cũ nâng cấp được — mọi dữ liệu có sẵn thuộc về khách mặc định.

## Bắt đầu khi

- Bước 31 xong: gate so `docs/openapi.json` với tag trước (chặn thay đổi phá vỡ), test nâng cấp từ bản phát
  hành trước, test migration up → down → up của mọi module trên cả hai database. Bước này dựa vào ba thứ đó
  để biết mình không làm vỡ bản cài cũ.
- Nhóm A của bước 32 xong (bố cục file của `newsletter`, `comments`, `media`, `forms`; nhóm file `kernel`).
- Có sẵn: `Database` (read/write/begin_write/write_with_retry/begin_snapshot), `rows`, `reference`,
  `SecretKey` (D-113), `Throttle` (D-116), `OneTimeTokens` (D-117), antispam nonce (D-119), `Outbox`,
  `JobQueue`/`Worker`/`Claim`, `Dispatcher`, `transfer::copy_modules` (D-070), `Migrator` (mỗi migration
  chạy trong transaction trên cả hai database — `crates/kernel/src/data/migration.rs`, `Transactional`).

## Phạm vi

**Kernel (`crates/kernel`)** — thêm nhóm `tenancy` (mới), sửa: `access.rs` (`Principal` mang khách),
`module.rs` (`StartContext::tenancy()`, hàm mới của `Module` có mặc định), `database.rs` (không đổi kiểu,
thêm `DatabaseRegistry` ở `tenancy`), `secret.rs` (mã hoá AEAD, khoá theo khách), `event/` (outbox mang
khách; dispatcher duyệt nhiều database), `job/` (job mang khách; worker duyệt nhiều database, chia lượt công
bằng), `throttle.rs`, `tokens.rs`, `antispam/`, `idempotency.rs`, `http/` (lớp giải ngữ cảnh khách, rate
limit, idempotency, CORS header mới), `mcp.rs`, `privacy.rs`, `resource.rs`, `mail.rs`, `ai.rs`,
`storage.rs`, `transfer.rs`, `migration.rs` (data upgrade), `builtin.rs` (bảng và API mặt điều khiển),
`doctor.rs`, `locales/vi.json`.

**Testkit (`crates/testkit`)** — `TestApp` dựng nhiều khách và nhiều database; `TestDatabase` cấp thêm
database; kiểm schema theo khách (cạnh `schema_ownership_violations`); bài rò rỉ giữa khách dùng chung.

**App (`crates/app`)** — `migrate up --all` / `status --all`, lệnh `tenant` và `placement`, `copy` mở rộng
(dời khách), `serve` (khách trên database chưa migrate thì 503, khách khác chạy), `doctor`, cấu hình mới.

**13 module** — `ai`, `audit`, `auth`, `comments`, `content`, `files`, `forms`, `idempotency`, `mail`,
`media`, `newsletter`, `notes`, `webhooks`: migration mới thêm `tenant_id`, entity, service lọc theo khách,
trait công bố nhận khách, thứ ngoài bảng (cache, khoá file, token ký) theo khách.

**`xtask`** — `cargo xtask check` chặn entity cấp khách hàng thiếu `tenant_id`; mẫu `cargo xtask new-module`
sinh module đã theo khách.

**Tài liệu** — `docs/MODULE_GUIDE.md` + `.vi.md` (phân loại bảng, lọc theo khách), `docs/DEPLOY.md` +
`.vi.md` (chế độ `multi`, placement, `AUTOXIDE_SECRET_KEY`, tên miền riêng, Caddy on-demand TLS),
`CHANGELOG.md`, `.env.example`, `docs/openapi.json`, SDK TypeScript của bước 31.

**NGOÀI phạm vi** (đừng làm ở đây):
- Cây tổ chức, `org_id`, `Scope` own/unit/subtree, danh tính hai lớp (nhân sự toàn nền tảng), API key gắn
  nút, quyền vào cấp có thời hạn của nhân sự nền tảng — bước 34. Bước 33 chỉ để sẵn cột `org_id` (NULL được)
  ở `kernel_tenant_domains`.
- Gói, `module_not_enabled`, hạn mức, `Entitlements` — bước 35. Cây khách hàng xuyên database, báo cáo gộp —
  bước 38 (bước 33 chỉ lưu `parent_id`).
- Backup theo khách, log/trace mang `tenant_id`, số đo theo khách — bước 39. Bước 33 chỉ cần `autoxide
  backup` ở chế độ `multi` **cảnh báo** những placement nó không chép.
- Bộ nối đối tác (cổng thanh toán, hoá đơn điện tử): chưa có; bước 33 dựng kho cấu hình theo khách để chúng
  dùng sau.

## Thiết kế

### Quyết định nền (ghi thành mục mới trong `docs/DECISIONS.md` bằng skill `record-decision`)

1. **Ngữ cảnh khách là tham số tường minh, không ẩn.** Service không giữ `Database` nữa: mọi hàm service
   chạm database nhận `tenant: &TenantContext`, và `TenantContext` là **nguồn duy nhất** của `Database`
   (`tenant.database()`). Quên khách thì không có database để truy vấn — trình biên dịch bắt. Đã cân nhắc:
   `tokio::task_local!` đặt khách ngầm cho mọi truy vấn — ít sửa chữ ký nhưng mất khi `tokio::spawn`, khó đọc
   cho agent, lỗi chỉ lộ lúc chạy.
2. **Mọi database có cùng một schema** (mọi module, kể cả bảng mặt điều khiển của `kernel`). Bảng mặt điều
   khiển chỉ có dữ liệu ở database điều khiển (`DATABASE_URL`); ở database dữ liệu chúng rỗng và `doctor`
   kiểm chúng rỗng. Một `Migrator` cho mọi database → `migrate --all`, `copy`, dời khách đơn giản.
3. **Khách mặc định có ID cố định** `tenancy::DEFAULT_TENANT` = `00000000-0000-7000-8000-000000000001`
   (đúng dạng UUIDv7, sinh trong ứng dụng dưới dạng hằng số): migration của từng module gán dữ liệu cũ cho
   nó mà không phải đọc bảng của `kernel`. Khách mặc định cũng là **khách nền tảng** (platform tenant) — nơi
   tài khoản của nhà vận hành sống. Ghi đây là ngoại lệ có chủ đích của luật "UUIDv7 sinh trong ứng dụng".
4. **Hằng `tenancy::NO_TENANT`** = UUID toàn số 0, chỉ dùng cho job của cả database (dọn dẹp theo lịch) trong
   bảng `<m>_jobs`. Không dùng NULL: unique index coi NULL là khác nhau trên cả hai database, nên
   `(tenant_id, unique_key)` sẽ không chống trùng job theo lịch.
5. **Nâng bảng bằng cách dựng lại bảng** (create mới → chép → drop cũ → rename), một helper của kernel, một
   đường code cho cả hai database. Lý do: SQLite không bỏ được `DEFAULT` hay đổi PRIMARY KEY bằng `ALTER
   TABLE`; để `DEFAULT <khách mặc định>` lại thì mọi lệnh `INSERT` quên `tenant_id` âm thầm ghi vào khách mặc
   định — đúng loại rò rỉ ta đang chặn. Chi phí: chép toàn bảng một lần khi nâng cấp (ghi trong tài liệu).
6. **Chế độ `multi` bị khoá tới khi mọi module chuyển xong**: hằng `tenancy::MULTI_TENANT_READY` (kernel)
   là `false` tới commit cuối của phần chuyển module; khi `false`, `AUTOXIDE_TENANCY_MODE=multi` là lỗi cấu
   hình nói rõ lý do. Không ai bật được `multi` trên bản đang chuyển dở. (Test vẫn bật được qua testkit.)
7. **Đổi chữ ký trait và hàm kernel là thay đổi phá vỡ hợp đồng module** (D-167): cho phép ở bản minor khi
   còn `0.x`, ghi **Changed** trong `CHANGELOG.md`. Thứ bị **bỏ** (hàm chuyển tiếp) phải `#[deprecated]` ít
   nhất một bản phát hành trước khi xoá — đánh dấu ở commit cuối của bước này (lúc không còn ai gọi), xoá ở
   đầu bước 34. Không dùng `#[allow(deprecated)]` để lách lint.

### Kiểu và trait (phác thảo — tra API thật bằng skill `check-library-api`, để `cargo check` xác nhận)

```rust
// autoxide_kernel::tenancy — API mới
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub struct TenantId(Uuid);              // DeriveValueType của SeaORM nếu dùng thẳng trong entity
pub struct OrgId(Uuid);                 // khai sẵn cho bước 34, chưa dùng
pub struct PlacementId(Uuid);
pub const DEFAULT_TENANT: TenantId;     // 00000000-0000-7000-8000-000000000001
pub const NO_TENANT: TenantId;          // nil — chỉ cho job của cả database
pub const MULTI_TENANT_READY: bool;

pub enum TenancyMode { Single, Multi }  // AUTOXIDE_TENANCY_MODE
pub enum TenantStatus { Provisioning, Active, Suspended, Maintenance, Moving }
pub enum PlacementKind { Shared, Dedicated }

/// Khách của request / job / event. Clone rẻ (Arc bên trong). So sánh bằng `id`.
#[derive(Clone)]
pub struct TenantContext { /* id, slug, status, placement, Database, is_platform */ }
impl TenantContext {
    pub fn id(&self) -> TenantId;
    pub fn database(&self) -> &Database;      // database của placement chứa khách
    pub fn is_platform(&self) -> bool;        // khách nền tảng (DEFAULT_TENANT)
    pub fn storage_prefix(&self) -> String;   // "" cho khách mặc định, "tenants/<id>/" cho khách khác
}
// Extractor axum: lấy từ extensions do lớp HTTP đặt; thiếu → `tenant_required`.
impl<S> FromRequestParts<S> for TenantContext { … }

/// Sổ database: database điều khiển + placement mở lười, có giới hạn, đóng bớt theo LRU.
#[derive(Clone)]
pub struct DatabaseRegistry { … }
impl DatabaseRegistry {
    pub fn mode(&self) -> TenancyMode;
    pub fn control(&self) -> &Database;
    pub async fn tenant(&self, id: TenantId) -> Result<TenantContext, Error>;   // cache có TTL
    pub async fn tenant_by_slug(&self, slug: &str) -> Result<TenantContext, Error>;
    pub async fn tenant_by_domain(&self, host: &str) -> Result<Option<TenantContext>, Error>;
    pub async fn databases(&self) -> Result<Vec<(PlacementId, Database)>, Error>; // cho worker, dispatcher
    pub fn invalidate(&self, id: TenantId);                                      // sau khi sửa khách
}

/// Lọc theo khách cho SeaORM: entity cấp khách hàng cài trait này (một dòng).
pub trait TenantScoped: EntityTrait { fn tenant_column() -> Self::Column; }
pub trait ScopeExt { fn scoped(self, tenant: &TenantContext) -> Self; }
// cài cho Select<E>, SelectTwo…, UpdateMany<E>, DeleteMany<E>, UpdateOne<A> (thêm WHERE tenant_id = ?)
// Ví dụ: Entity::find_by_id(id).scoped(&tenant).one(tenant.database().read())
```

`StartContext::tenancy() -> DatabaseRegistry` (mới). `StartContext::database()` giữ lại cho chỗ thật sự
của cả database (vd `SecretKey` đọc khoá gốc ở database điều khiển); hàm service không được dùng nó.

**Chữ ký đổi** (một commit cơ học — xem "Thứ tự làm"):

| Chỗ | Trước | Sau |
|---|---|---|
| `access::Principal` | `user_id`, grants, `api_key` | thêm `tenant_id()`, `operator_from() -> Option<TenantId>`; hàm tạo nhận `TenantId` |
| `access::Authenticator::authenticate` | `(authorization)` | `(authorization, hint: &TenantHint)` — `auth` tự đọc khách trong credential |
| `event::Outbox::record` | `(conn, actor, subject, event)` | `(conn, &TenantContext, actor, subject, event)` |
| `event::Delivered` | — | thêm field `tenant: TenantContext` (struct đã `#[non_exhaustive]`) |
| `job::JobQueue::enqueue` | `(conn, job, opts)` | `(conn, &TenantContext, job, opts)`; job của cả database dùng `enqueue_database(conn, job, opts)` |
| `job::JobContext` | — | thêm `tenant: Option<TenantContext>` (`None` = job của cả database) và `database: Database` |
| `throttle::Throttle`, `tokens::OneTimeTokens`, antispam nonce | khoá chữ | nhận `&TenantContext`, ghi/lọc `tenant_id` |
| `idempotency::IdempotencyStore::begin/complete/abandon` | — | nhận `&TenantContext` |
| `privacy::PersonalData::export` | `(user_id)` | `(&TenantContext, user_id)` |
| `resource::ResourceResolver::resolve` | `(id)` | `(&TenantContext, id)` |
| `mcp::ToolHandler::call` | `(principal, args)` | `(TenantContext, principal, args)` |
| `mail::Mailer::send` | `(mail)` | `(&TenantContext, mail)` — SMTP theo khách |
| `ai::TextModel::generate`, `ai::Embedder::embed` | — | thêm `&TenantContext` — khoá AI theo khách |
| `storage::Storage` | khoá thô | giữ nguyên trait; thêm `TenantStorage` (bọc, thêm `storage_prefix()`, chặn `..`) |
| `secret::SecretKey` | `derive(purpose)` | thêm `derive_for(tenant, purpose)` — khoá ký riêng mỗi khách |
| trait trong `api` của module (`auth::api::{Users, Accounts, AccountRecovery}`, `media::api::Media`, `notes::api::Notes`) | — | mọi hàm nhận `&TenantContext` |

**Hàm mới của `Module` (có thân mặc định, D-038):**
- `fn control_plane_tables(&self) -> Vec<&'static str>` — bảng mặt điều khiển của module (mặc định rỗng).
- `fn tenant_settings(&self) -> Vec<TenantSetting>` — cấu hình / secret theo khách module nhận.
- `fn seed_tenant(&self) -> Option<Arc<dyn TenantSeed>>` — gieo dữ liệu ban đầu cho khách mới (vd role
  `admin`, dòng khoá `media_lock`, đầu chuỗi `audit_chain`); phải chạy lại được (idempotent).
- `fn upgrades(&self) -> Vec<Box<dyn DataUpgrade>>` — nâng cấp dữ liệu cần cấu hình (vd mã hoá secret cũ),
  ghi vào cùng bảng `<m>_migrations` với tên `u0001_…`, chạy bởi `migrate up` sau migration schema; `serve`
  từ chối chạy khi còn upgrade chờ (như migration).

**Mã hoá secret (`autoxide_kernel::secret`, API mới):**

```rust
pub struct Sealer { /* khoá hiện tại + khoá cũ, theo key id */ }
impl Sealer {
    /// "sealed:v1:<key_id>:<base64url(nonce || ciphertext)>"; AAD = purpose + tenant + định danh dòng.
    pub fn seal(&self, tenant: TenantId, purpose: &str, row: &str, plaintext: &[u8]) -> Result<String, Error>;
    pub fn open(&self, tenant: TenantId, purpose: &str, row: &str, sealed: &str) -> Result<Zeroizing<Vec<u8>>, Error>;
    pub fn needs_reseal(&self, sealed: &str) -> bool;     // khoá cũ hay chữ thường cũ
}
```
- Thuật toán: AES-256-GCM qua `ring::aead` (ring đã có trong cây phụ thuộc qua `rustls`/`object_store` —
  không thêm thư viện; nonce 96 bit ngẫu nhiên, số lần mã hoá mỗi khoá nhỏ). Nếu kiểm phiên bản ghim thấy
  không dùng được, chọn `chacha20poly1305` (XChaCha20-Poly1305) và ghi `STACK.md`.
- Khoá mã hoá = `SecretKey::derive("autoxide/kernel/seal/v1")` của khoá gốc; `key_id` = 8 ký tự hex đầu của
  SHA-256 khoá đó (đây là "số phiên bản khoá" của ROADMAP). Xoay khoá: đặt khoá mới vào `AUTOXIDE_SECRET_KEY`,
  khoá cũ vào `AUTOXIDE_SECRET_KEY_PREVIOUS` (danh sách, cách nhau dấu phẩy), chạy `autoxide secrets reseal`
  (chạy mọi task `<module>.reseal_secrets`), rồi bỏ khoá cũ. Khoá gốc đang nằm trong `kernel_secrets` (khi chưa
  đặt `AUTOXIDE_SECRET_KEY`) luôn được thử như một khoá cũ — để lần đầu đặt biến môi trường không làm mất secret.
- Giá trị không có tiền tố `sealed:` là chữ thường cũ: đọc được (chỉ trong lúc chuyển tiếp), `doctor` báo
  WARN cho tới khi upgrade/reseal chạy.

### Phân loại bảng (đọc từ migration thật ngày viết đặc tả; 100 bảng + `notes`)

Ký hiệu: **C** mặt điều khiển (không có `tenant_id`), **H** lịch sử migration của từng database (không có
`tenant_id`), **T** cấp khách hàng (thêm `tenant_id` NOT NULL). PK `id` (UUIDv7, duy nhất toàn cục) giữ nguyên
— đó là ngoại lệ duy nhất của luật "khoá unique bắt đầu bằng `tenant_id`". Index phục vụ việc quét của cả
database (hết hạn, hàng đợi, giữ lại theo thời hạn) **không** bắt đầu bằng `tenant_id` và phải có tên kết thúc
`_sweep_idx`; mọi index khác bắt đầu bằng `tenant_id`.

**Bảng sinh từ helper của kernel, có ở nhiều module** (một helper nâng cho mọi module):

| Bảng | Có ở module | Khoá mới |
|---|---|---|
| `<m>_events` (outbox) | auth, comments, content, files, forms, mail, media, newsletter, notes, webhooks (+ `kernel_events` mới) | T; `pending_idx` → `<m>_events_pending_sweep_idx(delivered_at, failed_at, next_attempt_at)` |
| `<m>_jobs` | ai, auth, comments, content, files, forms, idempotency, mail, media, newsletter, webhooks (+ `kernel_jobs` mới) | T (cho phép `NO_TENANT`); unique `(tenant_id, unique_key)`; `(tenant_id, completed_at, failed_at, run_at)` + `<m>_jobs_pending_sweep_idx(completed_at, failed_at, run_at)` |
| `<m>_throttle` | auth, comments, forms, newsletter | T; PK `(tenant_id, key)`; `window_sweep_idx` |
| `<m>_tokens` | auth, comments, newsletter | T; unique `(tenant_id, token_hash)`; `(tenant_id, purpose, subject, created_at)`; `expires_sweep_idx` |
| `<m>_antispam_nonces` | comments, forms, newsletter | T; PK `(tenant_id, nonce_hash)`; `expires_sweep_idx` |
| `<m>_migrations` | mọi module | H |

**Bảng riêng của từng module:**

| Module | Bảng | Loại | Khoá unique / PK phải đổi (index thường thêm `tenant_id` đầu) |
|---|---|---|---|
| kernel | `kernel_secrets` | C | giữ (khoá ký gốc của bản cài) |
| kernel (mới) | `kernel_tenants`, `kernel_tenant_domains`, `kernel_placements` | C | xem "Bảng dữ liệu" |
| kernel (mới) | `kernel_tenant_settings` | T | PK `(tenant_id, name)` |
| ai | `ai_usage` | T | `(day, purpose)` → `(tenant_id, day, purpose)` |
| audit | `audit_entry` | T | `sequence` → `(tenant_id, sequence)`; module/action/subject/actor → `(tenant_id, …, id)` |
| audit | `audit_chain` | T | PK `name` → `(tenant_id, name)`; mỗi khách một chuỗi, gieo đầu chuỗi bằng `seed_tenant` |
| auth | `auth_user` | T | `email` → `(tenant_id, email)` |
| auth | `auth_role` | T | `name` → `(tenant_id, name)`; role `admin` gieo cho mỗi khách |
| auth | `auth_role_permission`, `auth_user_role`, `auth_api_key_permission` | T | `(role_id, permission)`, `(user_id, role_id)`, `(api_key_id, permission)` → thêm `tenant_id` đầu |
| auth | `auth_api_key` | T | `key_hash` → `(tenant_id, key_hash)` |
| auth | `auth_refresh_token`, `auth_mfa_challenge` | T | `token_hash` → `(tenant_id, token_hash)`; `expires`/`revoked` → `_sweep_idx` |
| auth | `auth_totp` | T | `user_id` → `(tenant_id, user_id)`; cột `secret` → mã hoá |
| auth | `auth_recovery_code`, `auth_passkey_ceremony` | T | index thêm `tenant_id`; `expires` → `_sweep_idx` |
| auth | `auth_passkey` | T | `credential_id` → `(tenant_id, credential_id)` |
| comments | `comments_comments`, `comments_bans`, `comments_reports`, `comments_reply_subscriptions` | T | `(comment_id, reporter_key)`, `(comment_id, email)` → thêm `tenant_id` đầu; `created`/`retention` → `_sweep_idx` |
| comments | `comments_profiles` | T | PK `user_id` → `(tenant_id, user_id)`; `name_key` → `(tenant_id, name_key)` |
| comments | `comments_targets` | T | PK `(target_type, target_id)` → `(tenant_id, target_type, target_id)` |
| content | `content_entries` | T | slug `(locale, kind, parent_key, slug)` và `(translation_group_id, locale)` → thêm `tenant_id` đầu |
| content | `content_published` | T | PK `entry_id` giữ; `(locale, path)` → `(tenant_id, locale, path)` |
| content | `content_authors`, `content_menus`, `content_redirects`, `content_term_translations` | T | `slug`, `(key, locale)`, `(locale, from_path)`, `(taxonomy, locale, slug)` → thêm `tenant_id` đầu |
| content | `content_tree_locks` | T | PK `locale` → `(tenant_id, locale)` |
| content | `content_translation_requests`, `content_embeddings`, `content_embedding_state` | T | `active_key`, `(entry_id, scope, chunk_index)`, `(scope, locale)` → thêm `tenant_id` đầu |
| content | `content_revisions`, `content_schedules`, `content_terms`, `content_entry_terms`, `content_entry_authors`, `content_menu_items` | T | không có unique riêng; `schedules_due_idx` → `_sweep_idx` |
| files | `files_file` | T | `(owner_id, id)` → `(tenant_id, owner_id, id)` |
| forms | `forms_forms`, `forms_form_versions`, `forms_submissions` | T | `key` → `(tenant_id, key)`; `(form_id, version)` → thêm `tenant_id`; `created`/`retention` → `_sweep_idx` |
| idempotency | `idempotency_record` | T | `(scope, key)` → `(tenant_id, scope, key)`; `expires` → `_sweep_idx` |
| mail | (chỉ `mail_events`, `mail_jobs`) | T | như bảng helper |
| menus | `menus_menus` | T | `(key, locale)` → `(tenant_id, key, locale)` (bước 34 thêm `org_id` sau `tenant_id`, D-238) |
| menus | `menus_items` | T | `(menu_id, position)` → thêm `tenant_id` đầu |
| media | `media_folder` | T | `(parent_id, name_key)` → `(tenant_id, parent_id, name_key)` (giữ nguyên ngữ nghĩa NULL của thư mục gốc) |
| media | `media_tag`, `media_variant` | T | `name_key` → `(tenant_id, name_key)`; `(item_id, version_id, width)` → thêm `tenant_id` |
| media | `media_lock` | T | PK `name` → `(tenant_id, name)`; dòng `folder_tree` gieo mỗi khách |
| media | `media_item`, `media_item_tag` | T | `trashed` → `_sweep_idx` |
| newsletter | `newsletter_lists`, `newsletter_subscribers`, `newsletter_suppressions` | T | `key`, `email`, `email` → thêm `tenant_id` đầu |
| newsletter | `newsletter_campaigns`, `newsletter_deliveries` | T | `(source, source_ref)`, `(campaign_id, subscriber_id)`, `unsubscribe_token_hash` → thêm `tenant_id` đầu |
| newsletter | `newsletter_subscriptions` | T | PK `(subscriber_id, list_id)` giữ (UUID) |
| notes | `notes_note` | T | `(owner_id, title)` → `(tenant_id, owner_id, title)` |
| webhooks | `webhooks_endpoint` | T | cột `secret` → mã hoá |
| webhooks | `webhooks_delivery` | T | `(endpoint_id, event_id)` → `(tenant_id, endpoint_id, event_id)` |

Foreign key **trong** một module (auth → `auth_user`, `auth_role`, `auth_api_key`; webhooks delivery →
endpoint) được giữ, trỏ theo `id`.

### Bảng dữ liệu (mới)

Kiểu dùng helper `schema::*` sẵn có (UUID, chữ có độ dài, `timestamp_with_time_zone`, boolean, integer) —
chạy trên cả hai database. Mọi bảng mới có `down`.

- `kernel_tenants` (C): `id` uuid PK, `slug` varchar(63) (chữ thường, số, `-`), `name` varchar(200),
  `status` varchar(16), `parent_id` uuid NULL (khách cha, bước 38 dùng; không FK), `placement_id` uuid,
  `created_at`, `updated_at`. Unique `kernel_tenants_slug_unique(slug)`; index `(parent_id)`,
  `(placement_id)`. Migration gieo khách mặc định: `id = DEFAULT_TENANT`, `slug = "default"`, `status =
  active`, `placement_id = MAIN_PLACEMENT`.
- `kernel_placements` (C): `id` uuid PK, `name` varchar(63) unique, `kind` (`shared` / `dedicated`),
  `backend` (`sqlite` / `postgres`), `url_sealed` text NULL (URL kết nối, mã hoá; NULL = `DATABASE_URL`),
  `read_url_sealed` text NULL, `storage_sealed` text NULL (bucket riêng cho khách `dedicated`),
  `schema_state` (`ready` / `behind` / `migrating`), `schema_checked_at` NULL, `created_at`, `updated_at`.
  Gieo placement `main` (id cố định `tenancy::MAIN_PLACEMENT`, url NULL).
- `kernel_tenant_domains` (C): `id` uuid PK, `tenant_id` uuid, `org_id` uuid NULL (bước 34), `domain`
  varchar(253) (chữ thường, dạng punycode, không cổng), `status` (`pending` / `verified` / `failed` /
  `revoked`), `verification_token` varchar(64), `verified_at` NULL, `last_checked_at` NULL, `check_error`
  varchar(200) NULL, `created_at`, `updated_at`. Unique `kernel_tenant_domains_domain_unique(domain)` (toàn
  nền tảng — một tên miền chỉ thuộc một khách); index `(tenant_id, id)`; `(status, last_checked_at)` sweep.
- `kernel_tenant_settings` (T): `tenant_id`, `name` varchar(100) (`<module>.<khoá>`), `value` text (chữ
  thường với cấu hình thường, `sealed:…` với secret), `secret` bool, `updated_by` uuid NULL, `updated_at`.
  PK `(tenant_id, name)`.
- `kernel_events`, `kernel_jobs`: tạo bằng `outbox_migration` / `jobs_migration` (đã theo khách).

**Helper nâng bảng** (`autoxide_kernel::tenancy::migration`, API mới):

```rust
pub fn adopt(name: &'static str, module: &'static str, tables: Vec<TenantTable>) -> Box<dyn MigrationTrait>;
TenantTable::new("auth_user")
    .unique("auth_user_email_unique", ["tenant_id", "email"])
    .index("auth_user_…_idx", ["tenant_id", …])
    .sweep_index("auth_refresh_token_expires_sweep_idx", ["expires_at"])
    .primary_key(["tenant_id", "key"])          // chỉ khi PK đổi
    .drop_index("…")                            // bỏ hẳn một index cũ (phải nói rõ)
```
`up` trong MỘT transaction (Migrator đã bọc): (1) đọc cột (tên, kiểu khai báo nguyên văn, NULL, DEFAULT),
PK, FK, index của bảng cũ từ danh mục của database (SQLite `pragma table_info` / `index_list` /
`foreign_key_list`; Postgres `information_schema` + `pg_catalog`, như `testkit/src/ownership.rs` và
`transfer.rs` đang làm); (2) tạo `<bảng>__t` với `tenant_id` (kiểu UUID của `schema::uuid`) NOT NULL **không
DEFAULT**, cùng cột, PK mới; FK trỏ tới `<cha>__t` khi cha cùng đợt; (3) `INSERT INTO <bảng>__t (tenant_id,
…) SELECT <DEFAULT_TENANT>, … FROM <bảng>` (cha trước con), so số dòng; (4) drop bảng cũ (con trước cha);
(5) rename `__t` → tên cũ (SQLite ≥ 3.26 tự sửa tham chiếu FK; Postgres theo OID), Postgres đổi lại tên
constraint PK `<bảng>__t_pkey` → `<bảng>_pkey`; (6) tạo index đã khai. Helper **từ chối** chạy nếu một index cũ
không được khai lại hay `drop_index` (không mất index âm thầm), nếu PK mới không chứa `tenant_id` (trừ PK
`["id"]`). `down`: dựng lại bảng không có `tenant_id`; **từ chối** (lỗi nói rõ số dòng) nếu còn dữ liệu của
khách khác `DEFAULT_TENANT` — down không được làm mất dữ liệu.

### Ngữ cảnh khách cho request

Lớp HTTP mới `tenancy::resolve` (ngoài rate limiting, trong `client::attach` để đọc được địa chỉ và proxy tin
cậy) đặt `TenantHint` vào extensions; `Principal` và `TenantContext` đọc nó. Thứ tự:

1. **Chế độ `single`**: luôn khách mặc định; bỏ qua Host và header; không truy vấn gì thêm (đường nóng như cũ).
2. **Có credential** (`Authorization`): khách lấy **từ chính credential** — access token có claim `tid`; API
   key, refresh token, token dùng một lần do `auth` cấp sau bước này mang ID khách trong chuỗi (định dạng do
   `auth` chọn, kernel không đọc) để định tuyến tới đúng database **trước** khi tra bản băm; bản tra xong vẫn
   kiểm `tenant_id` của dòng. Credential cấp trước bước này (không mang khách) thuộc khách mặc định.
3. **Header `Autoxide-Tenant: <slug | id>`**:
   - có credential của khách nền tảng và permission `kernel.tenants.operate` → vào khách đó;
     `Principal::operator_from()` = khách nền tảng; quyền giữ nguyên tên permission đã có; ghi log
     `tenancy entered` (user_id, tenant_id). Audit có thời hạn / lý do / khách xem được là bước 34;
   - có credential của khách khác và header trỏ khách khác → `tenant_mismatch` (403);
   - không credential → chọn khách cho route công khai (dữ liệu công khai vốn công khai; giao diện chạy trên
     tên miền nền tảng cần cách này).
4. **Host** (hay `Forwarded` / `X-Forwarded-Host` chỉ khi đến từ proxy tin cậy — dùng cấu hình
   `client.rs` sẵn có) khớp tên miền `verified` → khách đó. Host khớp khách khác khách của credential →
   `tenant_mismatch`.
5. Không xác định được → handler cần `TenantContext` nhận `tenant_required` (400). Route không cần khách
   (health, `openapi.json`, `/api/v1/kernel/domains/allowed`) không dùng extractor.

Sau khi biết khách: `Suspended` → `tenant_suspended` (403) cho mọi route trừ đọc thông tin khách hiện tại;
`Maintenance`/`Moving` hay placement `schema_state != ready` → `tenant_maintenance` (503, `Retry-After: 30`).
MCP (`/mcp`) đi qua cùng lớp; tool nhận `TenantContext` như REST. Thêm `Autoxide-Tenant` vào CORS
`allow_headers`; OpenAPI khai header này là tham số tuỳ chọn ở tài liệu chung (thêm tham số tuỳ chọn không phá
vỡ — gate bước 31 cho qua).

Cache: tra khách theo ID / slug / tên miền đi qua cache trong bộ nhớ có TTL
(`AUTOXIDE_TENANCY_CACHE_SECONDS`, mặc định 10); sửa khách trên cùng máy chủ thì `invalidate` ngay; máy chủ
khác thấy sau tối đa TTL (ghi trong tài liệu: tạm khoá khách có hiệu lực trong ≤ TTL giây).

### Sổ kết nối, job, event, migration trên nhiều database

- `DatabaseRegistry` mở pool của placement lần đầu cần, tối đa `AUTOXIDE_TENANCY_MAX_OPEN_DATABASES` (mặc định
  32) pool cùng lúc, bỏ pool ít dùng nhất khỏi sổ (pool tự đóng khi không còn bản clone nào). Pool của
  placement nhỏ: `AUTOXIDE_TENANCY_WRITE_CONNECTIONS` (mặc định 2), `AUTOXIDE_TENANCY_READ_CONNECTIONS` (4).
  SQLite `dedicated`: file `<AUTOXIDE_TENANCY_SQLITE_DIR>/<tenant_id>.db` (mặc định thư mục `tenants/` cạnh
  file của `DATABASE_URL`), tạo khi cấp phát.
- Mở pool lần đầu thì so migration đã áp dụng với `Migrator`: thiếu → `schema_state = behind`, khách trên đó
  nhận `tenant_maintenance`, khách nơi khác chạy bình thường.
- Worker và dispatcher duyệt `registry.databases()` (lọc theo `AUTOXIDE_WORKER_PLACEMENTS`, mặc định `all`);
  mỗi database dùng `Claim` sẵn có (Postgres `SKIP LOCKED`, SQLite `BEGIN IMMEDIATE`) nên nhiều máy chủ chạy
  cùng lúc an toàn. `Outbox` đánh thức dispatcher kèm placement.
- **Chia lượt công bằng**: mỗi lượt, với mỗi database × module, lấy các khách đang có job tới hạn (dùng
  `_pending_sweep_idx`), chia `concurrency` đều cho họ (ít nhất 1 mỗi khách, xoay vòng điểm bắt đầu giữa các
  lượt), claim theo `(tenant_id, …)`. Thêm tham số khách vào `Claim::claim`.
- Lịch (`Schedule`) chạy **mỗi database một lần** với `NO_TENANT` (job dọn dẹp quét cả database bằng
  `_sweep_idx`); job cần theo khách tự đưa việc con vào hàng đợi theo khách.
- `autoxide migrate up --all` (và `status --all`): database điều khiển trước, rồi mỗi placement theo thứ tự
  tên; mỗi placement: đặt `schema_state = migrating`, `Migrator::up`, data upgrade, đặt `ready`. Hỏng thì dừng
  với lỗi nêu placement; chạy lại tiếp từ chỗ dừng (bảng `<m>_migrations` của từng database đã ghi phần xong).
  `migrate up` không `--all` ở chế độ `multi` chỉ chạy database điều khiển và in danh sách placement còn chờ.
- `serve`: database điều khiển phải đủ migration (như nay); placement thiếu thì chỉ khách trên nó 503.

### Cấp phát và dời khách

- Tạo khách (`TenantService::create`, REST/CLI): ghi `kernel_tenants` `Provisioning` → nếu `dedicated` tạo
  placement (SQLite: tạo file; Postgres: người vận hành đưa URL của database trống) → migrate placement → gọi
  `seed_tenant` của mọi module theo thứ tự đăng ký → `Active`. Hỏng giữa chừng: giữ `Provisioning`, chạy lại
  bằng `autoxide tenant provision <slug>` (mọi bước idempotent).
- Dời `shared` ↔ `dedicated` (`autoxide tenant move <slug> --to <placement>`): đặt `Moving` (khách nhận 503) →
  chờ job đang claim của khách hết hạn claim → hai database phải cùng phiên bản schema → chép mọi dòng
  `tenant_id = X` của mọi bảng cấp khách hàng (mở rộng `transfer.rs`: `copy_tenant(source, target, tenant,
  tables)`, cùng cách đọc/ghi theo kiểu cột, theo thứ tự tạo bảng, một transaction ở đích) → so số dòng từng
  bảng → đổi `placement_id` → xoá dòng ở nguồn (trừ khi `--keep-source`) → `Active`, `invalidate`. File lưu trữ:
  chép object có tiền tố khách nếu placement đích có bucket riêng. Outbox và job chưa chạy đi theo dữ liệu.

### Thứ ngoài bảng

| Thứ | Cách làm |
|---|---|
| Khoá file lưu trữ | `TenantStorage` thêm `storage_prefix()`: khách mặc định không tiền tố (file cũ không phải dời), khách khác `tenants/<id>/`; placement `dedicated` có `storage_sealed` thì dùng bucket riêng. Job dọn file mồ côi của `files`/`media` liệt kê theo tiền tố từng khách |
| Cache trong bộ nhớ | `content::ai::semantic::SearchCache`: khoá `(TenantId, Scope, locale)`, cache câu hỏi `(TenantId, text)`, `public_budget` theo khách; rà mọi `HashMap`/`Mutex` cache khác bằng grep `Mutex<HashMap` trong `modules/` |
| Rate limiting | lớp HTTP: khoá theo `(tenant, IP)` khi đã biết khách; theo API key giữ (ID toàn cục) |
| Login throttling | `auth_throttle` có `tenant_id` (qua `Throttle` đã theo khách) |
| Idempotency key | `scope` băm thêm `tenant_id`; bản ghi ở database của khách |
| Con trỏ pagination | dữ liệu đã lọc theo khách nên con trỏ của khách khác không lộ gì; test chứng minh (con trỏ của A dùng ở B chỉ trả dữ liệu B). Không đổi định dạng |
| Token và URL đã ký | khoá ký `SecretKey::derive_for(tenant, purpose)`: token form antispam, preview của `content` (`publishing/preview.rs`), JWT của `auth` (thêm `tid`, kiểm khớp khách của request); dùng ở khách khác → bị từ chối như token sai |
| Link trong email | URL công khai theo khách: cấu hình khách `kernel.public_url` (khai qua `tenant_settings`), rơi về `AUTOXIDE_HTTP_PUBLIC_URL` |

### Cấu hình và secret theo khách

- Module khai `TenantSetting { name: "mail.smtp_url", description, secret: true }`; kernel khai
  `kernel.public_url`. Đọc qua trait kernel `TenantSettings` (công bố bởi `kernel`, lấy bằng
  `context.require::<dyn TenantSettings>()`): `get(&tenant, &SETTING) -> Result<Option<SecretString | String>>`.
  Không đặt → module dùng cấu hình chung (biến môi trường) như nay.
- Áp ngay ở bước này: `mail` (`mail.smtp_url`, `mail.from`) — giữ transport theo khách trong cache có giới
  hạn; `ai` (`ai.provider`, `ai.api_key`, `ai.model`). Cổng thanh toán, hoá đơn điện tử dùng cùng cơ chế ở bước 36.
- Giá trị secret không bao giờ trả ra API, log hay event; event `kernel.setting_changed` chỉ mang tên.

### API REST (module `kernel`, dưới `/api/v1/kernel`; mọi handler có `///` tiếng Anh và `description` từng response)

| Method + path | Permission | Ghi chú |
|---|---|---|
| `GET /tenant` | đăng nhập | khách hiện tại (id, slug, name, status, mode) — giao diện cần |
| `GET /tenants` | `kernel.tenants.read` + khách nền tảng | `Page<Tenant>`, lọc `status`, `parent_id` |
| `POST /tenants` | `kernel.tenants.manage` + nền tảng | `{slug, name, parent_id?, placement?}` → `Created`; `Idempotency-Key`; chế độ `single` → `tenant_single_mode` |
| `GET /tenants/{id}` | `kernel.tenants.read` + nền tảng | |
| `PATCH /tenants/{id}` | `kernel.tenants.manage` + nền tảng | `name`, `status` (`active`/`suspended`); `If-Match` (D-084) |
| `GET /placements` | `kernel.placements.read` + nền tảng | không bao giờ trả URL; tạo placement chỉ bằng CLI (URL chứa mật khẩu) |
| `GET /domains`, `POST /domains` | `kernel.domains.manage` | tên miền của khách hiện tại; tạo trả bản ghi TXT phải đặt |
| `POST /domains/{id}/verify` | `kernel.domains.manage` | tra DNS ngay, trả trạng thái mới |
| `DELETE /domains/{id}` | `kernel.domains.manage` | `revoked` |
| `GET /domains/allowed?domain=` | công khai | 200 khi tên miền `verified` và khách `active`, không thì 404 — cho `ask` của Caddy on-demand TLS; có rate limit |
| `GET /settings` | `kernel.settings.manage` | mọi setting đã khai + `is_set` + `updated_at`; không trả giá trị secret |
| `PUT /settings/{name}`, `DELETE /settings/{name}` | `kernel.settings.manage` | `ValidJson<{value}>` |

"+ nền tảng": `principal.tenant_id()` phải là khách nền tảng **và không** đang vào khách khác qua header —
role `admin` của khách thường có mọi permission, nên chỉ kiểm permission là lỗ hổng (`tenant_platform_only`).
Xác minh tên miền: bản ghi TXT `_autoxide-challenge.<domain>` = `autoxide-verify=<verification_token>`; tra
qua trait `DnsTxtLookup` (cài thật bằng `hickory-resolver` — thư viện mới, ghi `STACK.md`; test dùng bản giả).
Không cho thêm tên miền là IP, `localhost`, tên miền của chính nền tảng (`AUTOXIDE_HTTP_PUBLIC_URL`).

### Tool MCP · Event · Job/lịch · Cấu hình · Permission · Mã lỗi

- **Tool MCP** (gọi cùng service): `kernel.tenant.current`, `kernel.tenants.list`, `kernel.tenants.get`,
  `kernel.domains.list`, `kernel.domains.add`, `kernel.domains.verify`, `kernel.settings.list` (không có tool
  ghi secret).
- **Event** (`kernel`): `kernel.tenant_created`, `kernel.tenant_updated`, `kernel.tenant_moved`,
  `kernel.domain_added`, `kernel.domain_verified`, `kernel.domain_revoked`, `kernel.setting_changed`.
- **Job / lịch**: `kernel.verify_domains` (mỗi 10 phút, tra lại tên miền `pending` tới 7 ngày rồi `failed`;
  tra lại `verified` mỗi ngày, mất TXT 3 lần liên tiếp → `failed`), `kernel.check_placements` (mỗi phút, cập
  nhật `schema_state`).
- **Setting mới** (khai bằng `Setting`, sinh lại `.env.example`): `AUTOXIDE_TENANCY_MODE` (`single`),
  `AUTOXIDE_TENANCY_CACHE_SECONDS` (10), `AUTOXIDE_TENANCY_MAX_OPEN_DATABASES` (32),
  `AUTOXIDE_TENANCY_WRITE_CONNECTIONS` (2), `AUTOXIDE_TENANCY_READ_CONNECTIONS` (4),
  `AUTOXIDE_TENANCY_SQLITE_DIR`, `AUTOXIDE_WORKER_PLACEMENTS` (`all`), `AUTOXIDE_SECRET_KEY_PREVIOUS` (secret).
- **Permission** (`kernel`): `kernel.tenants.read`, `kernel.tenants.manage`, `kernel.tenants.operate`,
  `kernel.placements.read`, `kernel.domains.manage`, `kernel.settings.manage`.
- **Mã lỗi** (kernel, `crates/kernel/locales/vi.json`):

| code | HTTP | Tiếng Việt |
|---|---|---|
| `tenant_required` | 400 | Không xác định được khách hàng của yêu cầu. Gửi tới tên miền của khách, kèm header `Autoxide-Tenant`, hoặc đăng nhập. |
| `tenant_not_found` | 404 | Không có khách hàng `{tenant}`. |
| `tenant_suspended` | 403 | Khách hàng này đang bị tạm khoá. Liên hệ nhà cung cấp dịch vụ. |
| `tenant_maintenance` | 503 | Dữ liệu của khách hàng này đang được bảo trì. Thử lại sau {seconds} giây. |
| `tenant_mismatch` | 403 | Thông tin đăng nhập thuộc khách hàng khác với khách hàng của yêu cầu này. |
| `tenant_platform_only` | 403 | Chỉ quản trị của nền tảng mới làm được việc này. |
| `tenant_single_mode` | 409 | Bản cài đang chạy chế độ một khách hàng; đặt `AUTOXIDE_TENANCY_MODE=multi` để thêm khách. |
| `tenant_slug_taken` | 409 | Tên rút gọn `{slug}` đã có khách hàng khác dùng. |
| `tenant_domain_invalid` | 422 | `{domain}` không phải tên miền dùng được (không nhận IP, localhost hay tên miền của nền tảng). |
| `tenant_domain_taken` | 409 | Tên miền `{domain}` đã thuộc một khách hàng khác. |
| `tenant_domain_not_found` | 404 | Không có tên miền này trong khách hàng hiện tại. |
| `tenant_placement_not_found` | 404 | Không có vị trí dữ liệu `{placement}`. |
| `tenant_setting_unknown` | 422 | Không module nào nhận cấu hình `{name}`. |

## Thứ tự làm

Mỗi dòng là một commit (hoặc vài commit nhỏ), `./gate.sh --quick` xanh trước mỗi commit, gate đầy đủ xanh
ở cuối mỗi phần. Trong lúc chuyển: testkit có danh sách tạm `PENDING_TENANCY` (bảng chưa chuyển); mỗi commit
module **xoá** bảng của mình khỏi danh sách; commit cuối xoá cả danh sách.

**Phần A — nền kernel (chế độ `single` không đổi hành vi):**
1. `tenancy`: kiểu, hằng, `TenancyMode`, setting, bảng `kernel_tenants`/`kernel_placements`/`kernel_tenant_domains`
   + gieo khách mặc định + placement `main`, `DatabaseRegistry` (single + multi, mở lười, LRU), mã lỗi + bản dịch.
2. Ngữ cảnh request: lớp `tenancy::resolve`, extractor `TenantContext`, `Principal` mang khách,
   `Authenticator(…, hint)`, header + CORS, MCP; testkit: `TestApp` nhiều khách, `TestDatabase` cấp thêm database.
3. Helper `tenancy::migration::adopt` + test trên bảng mẫu (có dữ liệu, có FK, PK đổi; up → down → up; down từ
   chối khi có khách khác).
4. Commit cơ học đổi chữ ký (bảng "Chữ ký đổi"): module chưa chuyển truyền ngữ cảnh khách mặc định qua hàm
   chuyển tiếp `TenantContext::legacy_default(&database)` (`#[doc(hidden)]`). Cùng commit: migration nâng bảng
   helper (`<m>_events`, `<m>_jobs`, `<m>_throttle`, `<m>_tokens`, `<m>_antispam_nonces`) cho **mọi** module
   (thêm một dòng vào `migration/mod.rs` của từng module), `Outbox`/`JobQueue`/`Throttle`/`OneTimeTokens`/nonce
   ghi `tenant_id`; worker, dispatcher duyệt nhiều database, chia lượt công bằng.
5. `TenantScoped`/`ScopeExt`; testkit `schema_tenancy_violations` (bảng T thiếu `tenant_id` NOT NULL, unique/PK
   không bắt đầu bằng `tenant_id`, index thường không bắt đầu bằng `tenant_id` mà tên không `_sweep_idx`, bảng
   C/H ngoài danh sách khai) chạy trong `crates/app/src/modules.rs` cạnh bài sở hữu bảng; bài rò rỉ dùng chung
   (xem "Test"); `cargo xtask check` chặn entity thiếu `tenant_id` (trừ bảng module khai ở `control_plane_tables`).
6. `Sealer` (AEAD, key id, khoá cũ), `Module::upgrades` + `DataUpgrade`, `autoxide secrets reseal`; `doctor` và
   log khởi động nói rõ khi `AUTOXIDE_SECRET_KEY` chưa đặt (khoá gốc trong database, backup không được bảo vệ).
7. `kernel_tenant_settings`, `Module::tenant_settings`, trait `TenantSettings`, REST/MCP `settings`.
8. Thứ ngoài bảng ở kernel: rate limit, idempotency scope, `derive_for`, token antispam, `TenantStorage`.
9. `migrate up --all` / `status --all`, `schema_state`, `tenant_maintenance`, `serve`; `Module::seed_tenant`,
   `TenantService::create` + CLI `tenant create|list|show|suspend|resume|provision`, `placement add|list|check`.

**Phần B — chuyển module, mỗi module một commit** (migration `adopt` cho bảng riêng, entity thêm `tenant_id` +
`impl TenantScoped`, service nhận `&TenantContext` và dùng `.scoped(&tenant)` ở MỌI truy vấn, `INSERT` đặt
`tenant_id`, trait `api` nhận khách, `seed_tenant`, thứ ngoài bảng của module, test rò rỉ của module). Nhỏ
trước, module được dùng trait trước module dùng nó:
1. `notes` (mẫu — làm kỹ, các module sau chép cách làm) · 2. `idempotency` · 3. `mail` (SMTP theo khách) ·
4. `ai` (`ai_usage`, khoá theo khách) · 5. `audit` (chuỗi theo khách, `doctor audit.chain` theo từng khách) ·
6. `files` (`TenantStorage`, dọn mồ côi theo tiền tố) · 7. `webhooks` (mã hoá `secret` + upgrade; chỉ gửi tới
endpoint **cùng khách** với event) · 8. `forms` · 9. `media` (`media_lock` gieo theo khách) · 10. `newsletter` ·
11. `comments` · 12. `auth` (email/role/key theo khách, credential mang khách, `tid` trong JWT, mã hoá TOTP +
upgrade, gieo `admin`, CLI `user … --tenant`) · 13. `content` (cache tìm theo ý nghĩa, preview token, route
công khai theo Host / header).
14. Commit đóng phần B: xoá `PENDING_TENANCY`, `#[deprecated]` cho `legacy_default` và hàm chuyển tiếp khác
(không còn ai gọi), `MULTI_TENANT_READY = true`, `cargo xtask new-module` sinh module theo khách.

**Phần C — tên miền, vận hành nhiều database, đo, tài liệu:**
1. Tên miền riêng: bảng đã có từ A1; REST/MCP domains, `DnsTxtLookup`, job `kernel.verify_domains`, endpoint
   `allowed`; `docs/DEPLOY.md` + `.vi.md` (Caddyfile mẫu `on_demand_tls { ask … }`).
2. Dời khách: `copy_tenant`, `autoxide tenant move`; `autoxide copy` cũ giữ nguyên hành vi cho cả database.
3. `autoxide backup` ở `multi` cảnh báo placement không chép (bước 39 làm đủ).
4. Đo: `scripts/loadtest.sh` trên commit trước phần A (đã đo sẵn, lưu số) và sau phần C, cả SQLite và Postgres;
   thêm bài đo `multi` (100 khách chung + 10 khách SQLite riêng). Ghi số vào quyết định.
5. `docs/MODULE_GUIDE.md` + `.vi.md`: mục phân loại bảng, `TenantContext`, `.scoped`, `seed_tenant`,
   `tenant_settings`, `_sweep_idx`, bài rò rỉ; `CHANGELOG.md` (Added + **Changed** liệt kê mọi chữ ký đổi);
   `docs/DECISIONS.md`; đánh dấu ROADMAP; phát hành theo `docs/RELEASING.md`.

**Ánh xạ mục ROADMAP → commit:** mặt điều khiển → A1, A9; ngữ cảnh khách → A2, A9, B12, B13; phân loại bảng →
A3, A4, A5, B1–B14; lọc theo khách + bài rò rỉ → A5, B1–B14; job/event/outbox/lịch, `migrate --all`, dời khách
→ A4, A9, C2; đo + hướng dẫn module → C4, C5; mã hoá → A6, B7, B12; cấu hình/secret theo khách → A7, B3, B4;
thứ ngoài bảng → A8, B6, B12, B13; tên miền riêng → C1; an toàn nhiều database → A4 (công bằng), A9.

## Test bắt buộc

Mọi test chạy trên SQLite và Postgres (gate đầy đủ), lấy database qua `TestDatabase::run`, app qua `TestApp::builder`.

**Testkit (API mới cần thêm, phác thảo):**
```rust
TestApp::builder(&test)
    .multi_tenant()                         // AUTOXIDE_TENANCY_MODE=multi (bỏ qua MULTI_TENANT_READY)
    .tenant("a")                            // placement chung
    .tenant_dedicated("b")                  // database riêng: file SQLite khác / database Postgres khác
    .user_in("a", "token-a", 1, &["notes.write"])
    .build().await?;
app.get("/api/v1/notes").token("token-a"); app.get(…).host("a.test"); app.get(…).header("Autoxide-Tenant", "b");
app.tenant("b") -> TenantContext;  app.run_jobs().await;  app.dispatch_events().await;   // mọi database
test.extra_database().await -> TestDatabase                // dọn khi test xong
autoxide_testkit::tenancy::leak_check(&app, "b-token", MARKER).await -> Vec<String>
```
`leak_check`: duyệt **mọi** operation `GET` trong OpenAPI của app (thay `{tham số}` bằng mọi ID đã gieo ở khách
A theo tên tham số) và mọi tool MCP `read_only`, gọi bằng người của khách B có **mọi** permission; vi phạm khi
response 2xx có chứa chuỗi `MARKER` (gieo trong tiêu đề, nội dung, email, tên file… ở khách A) hay một ID của A.
Nên có thêm (agent quyết): bộ canh SQL trong test — callback số đo của kết nối SeaORM (tra API
`set_metric_callback` của bản ghim) báo câu `SELECT`/`UPDATE`/`DELETE` chạm bảng T mà không có `tenant_id`.

**Kernel:** `adopt_giu_du_lieu_va_gan_khach_mac_dinh`, `adopt_tu_choi_khi_quen_index_cu`,
`adopt_down_tu_choi_khi_con_khach_khac`, `adopt_doi_pk_va_giu_fk` · `single_bo_qua_host_va_header` ·
`credential_thang_header_va_host_khac_khach_thi_tenant_mismatch` · `header_cua_nhan_su_nen_tang_vao_khach_khac`
· `admin_khach_thuong_khong_quan_ly_duoc_khach_khac` (`tenant_platform_only`) · `khach_bi_khoa_nhan_403` ·
`placement_chua_migrate_chi_khach_tren_do_503` · `registry_mo_luoi_va_gioi_han_so_pool` ·
`worker_chia_luot_cong_bang` (khách A 1000 job, B 1 job: job của B chạy trong lượt đầu) ·
`lich_chay_mot_lan_moi_database` · `dispatcher_giao_event_tren_moi_database` ·
`job_trung_unique_key_khac_khach_deu_chay` · `seal_open_va_xoay_khoa` (khoá cũ đọc được, `reseal` chuyển sang
khoá mới, AAD khác khách/dòng thì mở thất bại) · `khoa_trong_database_van_mo_duoc_sau_khi_dat_bien_moi_truong` ·
`upgrade_du_lieu_chay_mot_lan_va_serve_tu_choi_khi_con_cho` · `migrate_all_chay_lai_tu_cho_dung` ·
`tao_khach_goi_seed_cua_moi_module_va_chay_lai_duoc` · `doi_khach_shared_sang_dedicated_va_nguoc_lai` (đếm dòng
từng bảng; nguồn sạch; khách khác không bị đụng) · `ten_mien_xac_minh_bang_txt` (DNS giả) ·
`allowed_chi_tra_200_cho_ten_mien_verified_cua_khach_active` · `setting_secret_khong_bao_gio_tra_ra` ·
`rate_limit_va_idempotency_key_khong_dung_chung_giua_khach` · `token_ky_cua_khach_a_bi_tu_choi_o_khach_b` ·
`con_tro_cua_khach_a_o_khach_b_chi_tra_du_lieu_b` · `tenant_context_dong_thoi` (nhiều request song song của
nhiều khách trên hai placement không lẫn database). · `flag_cua_khach_nay_khong_lo_sang_khach_khac` (bảng
`kernel_user_flags` của D-207 thêm `tenant_id`; cùng `user_id` và `key` ở hai khách là hai dòng).

**Mỗi module (B1–B13):** `khong_ro_ri_giua_khach` (gieo qua REST ở khách A, `leak_check` bằng khách B; một
khách trên placement chung và một khách `dedicated`) · `trung_ten_o_hai_khach_duoc_phep` (cùng email / slug /
key / title ở hai khách đều tạo được; trong một khách vẫn bị chặn) · `nang_cap_gan_du_lieu_cu_cho_khach_mac_dinh`
· `event_va_job_chay_dung_khach` · test đồng thời sẵn có chạy lại trong khách không mặc định. Riêng: `webhooks`
`event_khach_a_khong_gui_toi_endpoint_khach_b`; `audit` `chuoi_rieng_moi_khach_va_doctor_kiem_tung_khach`;
`auth` `email_trung_o_hai_khach_dang_nhap_dung_khach`, `api_key_dinh_tuyen_dung_database`,
`totp_luu_ma_hoa_va_ma_cu_doc_duoc`; `content` `cache_tim_theo_y_nghia_khong_dung_chung`,
`trang_cong_khai_theo_host`; `files`/`media` `khoa_file_co_tien_to_khach`.

**App:** test nâng cấp của bước 31 xanh (bản trước → bản này, dữ liệu đọc lại được); test sở hữu bảng + kiểm schema
theo khách; e2e mọi module trên `multi` với hai khách. Đề xuất cho gate: lượt SQLite chạy bộ test như hiện nay
(`single`), lượt Postgres đặt `TEST_TENANCY=multi` để `TestApp` mặc định đưa người dùng giả vào một khách không
mặc định trên placement riêng — phủ chế độ `multi` cho mọi test sẵn có mà không gấp đôi thời gian (agent quyết).

## Tiêu chí xong

- [ ] Mọi mục `- [ ]` của bước 33 trong ROADMAP có commit và test tương ứng (bảng ánh xạ ở trên).
- [ ] `./gate.sh` đầy đủ xanh (SQLite + Postgres); test nâng cấp từ bản phát hành trước xanh; gate chặn thay
      đổi phá vỡ của `/api/v1` xanh (chỉ thêm endpoint, header tuỳ chọn).
- [ ] Kiểm schema theo khách không còn ngoại lệ tạm (`PENDING_TENANCY` đã xoá); `cargo xtask check` xanh.
- [ ] `leak_check` chạy cho cả 13 module và e2e của app, trên placement chung và riêng.
- [ ] `auth_totp.secret`, `webhooks_endpoint.secret`, `kernel_tenant_settings` (secret), `kernel_placements.url_sealed`
      không còn chữ thường sau `migrate up` (test đọc thẳng database); `doctor` cảnh báo đúng khi chưa đặt
      `AUTOXIDE_SECRET_KEY`.
- [ ] `MULTI_TENANT_READY = true`; chế độ `single` không cần cấu hình gì thêm.
- [ ] Số đo trước / sau ghi trong quyết định; đường nóng `single` không chậm hơn ngoài độ dao động đã biết
      (D-179 ghi ±20%: chạy mỗi bài 3 lần, so trung vị).
- [ ] `docs/openapi.json`, `.env.example`, SDK TypeScript sinh lại; `docs/MODULE_GUIDE.md` + `.vi.md` và
      `docs/DEPLOY.md` + `.vi.md` cùng nội dung, sửa cùng commit; `CHANGELOG.md` có **Changed** cho mọi chữ ký đổi.
- [ ] `docs/DECISIONS.md` có mục: thiết kế đa khách (quyết định nền 1–7), mã hoá secret (thuật toán, thư viện,
      xoay khoá, căn cứ pháp lý), DNS (`hickory-resolver` nếu dùng), kết quả đo. `STACK.md` cập nhật thư viện mới.
- [ ] ROADMAP đánh dấu `[x]`; phát hành theo `docs/RELEASING.md` (bản minor — có thay đổi phá vỡ hợp đồng module).

## Chỗ dễ sai

- **Quên `.scoped(&tenant)`**: `find_by_id(id)` không lọc khách vẫn chạy được vì ID là toàn cục — chính là rò rỉ.
  Mọi `find`, `find_by_id`, `update_many`, `delete_many`, `Entity::update(..)` (thêm `.filter`), câu `sea_query`
  viết tay, `fetch_page`, `rows::lock_one`/`adjust` (điều kiện phải có `tenant_id`), JOIN. Bài rò rỉ + bộ canh SQL
  là lưới an toàn, không thay cho đọc lại code.
- **`INSERT` không đặt `tenant_id`**: bảng không có DEFAULT nên lỗi NOT NULL — tốt; đừng thêm DEFAULT "cho tiện".
  Đừng dùng `..Default::default()` khi dựng `ActiveModel`.
- **Event đi sai khách**: subscriber (`audit`, `webhooks`, `newsletter`, `comments` nghe `content`, mọi bên nghe
  `auth.user_deleted`) phải ghi bằng `event.tenant`, không bằng database giữ từ lúc khởi động. `webhooks` chỉ gửi
  tới endpoint cùng khách — gửi nhầm là lộ dữ liệu ra ngoài hệ thống.
- **Admin của khách thường có mọi permission** (`Grants::All`) → mọi API mặt điều khiển phải kiểm thêm "khách nền
  tảng và không đang vào khách khác".
- **API key / refresh token cũ không mang khách**: định tuyến về khách mặc định; ở `multi`, credential của khách
  khác **phải** mang khách, không được dò lần lượt từng database (chậm và mở đường dò).
- **Credential vào khách khác qua header**: `user_id` của nhân sự nền tảng không có trong `auth_user` của khách
  đích — code tra người dùng theo `principal.user_id()` trong khách đích phải chịu được "không có".
- **SQLite**: `ALTER TABLE` không bỏ được DEFAULT, không đổi PK → dùng helper; FK bật (`foreign_keys(true)`) →
  drop con trước cha; rename dựa vào SQLite ≥ 3.26 tự sửa tham chiếu FK (đừng bật `legacy_alter_table`). Một
  file SQLite mỗi khách: single-writer theo từng file (D-009), pool ghi của placement nên là 1–2.
- **Postgres**: tên constraint PK sau rename (`…__t_pkey`) phải đổi lại; dựng lại bảng lớn giữ lock tới hết
  transaction — tài liệu nâng cấp ghi "chạy trong cửa sổ bảo trì", ước lượng thời gian theo số dòng.
- **Unique có cột NULL** (`media_folder.parent_id`, `<m>_jobs` nếu lỡ dùng NULL): hai database coi NULL là khác
  nhau → không chống trùng. Đó là lý do có `NO_TENANT`.
- **Chuỗi audit**: chuỗi `v1` không băm `tenant_id`; tách chuỗi theo khách giữ được chuỗi cũ (thuộc khách mặc
  định) và chuỗi đi theo khách khi dời database. Không đổi cách băm `v1` (D-183).
- **Đóng pool đang dùng**: LRU chỉ bỏ pool khỏi sổ; request đang giữ bản clone vẫn chạy. Đừng gọi `close()`.
- **Dời khách giữa chừng**: job đang claim có thể ghi vào nguồn sau khi chép → chờ hết `claimed_until`, chặn ghi
  bằng `Moving` trước khi chép; outbox chưa giao chép cùng; cache tên miền / khách `invalidate` ở mọi máy chủ (TTL).
- **Khoá gốc**: lần đầu đặt `AUTOXIDE_SECRET_KEY` trên bản cài cũ, secret đã mã hoá bằng khoá trong database
  phải vẫn mở được (khoá trong `kernel_secrets` luôn là khoá cũ). Mất khoá = mất secret: tài liệu nói rõ phải
  backup `AUTOXIDE_SECRET_KEY` riêng khỏi database. Backup cũ (trước nâng cấp) vẫn chứa TOTP chữ thường.
- **Đường nóng**: chế độ `single` không được truy vấn mặt điều khiển mỗi request; `multi` dùng cache có TTL.
- **Lịch theo database, không theo khách**: 10.000 khách × lịch mỗi giờ = 10.000 job — sai. Lịch dọn dẹp quét cả
  database một lần.
- **Thông báo lỗi của `tenant_required`** phải nói cách sửa (tên miền, header, đăng nhập) — agent hay gọi thiếu.

## Căn cứ pháp lý

- Luật An ninh mạng 116/2025/QH15 (hiệu lực 01/7/2026) và NĐ 53/2022/NĐ-CP: lưu trữ dữ liệu người dùng tại Việt
  Nam với một số loại dịch vụ — vị trí dữ liệu chọn theo từng khách (placement) đáp ứng được ở mức khách. Cần đối
  chiếu văn bản gốc về đối tượng bắt buộc.
- Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 + NĐ 356/2025/NĐ-CP (hiệu lực 01/01/2026): biện pháp bảo vệ dữ liệu
  (tách biệt dữ liệu giữa khách, mã hoá secret và dữ liệu nhạy cảm khi lưu). Cần đối chiếu điều khoản cụ thể về
  mã hoá với người có chuyên môn pháp lý (D-186) — bước này không chặn vì thiếu căn cứ chi tiết.
- Ghi căn cứ trong mục quyết định về mã hoá và vị trí dữ liệu.

## Agent làm bước này tự quyết (ghi DECISIONS khi quyết)

- Định dạng credential mang khách của `auth` (vd tiền tố riêng `axk1_<tenant>…` để phân biệt key cũ `axk_…`;
  chú ý `random_token` có thể chứa `_`). Khuyến nghị: tiền tố mới + phần khách độ dài cố định.
- `TenantId` trong entity: `DeriveValueType` (an toàn kiểu) hay `Uuid` trần (ít ma sát) — khuyến nghị
  `DeriveValueType` nếu bản SeaORM ghim hỗ trợ gọn.
- AES-256-GCM qua `ring` hay XChaCha20-Poly1305 qua `chacha20poly1305` — khuyến nghị `ring` (không thêm thư viện).
- DNS: `hickory-resolver` hay DNS-over-HTTPS qua `reqwest` — khuyến nghị `hickory-resolver` (không phụ thuộc dịch
  vụ ngoài, chuẩn).
- Chế độ `multi` có **bắt buộc** `AUTOXIDE_SECRET_KEY` không — khuyến nghị bắt buộc (nhiều khách, khoá trong
  database không bảo vệ backup của ai cả); `single` chỉ cảnh báo như ROADMAP.
- Gate phủ `multi` bằng `TEST_TENANCY=multi` ở lượt Postgres hay chỉ test `multi` riêng — khuyến nghị cách đầu.
- Bộ canh SQL trong test (callback của SeaORM) — khuyến nghị làm nếu API của bản ghim cho phép.
- Giới hạn mặc định của pool / số database mở; TTL cache — đo rồi chỉnh, ghi số.
- Tên hàm chuyển tiếp trong phần A4 — miễn là `#[doc(hidden)]`, bị `#[deprecated]` ở B14 và xoá đầu bước 34.

## Bổ sung theo đối chiếu thiết kế 2 (D-208…D-218)

> Nguồn: `docs/prompts/ui-trace-roadmap.md`; mã `A-…` / `B-…` trỏ tới `docs/research/ui-trace/`.

- **Cấu hình hành vi module theo khách (A-b7).** Áp `Module::tenant_settings` cho cấu hình nghiệp vụ của
  module đang có: từ cấm, từ nghi ngờ, số link tối đa, chế độ duyệt của `comments`; hạn lưu bài nộp của
  `forms`; ngôn ngữ bật và ngôn ngữ mặc định của `content`; ngôn ngữ dịch `alt` của `media`. Biến môi trường
  hiện có thành **giá trị mặc định** cho mọi khách; khách ghi đè qua `PUT /api/v1/kernel/tenants/{id}/settings/{key}`.
- **Danh sách khách cho mặt điều khiển (A-c7).** `GET /api/v1/kernel/tenants` thêm lọc `placement_kind`,
  `schema_state`, `subscription_status`, `near_limit=true` và `expand=placement,subscription,usage` — một
  câu gọi cho cả trang, không gọi riêng từng dòng.
- **API job đội database (D-210, A-c6).** `POST /api/v1/kernel/placements/migrations {placement_id,
  dry_run=true}`, `POST /api/v1/kernel/tenants/{id}/moves {target_placement_id, dry_run=true}` — chỉ nhân sự
  nền tảng (`kernel.platform.manage`), chạy bằng job, tiến độ qua job API chung (D-213). CLI vẫn là cách chính.
- **Tạm khoá khác chỉ đọc (A-3.11).** `kernel_tenants.status = suspended` (kernel chặn **mọi** route của
  khách, kể cả đọc, trả 403 `tenant_suspended`) tách khỏi `read_only` (thuê bao quá hạn, bước 35: đọc được,
  mọi lệnh ghi trả 402 `plans.subscription_past_due`). Hai cờ độc lập, một middleware kiểm cả hai.
- Test thêm: `cau_hinh_module_theo_khach_ghi_de_bien_moi_truong`, `khach_tam_khoa_khong_doc_duoc`,
  `khach_chi_doc_doc_duoc_khong_ghi_duoc`, `di_chuyen_khach_dry_run_khong_doi_gi`.

## Bổ sung theo đối chiếu thiết kế đợt 2 (D-223…D-227)

> Nguồn: `docs/prompts/ui-phase2-api.md`; màn tương ứng ghi trong `docs/research/ui-traceability.md` (đợt 2).

- **Gắn tên miền với nút.** `PATCH /api/v1/kernel/domains/{id} {org_id | null}` (`kernel.domains.manage`); `org_id`
  phải là nút của khách hiện tại (hỏi `OrgDirectory` của bước 34, chưa cài thì chỉ nhận `null`); event
  `kernel.domain_updated`. Tên miền đã `revoked` → 409 `kernel.domain_revoked`.
  *Đã làm:* kernel hỏi qua trait hẹp `autoxide_kernel::tenancy::OrgNodes` (`has_node`) — module
  `organizations` của bước 34 công bố nó cạnh `OrgDirectory` của mình; mã lỗi là `tenant_domain_revoked`
  (mã lỗi của kernel không mang tiền tố, cùng họ `tenant_domain_*`; `kernel.domain_revoked` đã là tên event);
  tool MCP `kernel.update_domain`.
- **Tên setting của kết nối đối tác.** `TenantSetting` thêm `group: Option<&'static str>`; mọi cấu hình kết nối
  đối tác khai `group = "connection"` và đặt tên `<module>.connection.<loại>.<trường>` (trường bí mật đánh dấu
  secret, vào kho secret, không bao giờ trả lại). `GET /api/v1/kernel/settings?group=connection` là nguồn của màn
  "Kết nối đối tác"; kiểm thử dùng `POST /kernel/settings/{name}/test` đã có (bước 51). Tên cụ thể:

  | Kết nối | Bước | Tên setting |
  |---|---|---|
  | SMS (OTP, brandname) | 37 | `compliance.connection.sms.provider`, `….sms.credentials` (secret), `….sms.brandname` |
  | Xác thực danh tính (eKYC) | 37 | `compliance.connection.identity.provider`, `….identity.credentials` (secret) |
  | Cổng thanh toán | 36 | `ledger.connection.payment.<mã cổng>.credentials` (secret), `….payment.<mã cổng>.enabled` |
  | Chi trả (payout) | 36 | `ledger.connection.payout.<mã>.credentials` (secret) |
  | Hoá đơn điện tử | 36 | `tax.connection.einvoice.provider`, `….einvoice.credentials` (secret), `….einvoice.serial` |
  | Hãng vận chuyển | 44 | không dùng setting: nhiều tài khoản mỗi hãng nên giữ bảng `shipping_carrier_accounts`; màn liệt kê thêm từ `GET /api/v1/shipping/carrier-accounts` |
  | Máy chủ mail (SMTP) | 33 | `mail.connection.smtp.url` (secret), `….smtp.from` — tên cũ `mail.smtp_url`, `mail.from` vẫn nhận |
  | Nhà cung cấp AI (sinh chữ) | 33 | `ai.connection.text.api_key` (secret), `….text.model` — tên cũ `ai.api_key`, `ai.model` vẫn nhận |

  *Đã làm:* `TenantSetting::group(CONNECTION_GROUP)`; đổi tên bằng `TenantSetting::formerly(<tên cũ>)` (giá trị
  đặt dưới tên cũ vẫn đọc được, không cần upgrade dữ liệu). `POST /kernel/settings/{name}/test` **chưa có** ở
  bước này — thuộc bước 51.
