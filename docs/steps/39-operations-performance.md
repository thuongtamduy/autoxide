# Bước 39 — Vận hành và hiệu năng

> Lộ trình: `docs/ROADMAP.md` bước 39 · Thiết kế: `docs/research/business-foundation.md` mục 7
> (dòng "Quan sát theo khách") · Quyết định: D-073 (định dạng backup), D-076 (đo tải qua API thật),
> D-110 (ngưỡng 50 ms của tìm theo ý nghĩa), D-113 (khoá ký của kernel), D-158 (job queue, event,
> backup sau rà soát), D-185, D-189, D-190, D-196 (WebP), D-232 (AVIF đo trước)

## Mục tiêu

Khi đã có người dùng thật trên nền đa khách: dữ liệu được backup tự động và đã diễn tập restore;
người vận hành **thấy** hàng đợi kẹt trước khi khách thấy (gauge tồn đọng, dashboard, cảnh báo kèm
câu "làm gì"); đổi khoá không đăng xuất ai; có sổ tay xử lý sự cố. Hiệu năng được đo trên bộ dữ liệu
cỡ thật (không phải database trống), index được rà bằng `EXPLAIN` trên cả hai database, response
được nén và cache đúng cho CDN, và mọi log / trace mang `tenant_id` để tra sự cố của một khách.
Phục vụ mọi mô hình kinh doanh (D-185): đơn vị backup và số đo là **database**; tra theo **khách**
đi qua log, trace, CLI và số đo dùng của bước 35 — không qua nhãn số đo.
Ảnh AVIF chỉ thêm khi đo được lợi đủ lớn so với WebP đang có; không đạt thì ghi quyết định "không làm" (D-232).

## Bắt đầu khi

- Bước 38 xong (bảng phụ thuộc trong ROADMAP). Cần sẵn từ các bước trước:
  - Bước 33: `DatabaseRegistry` (sổ database, mỗi database có một khoá ổn định — gọi là
    `database_key` dưới đây, `default` ở chế độ `single`), worker / dispatcher duyệt mọi database,
    `tenant_id` trên mọi bảng cấp khách (gồm `<module>_jobs`, `<module>_events`), mã hoá AEAD có số
    phiên bản khoá (`autoxide_kernel::secret`), khoá file có tiền tố khách, `autoxide copy` lọc theo
    khách, chia lượt job công bằng.
  - Bước 34: `Principal` mang tenant và `Scope`; `TenantContext`.
  - Bước 35: số đo dùng (usage) theo khách trong module `plans`.
- Làm **nhóm A trước**: gauge tồn đọng của A dùng để quan sát lúc chạy tải ở nhóm B.

## Phạm vi

Sửa: `crates/kernel` (`builtin.rs`, `job/worker.rs`, `job/mod.rs`, `metrics.rs`, `secret.rs`,
`http/mod.rs`, `http/log.rs`, `ip_hash.rs`, `antispam/`), module mới bên trong kernel
`crates/kernel/src/backup.rs` và `crates/kernel/src/ops.rs`, `crates/app/src/commands/{backup,doctor,
jobs}.rs`, `modules/auth/src/{token.rs,config.rs}`, `modules/webhooks` (khoá ký chồng), `modules/content`,
`modules/media`, `modules/comments` (cache header, index, task sinh dữ liệu đo), `scripts/loadtest.sh`,
`docs/BENCHMARKS.md`, `docs/DEPLOY.md` + `docs/DEPLOY.vi.md`, `.env.example`, `CHANGELOG.md`; nếu AVIF đạt
ngưỡng (D-232): `modules/media/src/{variants,jobs,config}.rs`, `modules/content` (`avif_url`), `STACK.md`.
File mới: `deploy/grafana/autoxide-dashboard.json`, `deploy/prometheus/autoxide-alerts.yml`,
`scripts/bench-seed.sh`.

Không làm (NGOÀI phạm vi): backup tăng dần / point-in-time (WAL archiving — chỉ Postgres, trái Luật số
một); restore một khách từ backup database chung bằng lệnh riêng (dùng quy trình restore vào database
tạm + `autoxide copy --tenant` của bước 33, ghi trong sổ tay); upload multipart; nhãn số đo theo khách;
index ANN / `pgvector` khi số đo chưa vượt ngưỡng; Redis hay bất kỳ dịch vụ ngoài nào.

## Thiết kế

### Kiểu và trait

Phác thảo — tra API thật bằng skill `check-library-api` (sea-orm 2 lấy pool sqlx, `tower-http` 0.7
nén, `metrics` 0.24 gauge).

```rust
// crates/kernel/src/backup.rs — lõi của `autoxide backup` dời từ crates/app vào kernel để job của
// module `kernel` và lệnh CLI dùng CHUNG một đường (một việc một cách làm).
pub struct BackupManifest { pub format: u64, pub autoxide: String, pub created_at: DateTime<Utc>,
    pub source: String, pub database_key: String, pub tenants: Vec<TenantId>,
    pub tables: Vec<CopiedTable>, pub files: u64, pub file_bytes: u64 }
pub trait BackupSink: Send + Sync {          // nơi ghi: thư mục (CLI) hay Storage (job)
    async fn put(&self, relative: &str, content: ByteStream) -> Result<u64, Error>;
}
pub async fn write_backup(database: &Database, migrator: &Migrator, modules: &[&'static str],
    files: Option<Arc<dyn Storage>>, sink: &dyn BackupSink, scratch: &Path)
    -> Result<BackupManifest, Error>;
/// Tiền tố khoá Storage của bản backup theo lịch — bị loại khỏi phần chép file của mọi bản backup.
pub const BACKUP_PREFIX: &str = "kernel/backups/";

// crates/kernel/src/job/mod.rs — API mới: job chạy lâu (backup) được gia hạn lease.
impl JobRegistration { pub fn timeout(self, limit: Duration) -> Self; }   // mặc định = LEASE - 30s

// crates/kernel/src/ops.rs — số đo tồn đọng: tính riêng (test được trên hai database), đăng riêng.
pub struct BacklogSnapshot { pub database_key: String, pub modules: Vec<ModuleBacklog>,
    pub pools: Vec<PoolUsage> }
pub struct ModuleBacklog { pub module: &'static str, pub jobs_pending: u64,
    pub jobs_oldest_pending_seconds: u64, pub jobs_dead: u64, pub events_pending: u64,
    pub events_oldest_pending_seconds: u64, pub events_dead: u64 }
pub struct PoolUsage { pub pool: &'static str /* "read" | "write" */, pub in_use: u32,
    pub idle: u32, pub max: u32 }
pub async fn backlog(database: &Database, key: &str, modules: &Modules) -> Result<BacklogSnapshot, Error>;
pub fn publish(snapshots: &[BacklogSnapshot], max_database_labels: usize);   // đặt gauge
pub const KNOWN_METRICS: &[&str];   // mọi tên số đo kernel phát ra; test của dashboard/alert so với nó

// crates/kernel/src/security/secret.rs — xoay khoá gốc (API mới, không đổi hàm cũ).
impl SecretKey {
    pub async fn derive(&self, purpose: &str) -> Result<[u8; 32], Error>;            // khoá HIỆN TẠI: ký
    pub async fn derive_all(&self, purpose: &str) -> Result<Vec<[u8; 32]>, Error>;   // hiện tại + cũ: kiểm
}
```

`modules/auth/src/token.rs`: `AccessTokens { key: Vec<u8>, previous: Vec<Vec<u8>>, ttl }` — `issue`
ký bằng `key`; `verify` thử `key` rồi từng khoá cũ (so sánh hằng thời gian của `hmac::Mac::verify_slice`
cho mỗi khoá); header JWT giữ nguyên (không thêm `kid`, token cũ vẫn đọc được).

### Bảng dữ liệu

Hai migration mới của module nội bộ `kernel` (chạy trên cả hai database, có `down`):

1. `m0003_create_jobs` (số thật: số kế tiếp trong `builtin.rs`) = `jobs_migration(…, "kernel")` →
   bảng `kernel_jobs` (định dạng chung của job queue; bước 33 đã thêm `tenant_id` vào khuôn này).
2. `m0004_create_backups` → bảng `kernel_backups`, **bảng mặt điều khiển** (khai đúng phân loại của
   bước 33 để `cargo xtask check` không đòi `tenant_id`):

| Cột | Kiểu (SeaORM `schema::…`) | Ghi chú |
|---|---|---|
| `id` | `pk_uuid` | UUIDv7 |
| `database_key` | `string_len(64)` | database được backup |
| `status` | `string_len(16)` | `running` / `succeeded` / `failed` |
| `active_key` | `string_len_null(80)` | `<database_key>` khi `running`, `NULL` khi xong — unique index chặn hai bản cùng lúc (cách của D-110: nhiều `NULL` được trên cả hai database) |
| `storage_prefix` | `string_len(255)` | `kernel/backups/<database_key>/<id>/` |
| `started_at`, `heartbeat_at` | `timestamp_with_time_zone` | UTC |
| `finished_at` | `timestamp_with_time_zone_null` | |
| `size_bytes`, `rows`, `files` | `big_integer_null` | |
| `tables` | `integer_null` | |
| `autoxide_version` | `string_len(32)` | |
| `error` | `text_null` | cắt 2000 ký tự như `last_error` của job |

Index: unique `kernel_backups_active_key` (`active_key`); `kernel_backups_database_started`
(`database_key`, `started_at`). Bản `running` có `heartbeat_at` cũ hơn 3 × lease (tiến trình chết) thì
job kế tiếp đánh `failed` rồi mới mở bản mới.

Nhóm B thêm index thiếu bằng **migration mới** của module sở hữu bảng (không sửa migration cũ), index
bắt đầu bằng `tenant_id` (luật của bước 33).

Bản AVIF (nếu làm, D-232) không cần bảng mới: là dòng `media_variant` có `content_type = 'image/avif'` — unique
index (item, phiên bản, chiều rộng, `content_type`) của `m0007` (D-196) đã cho mỗi cỡ một bản mỗi loại.

### API REST

- `GET /api/v1/kernel/ops/backlog` — permission mới `kernel.ops.read` ("See queue backlogs, dead
  jobs, undelivered events, database connections and backups."). Trả `Data<OpsStatus>`: mỗi database
  một mục (backlog theo module, pool, bản backup gần nhất), và `top_tenants` (tối đa 20 khách có nhiều
  job chờ nhất mỗi database — đây là chỗ tra **theo khách**, vì `tenant_id` không được làm nhãn số
  đo). Chỉ nhân sự nền tảng có permission này (bước 34); quản trị của một khách chỉ thấy khách mình
  (`Scope::Tenant` — lọc qua công cụ của kernel). Handler chỉ gọi `ops::status(...)`.
- `GET /api/v1/kernel/ops/backups` — cùng permission, `Page<BackupRecord>` qua `fetch_page`.
- Không có endpoint chạy backup qua HTTP (job + CLI đủ; tránh một lệnh nặng gọi được từ Internet).

### Tool MCP

`kernel.ops_backlog` (read-only, `kernel.ops.read`) và `kernel.list_backups` (read-only, `PageInput`)
— cùng service với REST (luật kiến trúc 8). Agent vận hành tự đọc được hàng đợi kẹt.

### Event

`kernel.backup_completed` (payload: `database_key`, `backup_id`, `size_bytes`, `files`) và
`kernel.backup_failed` (`database_key`, `backup_id`, `error`) — ghi bằng `Outbox::record` cùng
transaction cập nhật `kernel_backups`; module `audit` / `notifications` (bước 37) nhận nếu muốn báo.
Cần `outbox_migration` cho module `kernel` (bảng `kernel_events`) nếu bước 33 chưa thêm.

### Job / lịch

| Kind | Lịch | Việc |
|---|---|---|
| `kernel.backup` | `Schedule::new("kernel.backup", "kernel.backup", interval)` khi interval > 0 | mở dòng `running` (unique `active_key`), `write_backup` vào `StorageSink` dưới `kernel/backups/<database_key>/<id>/`, cập nhật dòng + event, rồi xoá bản cũ quá `KEEP` (chỉ bản `succeeded`; luôn giữ bản tốt mới nhất). Timeout riêng `JobRegistration::timeout(6 h)` |
| `media.make_avif` (D-232, nếu làm) | job, xếp sau khi `media.make_variants` lưu xong, chỉ khi `AUTOXIDE_MEDIA_AVIF=true`; `unique_key = "avif:{item}:{version}"` | mã hoá AVIF từ ảnh gốc cho mỗi cỡ đã có, lưu dòng `media_variant` + file `…_w<rộng>.avif` trong một transaction; ảnh hỏng → log, kết thúc `Ok`, không thử lại |
| (không phải job) | task nền mỗi `AUTOXIDE_METRICS_BACKLOG_SECONDS` | `ops::backlog` cho mọi database được giao rồi `ops::publish`; chạy ở tiến trình có vai `worker` hay `all` |

Worker gia hạn lease: job có `timeout` > `LEASE` thì một task con `UPDATE <module>_jobs SET
claimed_until = now + LEASE WHERE id = ? AND claimed_by = ?` mỗi `LEASE / 3`; mất quyền giữ (0 dòng)
thì huỷ job. Lịch chạy theo từng database (bước 33), nên mỗi database tự backup mình.

### Cấu hình (`Setting`, sinh lại `.env.example`)

| Biến | Mặc định | Ý nghĩa |
|---|---|---|
| `AUTOXIDE_BACKUP_INTERVAL_HOURS` | `24` | 0 tắt backup theo lịch (1–720) |
| `AUTOXIDE_BACKUP_KEEP` | `7` | số bản `succeeded` giữ lại mỗi database (1–365) |
| `AUTOXIDE_BACKUP_INCLUDE_FILES` | `false` | chép cả file vào bản backup. Mặc định tắt: file đã nằm trên chính Storage đó, chép sang cùng chỗ không bảo vệ gì mà nhân đôi dung lượng; tài liệu hướng dẫn sao bucket / thư mục sang nơi khác |
| `AUTOXIDE_METRICS_BACKLOG_SECONDS` | `30` | chu kỳ cập nhật gauge tồn đọng (5–3600) |
| `AUTOXIDE_METRICS_MAX_DATABASE_LABELS` | `50` | quá số database này thì gộp phần còn lại vào nhãn `database="other"` |
| `AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS` | (trống, secret) | khoá ký access token cũ, cách nhau bởi dấu phẩy; chỉ dùng để KIỂM |
| `AUTOXIDE_SECRET_KEY_PREVIOUS` | (trống, secret) | khoá gốc cũ của kernel; chỉ dùng để kiểm / giải mã |
| `AUTOXIDE_WEBHOOKS_SECRET_OVERLAP_HOURS` | `24` | sau khi xoay secret của webhook, ký bằng cả secret cũ trong chừng này giờ (0–168) |
| `AUTOXIDE_HTTP_COMPRESSION` | `true` | (chỉ khi quyết nén trong app) |
| `AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS` | `60` | `s-maxage` cho endpoint công khai (0–86400) |
| `AUTOXIDE_MEDIA_AVIF` | `false` | (chỉ khi AVIF đạt ngưỡng, D-232) sinh thêm bản AVIF của ảnh thu nhỏ bằng job nền |

Ngưỡng cảnh báo "backup quá cũ" của `autoxide doctor` = 2 × interval (không thêm biến); interval = 0
thì doctor cảnh báo "không có backup tự động".

### Permission · Mã lỗi

Permission mới: `kernel.ops.read`. Mã lỗi mới (kèm `crates/kernel/locales/vi.json`):

| `code` | HTTP | Câu tiếng Việt |
|---|---|---|
| `kernel.backup_running` | 409 | "Đang có một bản backup của database {database} chạy (bắt đầu lúc {started_at}). Chờ nó xong rồi thử lại." |
| `kernel.backup_storage_missing` | 500 | "Backup theo lịch cần nơi lưu file (module `files`). Bật module `files` hoặc đặt AUTOXIDE_BACKUP_INTERVAL_HOURS=0." |

(Lỗi của job không trả qua HTTP; mã dùng cho `last_error`, `kernel_backups.error` và CLI.)

### Số đo (Prometheus) — tên theo họ đã có của kernel (`jobs_*`, `events_*`, `database_*`)

| Tên | Loại | Nhãn |
|---|---|---|
| `jobs_pending` | gauge | `database`, `module` |
| `jobs_oldest_pending_seconds` | gauge | `database`, `module` (job tới hạn chờ lâu nhất; 0 khi không có) |
| `jobs_dead` | gauge | `database`, `module` (dead-letter queue: `failed_at IS NOT NULL`) |
| `events_pending` / `events_oldest_pending_seconds` / `events_dead` | gauge | `database`, `module` |
| `database_connections` | gauge | `database`, `pool` (`read`/`write`), `state` (`in_use`/`idle`) |
| `database_connections_max` | gauge | `database`, `pool` |
| `backup_last_success_timestamp_seconds` | gauge | `database` |
| `backup_runs_total` | counter | `outcome` (`succeeded`/`failed`) |
| `backup_duration_seconds` / `backup_size_bytes` | histogram / gauge | `database` |

`module` là tập nhỏ cố định (module đã đăng ký); `database` bị chặn bởi
`AUTOXIDE_METRICS_MAX_DATABASE_LABELS`. **Không** có nhãn `tenant_id` hay `org_id`.

### Dashboard và cảnh báo

`deploy/prometheus/autoxide-alerts.yml` — mỗi luật có `annotations.summary` và `annotations.action`
(câu "làm gì", tiếng Anh, trỏ mục của sổ tay trong `docs/DEPLOY.md`):

| Cảnh báo | Biểu thức (phác thảo) | Làm gì |
|---|---|---|
| `AutoxideHigh5xxRate` | tỉ lệ `http_requests_total{status=~"5.."}` > 2 % trong 10 phút | đọc log lỗi 5xx theo `request_id` |
| `AutoxideSlowP95` | `histogram_quantile(0.95, …http_request_duration_seconds_bucket…)` > 0,5 s trong 10 phút | xem route nào, `EXPLAIN`, pool |
| `AutoxideJobsStuck` | `max(jobs_oldest_pending_seconds) > 600` | worker có chạy không; `autoxide jobs …` |
| `AutoxideDeadLetterGrowing` | `increase(jobs_failed_total[1h]) > 0` hay `jobs_dead` tăng | `autoxide jobs failed`, sửa nguyên nhân, `autoxide jobs retry` |
| `AutoxideEventsBacklog` | `max(events_oldest_pending_seconds) > 300` | bên nhận event lỗi; xem `last_error` |
| `AutoxideDbPoolExhausted` | `in_use / max >= 0.9` trong 5 phút | tăng pool, tìm truy vấn chậm |
| `AutoxideBackupTooOld` | `time() - backup_last_success_timestamp_seconds > 2 × interval` | đọc `kernel_backups.error`, chạy `autoxide backup` tay |

`deploy/grafana/autoxide-dashboard.json` — bảng: request/s, tỉ lệ 5xx, p95 theo route, tồn đọng job
và event theo module, dead-letter, pool, tuổi backup. Biến chọn `database`.

### Nén response, cache cho CDN, tìm theo ý nghĩa — xem "Thứ tự làm" nhóm B.

### Ảnh AVIF — đo trước rồi mới quyết (D-232)

**Đo trước** (chưa thêm thư viện vào workspace; đo trong một crate tạm không commit, chỉ commit số đo):

- Bộ ảnh mẫu: ảnh sản phẩm (nền trắng, có ảnh cắt nền trong suốt), ảnh có chữ (ảnh chụp màn hình, banner có
  chữ), ảnh nền / phong cảnh (ảnh chụp lớn, như `lake.jpg` của D-196) — ảnh CC0 hay tự làm, ghi nguồn và license
  trong `docs/BENCHMARKS.md`. Mỗi ảnh ở bốn cỡ mặc định của `AUTOXIDE_MEDIA_IMAGE_WIDTHS`.
- Ứng viên: crate mã hoá AVIF bản ổn định mới nhất, tra bằng skill `check-library-api`; ưu tiên Rust thuần
  (vd `ravif` trên `rav1e`, hay feature `avif` của `image` đang ghim). Bản bọc C (`libavif`) chỉ xét khi không có
  bản Rust thuần đạt. License phải hợp MIT / Apache (`cargo deny` của gate quyết).
- So với WebP chất lượng 80 hiện có (D-196): **cỡ file ở chất lượng tương đương** — chọn mức chất lượng AVIF để
  điểm SSIM (hay SSIMULACRA2) không thấp hơn bản WebP của cùng ảnh, rồi so cỡ; **thời gian mã hoá** mỗi cỡ và
  tổng bốn cỡ (build release, một thread và số thread mặc định); **RAM đỉnh** (`/usr/bin/time -v`, max RSS);
  **thời gian biên dịch thêm** (`cargo build --release` sạch có / không crate) và cỡ binary thêm. Công cụ chấm
  điểm chạy ngoài repo, không thêm vào workspace.
- Ghi số vào `docs/BENCHMARKS.md` kèm máy, ngày, phiên bản crate, tốc độ / chất lượng đã chọn.

**Ngưỡng quyết** (agent đặt, ghi DECISIONS kèm số đo): làm khi trung vị trên bộ ảnh tiết kiệm **≥ 20%** dung
lượng so với WebP ở chất lượng tương đương **và** thời gian mã hoá chấp nhận được khi chạy bằng job — tổng bốn
cỡ của một ảnh ≤ 10 s ở một thread trên máy đo, RAM đỉnh ≤ 512 MiB. Không đạt một điều kiện nào: ghi quyết định
"không làm AVIF" kèm số đo vào `docs/DECISIONS.md`, dừng mục này (không thêm thư viện, không thêm cấu hình).

**Nếu làm:**

- Biến thể AVIF sinh **bằng job `media.make_avif` sau khi tải lên** — không bao giờ trên đường request tải lên
  hay trong job `media.make_variants` (job đó giữ thời gian như cũ). Mã hoá trong `spawn_blocking`, từ ảnh gốc
  đã xoay theo EXIF (không từ bản JPEG đã nén, tránh mất chất lượng hai lần), cùng giới hạn `MAX_PIXELS`.
- `AUTOXIDE_MEDIA_AVIF` mặc định `false`: tắt thì không xếp job, không sinh; bản AVIF đã có vẫn phục vụ.
  `autoxide task media.refresh_variants` xếp `media.make_avif` cho ảnh còn thiếu khi bật.
- Phục vụ theo cách của WebP (D-196): địa chỉ riêng `…/content?v=…&w=<rộng>&format=avif`, ETag riêng (`-avif`),
  cache một năm; mỗi mục `variants` thêm `avif_url` (và `avif_size` ở REST của `media`), `MediaVariant` của `api`
  và `PublicVariant` của `content` cũng thêm — `null` khi chưa có. Không chọn loại theo header `Accept` (lý do ở
  D-196: CDN và `Vary`). Xin `format=avif` khi chưa có bản AVIF → trả bản WebP cùng cỡ (không có thì JPEG / PNG),
  cache ngắn. WebP vẫn là định dạng mặc định; `url` cũ không đổi hành vi. Thêm thư viện vào `STACK.md`.
- Khoá nơi lưu: khoá của bản nhỏ thêm `.avif`; job dọn mồ côi (`parse_key`) hiểu thêm đuôi này.

## Thứ tự làm

Mỗi dòng một commit (chạy `./gate.sh --quick` trước mỗi commit). Mã `A1…B8` ánh xạ tới mục `- [ ]`
của ROADMAP theo đúng thứ tự xuất hiện.

**Nhóm A — vận hành**

1. (A1) Dời lõi backup sang `autoxide_kernel::backup` (`write_backup`, `BackupSink`,
   `BACKUP_PREFIX`); `autoxide backup` / `restore` dùng nó; loại `kernel/backups/` khỏi phần chép
   file. Test sẵn có của `backup.rs` phải xanh không sửa ý nghĩa.
2. (A1) `JobRegistration::timeout` + gia hạn lease trong `worker.rs`; test job dài.
3. (A1) Bảng `kernel_jobs`, `kernel_backups`, (`kernel_events` nếu thiếu); job `kernel.backup`, lịch,
   giữ N bản, event, counter / gauge backup; cấu hình; `autoxide doctor` (check `kernel.backup_age`
   khai qua `Module::checks` của module `kernel`); sinh lại `.env.example`. Kiểm job `files` dọn
   file mồ côi (D-160) không động vào `kernel/backups/` (nó chỉ được liệt kê tiền tố của mình).
4. (A2) Test diễn tập restore đầu-cuối (dưới) + quy trình restore từng bước trong
   `docs/DEPLOY.md` / `.vi.md` (gồm: restore vào database tạm rồi `autoxide copy --tenant` để lấy lại
   một khách; lấy bản backup từ Storage về thư mục).
5. (A3) `ops::backlog` + `ops::publish` + task nền; `KNOWN_METRICS`; xtask kiểm macro số đo không
   dùng nhãn tên `tenant`, `tenant_id`, `org_id`, `user_id`.
6. (A3) `GET /api/v1/kernel/ops/backlog`, `/backups`, tool MCP, permission `kernel.ops.read`;
   CLI `autoxide jobs stats [--database KEY]` in cùng dữ liệu (kèm top khách).
7. (A4) `deploy/prometheus/autoxide-alerts.yml`, `deploy/grafana/autoxide-dashboard.json`, test
   giữ khớp; mục "Monitoring" trong `docs/DEPLOY.md` (EN + VI).
8. (A5) `AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS` + `AccessTokens::verify` nhiều khoá;
   `AUTOXIDE_SECRET_KEY_PREVIOUS` + `SecretKey::derive_all` (antispam token, chữ ký ngắn hạn kiểm
   bằng mọi khoá; AEAD của bước 33 giải mã theo số phiên bản khoá); task `kernel.reencrypt_secrets`
   (`autoxide task kernel.reencrypt_secrets`) mã hoá lại bằng khoá mới, chạy lại được; webhooks giữ
   secret cũ `AUTOXIDE_WEBHOOKS_SECRET_OVERLAP_HOURS` và gửi hai chữ ký (Standard Webhooks cho nhiều
   chữ ký cách nhau bằng dấu cách) — migration mới của `webhooks`: `previous_secret` (mã hoá),
   `previous_secret_until`; sửa mô tả OpenAPI của `rotate_webhook_secret` ("The old secret stays
   valid for …"). Quy trình xoay khoá trong `docs/DEPLOY.md` (EN + VI).
9. (A6) Sổ tay xử lý sự cố trong `docs/DEPLOY.md` + `.vi.md`: hàng đợi kẹt, dead-letter
   (`autoxide jobs failed` / `retry`), database đầy (SQLite: file + WAL; Postgres: bảng lớn nhất,
   `VACUUM`), lộ khoá (xoay theo mục 8; lộ `AUTOXIDE_SECRET_KEY` thì mã hoá lại), mất admin cuối cùng
   (`autoxide user …`), nâng cấp khi hai phiên bản code cùng chạy (migration "mở rộng trước, thu hẹp
   sau": thêm cột nullable → code mới ghi cả hai → bỏ cột cũ ở bản sau), `tenant_maintenance` của một
   database (bước 33). Mỗi tình huống: dấu hiệu (cảnh báo nào), lệnh kiểm, cách sửa.

**Nhóm B — hiệu năng ở quy mô thật**

10. (B1) Task sinh dữ liệu đo, mỗi module tự ghi bảng của mình (luật kiến trúc 1): `auth.bench_seed`,
    `media.bench_seed`, `content.bench_seed`, `comments.bench_seed`, chỉ biên dịch khi bật cargo
    feature `bench-seed` của crate `autoxide` (bản phát hành không có), từ chối chạy khi database đã
    có dữ liệu thật (đếm người dùng > 1 hoặc thiếu cờ `--yes-this-is-a-benchmark-database`). Ghi theo
    lô (multi-row insert qua SeaORM `insert_many`, một transaction mỗi lô 1.000 dòng), dữ liệu có
    tenant / org hợp lệ (2 khách, mỗi khách 3 nút). `scripts/bench-seed.sh` gọi lần lượt theo cỡ
    `small` (1/100) và `full` (100k bài nhiều ngôn ngữ có danh mục, thẻ, tác giả; 50k ảnh kèm bản
    nhỏ; 1M bình luận; 5k người dùng); task sau đọc ID task trước từ file `--ids` (JSON).
11. (B2) `scripts/loadtest.sh` thêm `LOADTEST_DATASET=small|full` (chạy `bench-seed.sh` trước) và
    các bài đọc công khai: `content/public/resolve`, `/public/posts`, `/public/terms/{taxonomy}/{slug}`,
    `/public/sitemap`, `/public/feed`, `/public/search`, `media/{id}/content` (gốc và bản nhỏ),
    `comments/public/threads`, `/public/comments`. Ghi số đo kèm máy, ngày, cỡ dữ liệu vào
    `docs/BENCHMARKS.md` và ngưỡng p95 mỗi đường (khuyến nghị: đọc JSON ≤ 50 ms, sitemap / feed ≤
    200 ms, search ≤ 150 ms ở 64 kết nối trên máy 4 nhân — chỉnh theo số đo thật, ghi lý do).
12. (B3) Rà `EXPLAIN QUERY PLAN` / `EXPLAIN ANALYZE` cho mọi truy vấn danh sách, lọc, đếm của 13
    module + module bước 33–38 trên bộ `full`; bảng "truy vấn → kế hoạch trước / sau" trong
    `docs/BENCHMARKS.md`; mỗi module thiếu index một migration mới. Có thể thêm `cargo xtask explain`
    in kế hoạch các truy vấn mẫu (tuỳ chọn).
13. (B4) Đo nén: cỡ và thời gian gzip / brotli trên `openapi.json` (> 1 MB), một trang
    `/public/posts`, một trang danh sách quản trị; quyết nén trong app (`tower-http` feature
    `compression-gzip`, `compression-br`, `CompressionLayer` với predicate) hay để reverse proxy;
    ghi quyết định (skill `record-decision`), `STACK.md` nếu thêm feature, sửa `docs/DEPLOY.md`.
14. (B5) Cache cho CDN: một lớp kernel `http/cache.rs` — response chưa có `Cache-Control` thì
    đặt `private, no-store` khi request mang `Authorization` hay cookie, và `no-store` cho mọi phương
    thức không phải GET/HEAD; endpoint công khai đặt `public, max-age=0, s-maxage=<…>,
    stale-while-revalidate=300` + `ETag` + `Vary: Accept-Language, Accept-Encoding, Origin`
    (bỏ `Origin` khi CORS là `*`); media có `?v=` đúng phiên bản giữ `max-age=31536000, immutable`;
    `/public/preview` và mọi thứ riêng tư giữ `no-store`. Hướng dẫn đặt CDN (Cloudflare / CloudFront:
    tôn trọng header gốc, không cache khi có `Authorization`, khoá cache gồm Host) trong DEPLOY.
15. (B6) Tìm theo ý nghĩa trên bộ `full`: đo như mục "Tìm theo ý nghĩa" của BENCHMARKS (cùng máy,
    cùng cách, thêm cỡ thật theo ngôn ngữ). Dưới 50 ms → ghi số đo, giữ nguyên. Vượt → chọn cách
    giảm chạy được trên cả hai database (khuyến nghị theo thứ tự: chia tính toán ra nhiều nhân bằng
    `spawn_blocking` theo khúc; vector `f16` / `int8` trong bộ nhớ; index sau một trait), ghi quyết định.
    Cache vector trong bộ nhớ có khách trong khoá (bước 33) — đo cả bộ nhớ khi nhiều khách.
16. (B7) Quan sát theo khách: span request (`http/log.rs`), span job (`run_job`), span giao event
    (`dispatcher.rs`), span tool MCP mang trường `tenant_id` (và `org_id` khi có); thuộc tính OTLP
    `autoxide.tenant_id`; tài liệu: tra một khách bằng log / trace, lượng dùng theo khách lấy từ
    số đo dùng của bước 35 (`plans`), tồn đọng theo khách từ `ops/backlog`.
17. (B8) Đo AVIF (D-232): bộ ảnh mẫu, crate ứng viên, số đo vào `docs/BENCHMARKS.md`; quyết theo ngưỡng, ghi
    `docs/DECISIONS.md` (skill `record-decision`). Không đạt → dừng ở đây, mục B8 xong với quyết định "không làm".
18. (B8, chỉ khi đạt) Job `media.make_avif`, `AUTOXIDE_MEDIA_AVIF`, `format=avif`, `avif_url` (`media`,
    `content`), dọn mồ côi hiểu `.avif`, `STACK.md`, `docs/openapi.json` + `.env.example` sinh lại, tài liệu media
    (EN + VI).
19. Cuối: CHANGELOG, đánh dấu ROADMAP, gate đầy đủ, phát hành theo `docs/RELEASING.md`.

## Test bắt buộc

Mọi test chạm database dùng `TestDatabase::run` (tự chạy trên SQLite và Postgres theo
`TEST_DATABASE_URL`); test qua REST / MCP dựng bằng `TestApp::builder`.

- `crates/app` (e2e): `dien_tap_backup_restore_roi_khoi_dong` — dựng dữ liệu qua REST (admin, bài
  `content` có ảnh `media`, bình luận, webhook), `run_backup` (có file), restore vào database trống
  khác, `runtime::start(RequireApplied)`, `/health/ready` 200, đăng nhập bằng admin cũ được, bài và
  bình luận khớp, nội dung ảnh khớp SHA-256. Thêm một lượt chép chéo: backup từ database của gate →
  restore vào SQLite (đã có cho dữ liệu; thêm phần khởi động và file).
- `kernel`: `backup_theo_lich_ghi_vao_storage_va_giu_n_ban` (KEEP=2, ba lượt → còn 2, bản `failed`
  không đếm); `hai_worker_cung_luc_chi_mot_ban_backup` (hai `Worker` cùng database, `run_once` song song
  → một dòng `succeeded`, một bên nhận `kernel.backup_running` hay bỏ qua); `backup_khong_chep_thu_muc_backup`;
  `backup_dang_running_ma_tien_trinh_chet_thi_danh_failed`; `job_dai_duoc_gia_han_lease` (timeout >
  LEASE, lease giả ngắn qua hàm `pub(crate)` cho test); `job_mat_quyen_giu_thi_dung`.
- `doctor_canh_bao_backup_qua_cu` và `doctor_bao_khong_co_backup_tu_dong`.
- `ops`: `ton_dong_dem_job_cho_tuoi_va_dead_letter`, `ton_dong_dem_event_chua_giao`,
  `ton_dong_khong_dem_job_hen_gio_chua_toi` (job `run_at` tương lai không tính tuổi);
  `publish_gop_database_vuot_gioi_han_thanh_other` (recorder cục bộ — `metrics::with_local_recorder`,
  tra API — để không đụng recorder toàn tiến trình).
- `ops_backlog_can_permission` (REST và MCP), `quan_tri_khach_chi_thay_khach_minh` (rò rỉ giữa khách:
  khách A không thấy `top_tenants` của B).
- `deploy`: `canh_bao_va_dashboard_chi_dung_so_do_co_that` (đọc file alerts / dashboard dạng chữ, mọi
  tên số đo xuất hiện phải nằm trong `KNOWN_METRICS` hoặc số đo của module đã khai); mỗi luật có
  `action`.
- `xtask`: `nhan_so_do_khong_co_tenant` (một file mẫu vi phạm → check báo lỗi).
- `auth`: `token_ky_bang_khoa_cu_van_hop_le_trong_chuyen_tiep`, `token_ky_bang_khoa_la_bi_tu_choi`,
  `token_moi_ky_bang_khoa_moi`. `kernel`: `form_token_ky_bang_khoa_goc_cu_van_kiem_duoc`,
  `ma_hoa_lai_secret_bang_khoa_moi_chay_lai_duoc` (dừng giữa chừng rồi chạy lại: không hỏng dòng nào).
  `webhooks`: `xoay_secret_gui_hai_chu_ky_trong_khoang_chong`, `het_khoang_chong_chi_mot_chu_ky`.
- `http`: `response_co_authorization_khong_bao_gio_public` (lớp cache), `endpoint_cong_khai_co_s_maxage_etag_vary`,
  `media_co_phien_ban_immutable`, `preview_no_store`; nếu nén trong app: `khong_nen_anh_va_range`,
  `nen_json_lon_khi_client_nhan_br`, `etag_van_304_sau_khi_nen`.
- `logging`: `log_request_mang_tenant_id` (bắt log bằng subscriber thử, kiểm trường).
- Test đồng thời: backup song song (trên) và gia hạn lease khi hai worker tranh — chạy trên cả hai database.
- `media` (chỉ khi làm AVIF, D-232; cả hai database): `avif_tat_thi_khong_sinh`,
  `avif_bat_thi_job_sinh_dung_mot_lan` (xếp hai lần, chạy lại job → mỗi cỡ một dòng `image/avif`),
  `avif_khong_sinh_tren_request_tai_len` (ngay sau khi tải lên chưa có bản AVIF; có sau khi worker chạy),
  `avif_anh_hong_khong_lam_hong_job_khac` (ảnh hỏng kết thúc `Ok`, job ảnh khác trong cùng lượt vẫn sinh),
  `hai_worker_cung_job_avif_mot_ban` (đồng thời), `format_avif_chua_co_tra_ban_webp`,
  `don_mo_coi_hieu_duoi_avif`.

## Tiêu chí xong

- [ ] Mọi mục `- [ ]` của bước 39 trong ROADMAP xong theo bảng ánh xạ A1–A6, B1–B8 (B8 = AVIF, D-232).
- [ ] Backup theo lịch chạy, giữ đúng N bản, bản hỏng có log + `backup_runs_total{outcome="failed"}`;
      `autoxide doctor` cảnh báo khi cũ; backup không giữ transaction ghi (`begin_snapshot`).
- [ ] Diễn tập restore xanh trên SQLite và Postgres (và chép chéo); quy trình restore trong DEPLOY EN + VI.
- [ ] Gauge tồn đọng có trên `/metrics`; không có nhãn `tenant_id` (xtask chặn).
- [ ] File dashboard + alert trong `deploy/`, test giữ khớp, hướng dẫn EN + VI.
- [ ] Đổi khoá ký không đăng xuất ai (test), quy trình trong DEPLOY.
- [ ] Sổ tay sự cố EN + VI, cùng nội dung.
- [ ] BENCHMARKS có: lệnh sinh dữ liệu, số đo đường đọc công khai với ngưỡng p95, bảng EXPLAIN
      trước / sau, số đo nén, số đo tìm theo ý nghĩa — mỗi số kèm máy, ngày, cỡ dữ liệu, cả hai database.
- [ ] Quyết định ghi bằng `record-decision`: job dài + gia hạn lease, backup theo lịch vào Storage
      (mặc định không chép file), nhãn `database` có giới hạn, nén, cache CDN, (tìm theo ý nghĩa nếu đổi).
- [ ] `.env.example` và `docs/openapi.json` sinh lại; CHANGELOG `[Unreleased]`; ROADMAP đánh dấu.
- [ ] AVIF (D-232): số đo trong BENCHMARKS và quyết định trong DECISIONS — "làm" (kèm test trên) hay "không
      làm" (kèm số đo).
- [ ] `./gate.sh` đầy đủ xanh; phát hành theo `docs/RELEASING.md` (D-171).

## Chỗ dễ sai

- **Backup lâu hơn lease**: job hiện bị cắt sau `LEASE - 30s` (270 s) và worker khác lấy lại — với bộ
  dữ liệu `full` backup chạy lâu hơn thế, thành hai bản chạy song song. Phải có `timeout` riêng + gia
  hạn lease + unique `active_key`.
- **Backup chép chính nó**: Storage chứa `kernel/backups/` → bản sau chứa bản trước, phình theo cấp
  số. Loại tiền tố ở `write_backup` (cả CLI), và job dọn file mồ côi của `files` không được xoá nó.
- **SQLite WAL phình**: transaction đọc dài của `begin_snapshot` chặn checkpoint; ghi vẫn chạy nhưng
  file `-wal` lớn dần tới khi backup xong. Ghi vào sổ tay; không "sửa" bằng cách bỏ snapshot.
- **Thư mục tạm**: `write_backup` dựng file SQLite tạm trước khi đẩy lên Storage — cần chỗ trống cỡ
  database; dọn khi lỗi; quyền thư mục `0700` như `private_dir` hiện có.
- **Restore phải vào database trống** (D-073) — test đừng "dọn" database cũ để tái dùng.
- **Gauge nhiều tiến trình**: mỗi tiến trình `worker` đều đặt cùng gauge → Prometheus có nhiều chuỗi
  khác `instance`; luật cảnh báo dùng `max without(instance)`, không `sum`.
- **Truy vấn đếm tồn đọng** chạy định kỳ trên kết nối ĐỌC; `min(occurred_at)` của event không có
  index — bảng event chỉ giữ 7 ngày nên chấp nhận được, nhưng đo trên bộ `full`; đừng chạy trên kết
  nối ghi của SQLite (chỉ có một).
- **Nhãn `database`** có thể nhiều (mỗi khách `dedicated` một database): luôn qua giới hạn `other`.
- **Xoay khoá**: không thêm `kid` vào JWT (đổi định dạng token đang lưu ở client); kiểm mọi khoá theo
  hằng thời gian. Khoá băm IP (`ip_hash`, D-153) KHÔNG xoay theo kiểu "kiểm bằng cả hai": băm là để
  tra cứu, đổi khoá thì bộ đếm theo IP bắt đầu lại — chấp nhận, ghi trong quy trình. Lộ
  `AUTOXIDE_SECRET_KEY` mà khoá gốc đang nằm trong database (D-113) thì xoay không giúp với backup cũ.
- **Nén**: không nén `image/*`, `video/*`, `application/zip`, response có `Content-Range`, và
  `text/event-stream` (SSE của bước 40 — nén giữ đệm làm stream đứng); `ETag` của bản nén phải khác
  bản gốc hoặc là weak; đặt lớp nén NGOÀI lớp `not_modified`.
- **Cache CDN**: response lỗi 4xx/5xx không `public`; `Set-Cookie` không bao giờ đi với `public`;
  sau bước 33 tên miền xác định khách — CDN phải gồm Host trong khoá cache (mặc định có), header của
  nhân sự nền tảng chọn khách thì response luôn `private`.
- **Sinh dữ liệu**: chỉ qua task của module sở hữu bảng; SQLite chỉ một kết nối ghi — lô lớn trong
  một transaction ngắn, không giữ transaction qua việc tạo ảnh; ảnh mẫu tạo một lần, dùng lại nội dung
  (khử trùng theo SHA-256 của `media` sẽ báo trùng — đổi vài byte mỗi ảnh).
- **EXPLAIN** khác cú pháp và ý nghĩa giữa hai database; Postgres cần `ANALYZE` sau khi nạp dữ liệu,
  không thì kế hoạch sai. So kế hoạch trên dữ liệu cỡ thật, không trên database trống.
- **`tenant_id` trong log**: thêm vào span, không ghi thân request hay email (luật code).
- **AVIF chặn worker** (D-232): mã hoá AV1 nặng CPU hơn WebP nhiều lần; job AVIF chạy dài làm chậm job khác của
  `media`. Đo thời gian thật trên bộ ảnh; job dài hơn lease thì dùng `JobRegistration::timeout` (mục A1); giữ việc
  mã hoá trong `spawn_blocking`, không chặn thread async.
- **Nhầm hai chiều AVIF**: nhận ảnh AVIF tải lên (giải mã — vẫn không có bản nhỏ, không có bộ giải mã Rust thuần)
  khác với xuất bản AVIF (mã hoá). Mục này chỉ thêm chiều mã hoá.
- **So cỡ không cùng chất lượng**: AVIF ở chất lượng thấp luôn nhẹ hơn — chỉ so khi điểm chất lượng ngang WebP; ảnh
  có chữ dễ lộ nhoè ở AVIF chất lượng thấp, xem riêng nhóm ảnh này.

## Căn cứ pháp lý

- Luật Bảo vệ dữ liệu cá nhân 2025 (số 91/2025/QH15 + NĐ 356/2025, hiệu lực 01/01/2026, như
  `business-foundation.md` mục 3 trụ 5) — bản backup chứa dữ liệu
  cá nhân: phải có biện pháp bảo vệ (mã hoá secret của bước 33, quyền thư mục, bucket riêng tư) và
  dữ liệu đã xoá theo yêu cầu vẫn còn trong backup tới khi bản đó bị xoay vòng (`AUTOXIDE_BACKUP_KEEP`
  × interval) — ghi điều này trong DEPLOY để khách đưa vào thông báo xử lý dữ liệu. Cần đối chiếu
  văn bản gốc về thời hạn và nghĩa vụ cụ thể.
- Luật An ninh mạng 116/2025/QH15 (hiệu lực 01/7/2026) và NĐ 53/2022/NĐ-CP (lưu trữ dữ liệu người
  dùng tại Việt Nam với một số loại doanh nghiệp) — nơi đặt bucket backup đi theo vị trí dữ liệu của
  khách (khách chọn "lưu tại Việt Nam" ở bước 33 thì backup của database đó cũng phải ở Việt Nam);
  DEPLOY nhắc điều này. Điều khoản cụ thể và nghĩa vụ lưu nhật ký hệ thống: **cần đối chiếu văn bản
  gốc / người có chuyên môn pháp lý** — không chặn công việc (D-186).
- AVIF (D-232): license của crate mã hoá phải hợp MIT / Apache (sở hữu trí tuệ, Luật số hai). AV1 do AOMedia cấp
  phép bằng sáng chế miễn phí, nhưng có bên ngoài tuyên bố nhóm bằng sáng chế cho AV1 — cần đối chiếu người có
  chuyên môn pháp lý; không chặn việc đo.
- Chứng từ kế toán giữ ≥ 10 năm (Luật Kế toán 88/2015 Điều 41, D-190): xoay vòng backup KHÔNG phải
  nơi lưu chứng từ — việc giữ 10 năm nằm ở module `invoices` / `ledger` (bước 36), sổ tay ghi rõ.

## Agent làm bước này tự quyết

- Nén trong app hay để reverse proxy — quyết theo số đo (khuyến nghị: trong app, vì người cài một
  binary SQLite có thể không có proxy nén; tắt được bằng `AUTOXIDE_HTTP_COMPRESSION`). Ghi DECISIONS.
- Giá trị mặc định `AUTOXIDE_BACKUP_INCLUDE_FILES` (khuyến nghị `false`, lý do ở bảng cấu hình) và
  `INTERVAL` / `KEEP`. Ghi DECISIONS.
- Ngưỡng p95 cụ thể mỗi đường — theo số đo, ghi lý do trong BENCHMARKS.
- Đặt task sinh dữ liệu sau cargo feature hay trong `xtask` (khuyến nghị feature `bench-seed` + task
  của từng module, để module tự ghi bảng của mình). Ghi DECISIONS.
- Gauge chỉ ở tiến trình `worker` hay mọi tiến trình (khuyến nghị: `worker` / `all`, tránh N chuỗi trùng).
- Cách giảm của tìm theo ý nghĩa nếu vượt ngưỡng. Ghi DECISIONS.
- AVIF (D-232): crate, mức chất lượng / tốc độ mã hoá, và có làm hay không theo ngưỡng ở mục "Ảnh AVIF". Ghi
  DECISIONS cả khi không làm.
- Có thêm `promtool check rules` vào gate qua Docker hay không (khuyến nghị: có nếu image nhỏ và
  ghim phiên bản; nếu thêm, ghi đầu `gate.sh`).
