# Bước 35 — Gói, quyền dùng tính năng, hạn mức, thuê bao

> Lộ trình: `docs/ROADMAP.md` bước 35 · Thiết kế: `docs/research/business-foundation.md` trụ 3 (mô hình
> 3 — SaaS bán theo tính năng) · Quyết định: D-185, D-187, D-192; nền cũ D-038 (hàm mới của `Module`
> có mặc định), D-155 (sổ module), D-181 (`rows::adjust`), D-182 (`write_with_retry`), D-184
> (`reference`) · Mẫu tham khảo: Stripe Entitlements (`docs.stripe.com/billing/entitlements`).

## Mục tiêu

Xong bước này, nhà vận hành bán được phần mềm theo gói mà **không sửa code**: gói là dữ liệu (tập
module + tính năng + hạn mức + giá theo kỳ), có dùng thử, gói thêm, ghi đè riêng cho từng khách.
Kernel **cưỡng chế** theo khách: module chưa bật thì route, tool MCP, job, event của nó bị chặn;
module hỏi `has_feature`; hạn mức giữ chỗ / trả lại nguyên tử; số đo dùng được ghi cho tính tiền.
Thuê bao có vòng đời dùng thử → đang dùng → quá hạn (ân hạn) → tạm khoá (chỉ đọc) → huỷ → xoá, chạy
bằng job; hoá đơn SaaS thu qua cổng thanh toán của nhà vận hành (sau bước 36); khách tự đăng ký và
được cấp phát trọn gói (sau bước 37). Mô hình 1 (một khách, chế độ `single`) **không đổi gì**: khách
không có thuê bao và không có gói mặc định thì không bị giới hạn.

Cách nghĩ mượn Stripe Entitlements: module khai **Feature** (`lookup_key` ↔ khoá `<module>.<tên>`),
gói gắn feature như Product; ứng dụng chỉ hỏi "quyền dùng đang có" (active entitlements) của khách,
không suy từ hoá đơn; đổi quyền dùng thì phát event (như `entitlements.active_entitlement_summary.updated`).

## Bắt đầu khi

Bước 34 đã merge. Năm mục chia hai đợt: **mục 1–3** làm ngay; **mục 4 (tính tiền)** chờ bước 36 merge
(`PaymentGateway`, `invoices`, đánh số chứng từ); **mục 5 (cấp phát)** chờ bước 37 merge (KYC,
chấp nhận văn bản pháp lý). Làm hai mục cuối sau cùng, đọc `docs/steps/36-*.md`, `docs/steps/37-*.md`
và code thật để lấy tên thật — mục "Hợp đồng giả định" dưới đây là thứ bước này cần, không phải tên
chắc chắn.

Cần có sẵn: `TenantId`, `TenantContext`, khách vận hành, tạo khách / chọn vị trí dữ liệu / migrate
một database (bước 33); `Principal` có tenant và độ sâu, `OrgDirectory`, `api::Invitations` của
`auth`, module `address` (bước 34).

### Hợp đồng giả định với bước 36 (mục 4)

1. `autoxide_kernel::payment::PaymentGateway` lấy được cho **khách vận hành** (cổng thanh toán của
   nhà vận hành, thông tin kết nối trong kho secret theo khách của bước 33). Plans cần một hàm tạo lượt
   thanh toán: số tiền `Money`, mã tham chiếu (`reference`, D-184), mô tả, URL quay về → trả
   `provider_reference` và `checkout_url` (hay dữ liệu VietQR). Không cần trừ tiền tự động định kỳ.
2. Khi đối tác báo đã thu (webhook, hay `BankFeed` khớp VietQR), bước 36 **phát một event** mang
   `tenant_id` của bên nhận tiền, `reference`, `amount`, `currency`, `provider_reference` — plans
   nghe event đó (tên event lấy từ đặc tả bước 36; event bị phát lại phải vô hại).
3. `dyn Invoices` (module `invoices`) phát hành chứng từ thu phí SaaS: bên bán = khách vận hành, bên
   mua = hồ sơ thanh toán của khách thuê, các dòng (mô tả, số lượng, đơn giá `Money`, mã nhóm thuế),
   `reference`; trả ID, số chứng từ, tổng tiền gồm thuế; huỷ được. Đánh số tuần tự không nhảy số,
   thuế (qua `TaxEngine`) và hoá đơn điện tử là việc của `invoices` — plans không tự đánh số, không
   tự tính thuế, không ghi sổ cái.

### Hợp đồng giả định với bước 37 (mục 5)

1. `compliance` cho mở hồ sơ KYC trước khi có khách, gắn `reference` do plans đặt
   (`plans.signup:<id>`), và phát event "đã xác thực" / "bị từ chối" mang `reference` đó.
2. `compliance` ghi được bằng chứng chấp nhận văn bản pháp lý (điều khoản dịch vụ SaaS) theo phiên
   bản, cho một email / một khách.

## Phạm vi

Crate mới: `modules/plans` (`autoxide-plans`), mặt điều khiển. Đăng ký **sau mọi module nền nó cần**
(`address`, `organizations`, `auth`; khi bước 36 / 37 thêm `tax`, `ledger`, `invoices`, `compliance`,
`notifications` thì chèn chúng TRƯỚC `plans`) — xem "Agent tự quyết" #1.

Sửa kernel: `entitlement` (mới), `module.rs` (`features`, `limits`, `always_enabled`, `sample_data`),
`module/registry.rs` (catalog, gắn tay cầm), `http/mod.rs` (lớp chặn module / chỉ đọc), `http/mcp.rs`,
`job/worker.rs`, `event/dispatcher.rs`, `module_catalog.rs` (sổ module thêm feature, limit, module
phụ thuộc). Sửa module: `organizations`, `auth`, `files` (khai và gọi hạn mức), `content` (tính năng),
`notes` (ví dụ), module nền khai `always_enabled`. CLI `autoxide tenant provision`. Tài liệu:
`docs/MODULE_GUIDE.md` + `.vi.md` (mục "Tính năng và hạn mức"), `docs/PLANS.md` + `PLANS.vi.md`
(hướng dẫn cho nhà vận hành SaaS), `docs/DEPLOY.md` + `.vi.md`, `CHANGELOG.md`, `.env.example`,
`docs/openapi.json`.

NGOÀI phạm vi: hạn mức theo nút (vd số nhân sự mỗi cửa hàng); trừ tiền tự động định kỳ bằng thẻ đã
lưu; hoàn tiền tỉ lệ khi hạ gói / huỷ; mã giảm giá cho thuê bao; thuê bao định kỳ cho **người mua**
của khách ("sản phẩm thuê bao" — để sau, D-194); báo cáo doanh thu SaaS (bước 38).

## Thiết kế

### Kiểu và trait

Phác thảo — tra API thật bằng skill `check-library-api`, để `cargo check` xác nhận.

```rust
// autoxide_kernel::entitlement (mới)
pub struct Feature { key: &'static str, description: &'static str }   // `<module>.<tên>`
impl Feature { pub const fn new(key: &'static str, description: &'static str) -> Self; }

pub enum LimitKind {
    Count,                          // số bản ghi đang có (sản phẩm, nút, nhân sự)
    Amount { unit: &'static str },  // lượng đang dùng (`bytes`)
    Metered { period: Period },     // lượng dùng trong kỳ, về 0 đầu kỳ (request, token AI)
}
pub enum Period { Month }
pub struct Limit { key: &'static str, description: &'static str, kind: LimitKind,
                   counter: Option<Arc<dyn UsageCounter>> }
impl Limit {
    pub const fn count(key: &'static str, description: &'static str) -> Self;
    pub const fn amount(key: &'static str, description: &'static str, unit: &'static str) -> Self;
    pub const fn metered(key: &'static str, description: &'static str, period: Period) -> Self;
    /// Bộ đếm thật trong database của module, để job đối soát sửa số đang dùng bị lệch.
    pub fn counted_by(self, counter: Arc<dyn UsageCounter>) -> Self;
}
#[async_trait] pub trait UsageCounter: Send + Sync {
    async fn count(&self, tenant: TenantId) -> Result<i64, Error>;
}

pub enum TenantAccess { Full, ReadOnly }          // ReadOnly: thuê bao `suspended` / `canceled`

/// Quyền dùng hiện tại của một khách (đã gộp gói, gói thêm, ghi đè).
pub struct TenantEntitlements {
    pub unrestricted: bool,         // không thuê bao, không gói mặc định; hoặc khách vận hành
    pub access: TenantAccess,
    pub modules: BTreeSet<String>, pub features: BTreeSet<String>,
    pub limits: BTreeMap<String, Option<i64>>,     // None = không giới hạn
}

#[async_trait] pub trait Entitlements: Send + Sync {
    async fn snapshot(&self, tenant: TenantId) -> Result<Arc<TenantEntitlements>, Error>;
    /// Giữ chỗ `amount` của hạn mức `limit` theo `reference` (vd `catalog.product:<id>`), có hạn
    /// giữ. Gọi lại cùng `reference` + cùng `amount` → không giữ thêm; khác `amount` →
    /// `reference_reused`. Vượt hạn mức → `limit_exceeded`.
    async fn reserve(&self, tenant: TenantId, limit: &Limit, amount: i64, reference: &str)
        -> Result<(), Error>;
    /// Chỗ đã giữ thành vĩnh viễn (bản ghi đã commit). Không có chỗ giữ → lỗi nội bộ.
    async fn confirm(&self, tenant: TenantId, limit: &Limit, reference: &str) -> Result<(), Error>;
    /// Trả lại; gọi nhiều lần hay với `reference` lạ đều `Ok` (bên gọi hay retry).
    async fn release(&self, tenant: TenantId, limit: &Limit, reference: &str) -> Result<(), Error>;
    /// Ghi lượng dùng của hạn mức `Metered` (không trùng theo `reference`).
    async fn record_usage(&self, tenant: TenantId, limit: &Limit, quantity: i64, reference: &str)
        -> Result<(), Error>;
}

/// Tay cầm kernel đưa cho MỌI module lúc `start`, kể cả module đăng ký trước `plans`; trỏ tới
/// `dyn Entitlements` khi `plans` công bố, không có thì "không giới hạn". Clone rẻ.
#[derive(Clone)] pub struct EntitlementsHandle { /* Arc<OnceLock<Arc<dyn Entitlements>>> */ }
impl EntitlementsHandle {
    pub async fn module_enabled(&self, tenant: TenantId, module: &str) -> Result<bool, Error>;
    pub async fn has_feature(&self, tenant: TenantId, feature: &Feature) -> Result<bool, Error>;
    pub async fn require_feature(&self, tenant: TenantId, feature: &Feature) -> Result<(), Error>;
    // reserve / confirm / release / record_usage: chuyển tiếp, hoặc Ok khi không có bên cung cấp
}

// autoxide_kernel::module::Module (thêm, đều có mặc định — D-038)
fn features(&self) -> Vec<Feature> { Vec::new() }
fn limits(&self) -> Vec<Limit> { Vec::new() }
/// Module nền không bao giờ bị gói chặn (kernel, mail, files, auth, audit, idempotency, webhooks,
/// address, organizations, plans; sau này tax, ledger, invoices, compliance, notifications).
fn always_enabled(&self) -> bool { false }
/// Dữ liệu mẫu khi cấp phát khách mới (chạy lại được, không trùng).
fn sample_data(&self) -> Option<Arc<dyn SampleData>> { None }
// StartContext::entitlements() -> EntitlementsHandle; StartContext::entitlement_catalog()
// -> &EntitlementCatalog (feature + limit của mọi module, kiểm tên lúc khởi động như permission).

// autoxide_plans::api — cho CLI và cho module sau
#[async_trait] pub trait Provisioning: Send + Sync {
    async fn provision(&self, request: ProvisionRequest) -> Result<ProvisioningStatus, Error>;
}
```

**Gộp quyền dùng của một khách** (một hàm thuần, có test riêng): không có thuê bao → gói có
`is_default`; không có gói mặc định → `unrestricted`. Khách vận hành luôn `unrestricted`. Module =
module của gói chính ∪ của các gói thêm, rồi áp ghi đè; tính năng như vậy; hạn mức = của gói chính
+ Σ (của gói thêm × số lượng), `None` (không giới hạn) thắng mọi số, rồi ghi đè thay hẳn. Ghi đè hết
hạn (`expires_at`) thì bỏ qua. Module `always_enabled` luôn bật dù gói không nêu.

**Kernel cưỡng chế** (đọc `snapshot` qua tay cầm; cache trong tiến trình):

| Chỗ | Module chưa bật | Khách `ReadOnly` |
|---|---|---|
| Route REST của module (lớp bọc router của từng module khi `nest`, sau extractor ngữ cảnh khách) | 403 `module_not_enabled` | `POST`/`PUT`/`PATCH`/`DELETE` → 403 `tenant_read_only`; `GET`/`HEAD` qua |
| Tool MCP | không có trong `tools/list`; `tools/call` → `module_not_enabled` | tool không `read_only` bị chặn như trên |
| Job của module (worker, theo `tenant_id` của job) | hoãn `AUTOXIDE_JOBS_BLOCKED_DELAY_SECONDS`, không tính lượt thử | hoãn như bên trái |
| Event giao cho subscription của module | bỏ qua (đánh dấu đã giao, tăng số đo) | bỏ qua |
| Gọi trait của module qua `dyn Trait` | kernel không chặn được — module cung cấp tự kiểm bằng tay cầm trong hàm nhận `TenantId` | — |

Chỉ áp cho module không `always_enabled`. Request không có ngữ cảnh khách (route công khai của mặt
điều khiển) không bị chặn ở lớp này.

### Bảng dữ liệu

Mọi bảng `plans_*` ở **mặt điều khiển** (cùng database với sổ khách hàng). Bảng theo khách có
`tenant_id`, unique / index bắt đầu bằng nó. Tiền: `<tên>_minor` (`big_integer`) + `<tên>_currency`
(`string_len(3)`). Thời gian UTC (`timestamp_with_time_zone`). Mọi migration có `down`.

- `plans_plan`: `id`, `code` (unique, `[a-z][a-z0-9_-]{0,63}`), `name`, `description`, `kind`
  (`base` / `addon`), `status` (`draft` / `active` / `retired`), `is_default` (chỉ một gói `base`
  đang `active` mang cờ — kiểm trong service dưới khoá), `trial_days` (`integer`, 0 = không),
  `grace_days` (mặc định 7), `suspend_days` (30), `retention_days` (90), `renewal_lead_days` (7),
  `placement` (`shared` / `dedicated`), `requires_kyc` (`boolean`), `sample_data` (`boolean`),
  `created_at`, `updated_at`.
- `plans_plan_module` (`plan_id`, `module`), `plans_plan_feature` (`plan_id`, `feature`),
  `plans_plan_limit` (`plan_id`, `limit_key`, `max_value` `big_integer` null = không giới hạn); PK
  hai cột đầu.
- `plans_price`: `id`, `plan_id`, `kind` (`recurring` / `metered`), `interval` (`month` / `year`),
  `interval_count`, `amount_minor`, `amount_currency`, `limit_key` (với `metered`), `included`
  (`big_integer`, lượng miễn phí trong kỳ), `tax_category` (mã nhóm thuế của bước 36, null tới khi có),
  `active`, `created_at`. Giá không sửa: đổi giá = thêm dòng mới, tắt dòng cũ; thuê bao giữ giá cũ tới
  khi đổi gói.
- `plans_subscription`: `tenant_id` (unique — mỗi khách một dòng, đi suốt vòng đời), `id`, `plan_id`,
  `price_id`, `status` (`trialing` / `active` / `past_due` / `suspended` / `canceled` / `deleted`),
  `current_period_start`, `current_period_end`, `trial_ends_at`, `grace_ends_at`, `suspended_at`,
  `canceled_at`, `delete_after`, `cancel_at_period_end` (`boolean`), `scheduled_plan_id`,
  `scheduled_price_id` (hạ gói đợi hết kỳ), `trial_used` (`boolean`), `billing_email`,
  `created_at`, `updated_at` (`If-Match`). Index `(status, current_period_end)` cho job.
- `plans_subscription_addon`: `tenant_id`, `plan_id` (gói `addon`), `quantity`, `created_at`; PK
  `(tenant_id, plan_id)`.
- `plans_override`: `tenant_id`, `id`, `kind` (`module` / `feature` / `limit`), `key`, `enabled`
  (`boolean`, với module / feature), `max_value` (với limit), `reason`, `expires_at`, `created_by`,
  `created_at`; unique `(tenant_id, kind, key)`.
- `plans_usage`: `tenant_id`, `limit_key`, `period_start` (`string_len(10)`, `0000-00-00` cho
  `Count` / `Amount`), `used` (`big_integer`), `updated_at`; PK ba cột. Đây là cột `rows::adjust`
  cộng / trừ.
- `plans_reservation`: `tenant_id`, `id`, `limit_key`, `reference` (`string_len(200)`), `amount`,
  `status` (`held` / `confirmed` / `released`), `expires_at`, `created_at`, `updated_at`; unique
  `(tenant_id, limit_key, reference)`; index `(status, expires_at)`.
- `plans_usage_record`: `tenant_id`, `id`, `limit_key`, `period_start`, `quantity`, `reference`,
  `recorded_at`; unique `(tenant_id, limit_key, reference)` — sổ chỉ thêm, nguồn cho tính tiền theo
  lượng dùng.
- `plans_billing_profile` (mục 4): `tenant_id` (PK), `legal_name`, `tax_code`, `email`, `phone`,
  cột địa chỉ theo quy ước `address_*` của module `address`, `updated_at`.
- `plans_billing_period` (mục 4): `tenant_id`, `id`, `kind` (`initial` / `renewal` / `proration` /
  `metered`), `plan_id`, `price_id`, `period_start`, `period_end`, `amount_minor`,
  `amount_currency`, `reference` (unique `(tenant_id, reference)`, dạng
  `plans:<subscription_id>:<period_start>:<kind>`), `invoice_id`, `invoice_number`, `status`
  (`open` / `paid` / `void`), `due_at`, `paid_at`, `provider_reference`, `created_at`, `updated_at`.
- `plans_signup` (mục 5): `id`, `email`, `organization_name`, `plan_id`, `price_id`, `status`
  (`pending_email` / `awaiting_kyc` / `provisioning` / `completed` / `rejected` / `expired`),
  `provisioning_id`, `reference` (unique, D-184), `terms_version`, `created_at`, `updated_at`,
  `expires_at`; index `(email, status)`.
- `plans_provisioning` (mục 5): `id`, `tenant_id` (UUIDv7 sinh trước khi tạo khách — để chạy lại
  không tạo khách thứ hai), `signup_id`, `parent_tenant_id`, `tenant_name`, `plan_id`, `price_id`,
  `placement`, `admin_email`, `admin_name`, `sample_data`, `step` (`created` → `tenant_created` →
  `database_ready` → `migrated` → `seeded` → `subscribed` → `invited` → `done`), `status`
  (`running` / `failed` / `done`), `last_error`, `reference` (unique), `created_by`, `created_at`,
  `updated_at`.

**Giữ chỗ nguyên tử** (một transaction ghi trên mặt điều khiển, bọc `write_with_retry`):
1. Thêm `plans_reservation` (`held`, `expires_at = now + AUTOXIDE_PLANS_HOLD_MINUTES`); trùng unique →
   đọc dòng có sẵn: cùng `amount` và chưa `released` → trả `Ok` (gọi lại); khác → `reference_reused`.
2. Bảo đảm có dòng `plans_usage` (`INSERT … ON CONFLICT DO NOTHING` — cả hai database có, SeaORM
   `OnConflict::do_nothing`, đã dùng ở `newsletter`).
3. `rows::adjust::<usage::Entity, _>(&tx, Used, filter, +amount, Bounds { floor: 0, ceiling: max })`
   — `max = None` thì `i64::MAX`; trả 0 dòng → rollback, `limit_exceeded` kèm `limit`, `max`, `used`.

`release`: khoá dòng giữ chỗ (`rows::lock_one`), nếu `held` / `confirmed` → `released` và
`adjust(-amount, Bounds::NON_NEGATIVE)`; trả 0 dòng (số đang dùng đã lệch) → ghi cảnh báo, để job đối
soát sửa. Job `plans.expire_holds` trả các chỗ `held` quá hạn. Job `plans.reconcile_usage` (hằng ngày)
gọi `UsageCounter` của từng hạn mức `Count` / `Amount` cho từng khách có thuê bao, đặt lại `used` =
số đếm thật + Σ chỗ `held` còn hạn, ghi số đo độ lệch. Metered: `record_usage` thêm dòng
`plans_usage_record` (trùng `reference` → bỏ qua) và `adjust` dòng của kỳ hiện tại; vượt hạn mức đặt
trong gói → `quota_exhausted` (429, `Retry-After` = giây tới đầu kỳ sau).

**Hạn mức và tính năng khai ở bước này** (ví dụ thật cho mọi chỗ cưỡng chế):

| Khoá | Loại | Ai khai, gọi ở đâu |
|---|---|---|
| `organizations.units` | Count (nút không `archived`, trừ gốc) | `organizations`: tạo nút / bỏ lưu trữ giữ chỗ (`organizations.unit:<id>`), lưu trữ trả lại |
| `auth.members` | Count (nhân sự khác nhau có membership ở khách) | `auth`: membership đầu tiên của một người giữ chỗ (`auth.member:<user_id>`), membership cuối bị gỡ trả lại |
| `files.storage_bytes` | Amount `bytes` | `files`: tải lên giữ chỗ theo cỡ, xoá trả lại |
| `kernel.api_requests` | Metered tháng | kernel: đếm request theo khách trong bộ nhớ, `record_usage` mỗi 60 giây (`reference` = máy chủ + mốc phút) |
| `auth.api_keys` | Feature | `auth`: tạo API key cần tính năng này khi khách bị giới hạn |
| `content.ai_suggestions` | Feature | `content`: gợi ý AI cần tính năng này |
| `notes.notes` | Count | ví dụ trong `notes` và hướng dẫn viết module |

### Vòng đời thuê bao

| Từ | Sự việc | Sang |
|---|---|---|
| — | gán gói (API / cấp phát): gói có `trial_days` và khách chưa dùng thử | `trialing` |
| — | gán gói không dùng thử | `active` (giá > 0: kỳ đầu `initial` phải trả, chưa trả tới `due_at` → `past_due`) |
| `trialing` | hết dùng thử: giá 0 → `active`; giá > 0 → kỳ đầu `renewal` | `active` / `past_due` |
| `active` | trước hết kỳ `renewal_lead_days`: lập kỳ thanh toán kế, gửi mail | `active` |
| `active` | hết kỳ, kỳ kế đã trả | `active` (kỳ mới) |
| `active` | hết kỳ, chưa trả | `past_due` (`grace_ends_at = now + grace_days`) |
| `past_due` / `suspended` | nhận tiền | `active` |
| `past_due` | hết ân hạn | `suspended` (chỉ đọc) |
| `suspended` | quá `suspend_days` | `canceled` (`delete_after = now + retention_days`) |
| `trialing` / `active` / `past_due` | khách huỷ: `cancel_at_period_end`, hết kỳ | `canceled` |
| bất kỳ (chưa `deleted`) | nhà vận hành huỷ ngay | `canceled` |
| `canceled` | đăng ký lại trước `delete_after` | `active` (kỳ `initial`) |
| `canceled` | tới `delete_after` | `deleted` (gọi xoá khách của bước 33; phần kế toán giữ theo bước 36) |

`canceled` vẫn chỉ đọc (khách xuất dữ liệu qua API; nhà vận hành có `autoxide copy` của bước 33).
Nâng gói: hiệu lực ngay, lập kỳ `proration` = (giá mới − giá cũ) × số giây còn lại / số giây của
kỳ, làm tròn xuống đơn vị nhỏ nhất (tính bằng `i128`, hay `rust_decimal` khi bước 36 có). Hạ gói:
đặt `scheduled_*`, áp khi hết kỳ; đang dùng quá hạn mức mới thì không xoá gì, chỉ chặn giữ chỗ mới
và báo `over_limit` trong `GET /entitlements`. Thu hồi module: dữ liệu giữ nguyên; route bị chặn;
dữ liệu chỉ mất khi khách bị xoá. Mọi chuyển trạng thái: khoá dòng thuê bao (`rows::lock_one`), kiểm
trạng thái hiện tại rồi mới đổi — job và event thanh toán đến cùng lúc không đè nhau.

### API REST (`/api/v1/plans`)

"Vận hành" = permission giữ ở khách vận hành (bước 34); "khách" = permission ở khách hiện tại.
Mọi endpoint có `///` và `description` từng response; danh sách bằng `fetch_page`; sửa có `If-Match`.

| Method, path | Permission | Ghi chú |
|---|---|---|
| `GET /catalog` | công khai | gói `base` / `addon` đang `active` kèm giá — cho trang đăng ký |
| `GET /plans` · `POST /plans` · `GET/PUT /plans/{id}` · `POST /plans/{id}/retire` | vận hành `plans.plans.read` / `plans.plans.manage` | kiểm module / feature / limit với danh mục kernel (lạ → 422 trường `unknown`); module bật mà thiếu module nó cần (sổ module ghi ai `require` ai) → 422 `plans.module_dependency_missing`; không xoá gói đã có thuê bao — chỉ `retire` |
| `POST /plans/{id}/prices` · `POST /prices/{id}/deactivate` | vận hành `plans.plans.manage` | |
| `GET /entitlements` | đăng nhập (khách) | quyền dùng hiện tại: module, tính năng, hạn mức kèm `used`, `over_limit`, `access` — giao diện dùng để ẩn tính năng |
| `GET /subscription` · `GET /usage?period=` | khách `plans.billing.read` | |
| `POST /subscription/change` `{plan_id, price_id, addons?, reference}` · `POST /subscription/cancel` `{at_period_end}` · `POST /subscription/resume` | khách `plans.billing.manage` | `reference` theo D-184 |
| `GET /subscriptions?status=&plan_id=` · `GET /tenants/{tenant_id}/subscription` | vận hành `plans.subscriptions.read` | |
| `PUT /tenants/{tenant_id}/subscription` `{plan_id, price_id, trial?}` · `POST /tenants/{tenant_id}/subscription/cancel` | vận hành `plans.subscriptions.manage` | |
| `GET/POST /tenants/{tenant_id}/overrides` · `DELETE /tenants/{tenant_id}/overrides/{id}` | vận hành `plans.subscriptions.manage` | `reason` bắt buộc |
| `GET /tenants/{tenant_id}/usage` | vận hành `plans.subscriptions.read` | |
| `GET/PUT /billing-profile` | khách `plans.billing.read` / `.manage` | địa chỉ qua `AdministrativeUnits::validate` (mục 4) |
| `GET /billing-periods` · `GET /billing-periods/{id}` · `POST /billing-periods/{id}/pay` | khách `plans.billing.read` / `.manage` | `pay` → `checkout_url` từ `PaymentGateway` (mục 4) |
| `POST /billing-periods/{id}/mark-paid` `{provider_reference, paid_at}` · `POST /billing-periods/{id}/void` | vận hành `plans.subscriptions.manage` | thu tay (chuyển khoản đối soát tay) |
| `POST /signups` `{email, organization_name, plan_code, price_id?, terms_version, reference}` | công khai, 202 | tắt mặc định; chống spam bằng `kernel::antispam` + throttle theo IP; luôn xếp job (không lộ email đã có) (mục 5) |
| `POST /signups/confirm` `{token}` · `GET /signups/{id}?token=` | công khai | (mục 5) |
| `POST /provisionings` · `GET /provisionings/{id}` · `POST /provisionings/{id}/retry` | vận hành `plans.provision` | (mục 5) |

Sổ module (`GET /api/v1/kernel/modules/{name}`, D-155) thêm `features`, `limits`, `always_enabled`,
`requires`. OpenAPI: kernel thêm response 403 (`module_not_enabled`, `tenant_read_only`) có mô tả cho
mọi operation của module không `always_enabled` (trong `openapi::finish`).

### Tool MCP · Event · Job/lịch · Cấu hình · Permission · Mã lỗi

**Tool MCP** (cùng service với REST, cùng permission): `plans.list_plans`, `get_plan`, `create_plan`,
`update_plan`, `retire_plan`, `get_entitlements` (`read_only`), `get_subscription`, `change_subscription`,
`assign_subscription`, `set_override`, `list_usage`, `get_provisioning`. Không có tool cho `pay`,
`mark-paid`, đăng ký công khai.

**Event:** `plans.plan_created`, `plan_updated`, `plan_retired`, `plans.subscription_created`,
`subscription_status_changed` (`from`, `to`), `subscription_plan_changed`, `plans.override_set`,
`override_removed`, `plans.entitlements_changed` (một event mỗi khách khi quyền dùng của khách đó đổi
vì thuê bao / gói thêm / ghi đè; đổi nội dung một gói chỉ phát `plan_updated`),
`plans.billing_period_opened`, `billing_period_paid`, `billing_period_voided`,
`plans.signup_created`, `signup_completed`, `plans.tenant_provisioned`. Bên nhận chịu nhận lại.

**Job / lịch:** `plans.lifecycle` (mỗi 5 phút: chuyển trạng thái tới hạn theo bảng trên, mỗi thuê
bao một transaction, lô 100), `plans.renewals` (mỗi giờ: lập kỳ thanh toán sắp tới), `plans.reminders`
(mỗi giờ: mail nhắc trước hạn theo `AUTOXIDE_PLANS_REMINDER_DAYS`, khi quá hạn, mỗi 3 ngày trong ân
hạn — không gửi trùng: ghi mốc nhắc vào `plans_billing_period`), `plans.expire_holds` (mỗi phút),
`plans.reconcile_usage` (hằng ngày), `plans.provision` (mục 5), `plans.send_signup_link` (mục 5).
Mail qua `dyn Mailer` với `MailTemplate` có bản dịch; khi bước 37 có `notifications` thì đổi sang
`dyn Notifier` (tin giao dịch).

**Cấu hình:** `AUTOXIDE_PLANS_CACHE_SECONDS` (30; 0–300 — quyền dùng đổi có hiệu lực ở máy chủ khác sau
tối đa chừng này), `AUTOXIDE_PLANS_HOLD_MINUTES` (15; 1–1440), `AUTOXIDE_PLANS_REMINDER_DAYS` (`7,1`),
`AUTOXIDE_PLANS_SIGNUP` (`false`), `AUTOXIDE_PLANS_SIGNUP_URL` (trang có `{token}`),
`AUTOXIDE_PLANS_PAYMENT_RETURN_URL`; kernel: `AUTOXIDE_JOBS_BLOCKED_DELAY_SECONDS` (3600).

**Permission:** `plans.plans.read`, `plans.plans.manage`, `plans.subscriptions.read`,
`plans.subscriptions.manage`, `plans.provision` (vận hành); `plans.billing.read`,
`plans.billing.manage` (khách). Không permission nào `scoped()` (thuê bao là việc toàn khách).

**Mã lỗi:**

| Mã | HTTP | Câu tiếng Việt |
|---|---|---|
| `module_not_enabled` | 403 | Gói hiện tại của bạn chưa có module `{module}`. Nâng gói để dùng. |
| `feature_not_enabled` | 403 | Gói hiện tại của bạn chưa có tính năng `{feature}`. |
| `limit_exceeded` | 403 | Đã dùng hết hạn mức `{limit}` ({used}/{max}). Nâng gói hoặc giải phóng bớt. |
| `quota_exhausted` | 429 | Đã dùng hết `{limit}` của kỳ này; kỳ mới bắt đầu sau {retry_after} giây. |
| `tenant_read_only` | 403 | Thuê bao đang tạm khoá hoặc đã huỷ: chỉ xem được dữ liệu. Thanh toán để mở lại. |
| `plans.plan_not_found` | 404 | Không có gói `{id}`. |
| `plans.plan_code_taken` | 409 | Mã gói `{code}` đã dùng. |
| `plans.plan_not_active` | 409 | Gói `{code}` không còn bán. |
| `plans.plan_in_use` | 409 | Gói đang có khách dùng: chỉ ngừng bán được, không xoá được. |
| `plans.module_dependency_missing` | 422 | Module `{module}` cần module `{requires}` trong cùng gói. |
| `plans.price_not_found` | 404 | Không có giá `{id}` của gói này. |
| `plans.subscription_not_found` | 404 | Khách hàng này chưa có thuê bao. |
| `plans.invalid_transition` | 409 | Thuê bao đang `{status}` nên không làm được việc này. |
| `plans.trial_already_used` | 409 | Khách hàng này đã dùng thử một lần. |
| `plans.addon_not_allowed` | 422 | `{code}` không phải gói thêm. |
| `plans.billing_period_not_found` | 404 | Không có kỳ thanh toán `{id}`. |
| `plans.billing_period_not_payable` | 409 | Kỳ thanh toán này đã trả hoặc đã huỷ. |
| `plans.billing_profile_required` | 409 | Hãy điền hồ sơ thanh toán (tên, email) trước khi thanh toán. |
| `plans.payment_unavailable` | 503 | Chưa cấu hình cổng thanh toán của nhà vận hành. |
| `plans.signup_disabled` | 404 | Máy chủ này không mở đăng ký khách hàng mới. |
| `plans.signup_token_invalid` | 400 | Link xác nhận sai, đã dùng hoặc đã hết hạn. |
| `plans.provisioning_not_found` | 404 | Không có lượt cấp phát `{id}`. |
| `plans.provisioning_failed` | 409 | Cấp phát dừng ở bước `{step}`: {reason}. Sửa rồi chạy lại. |

## Thứ tự làm

1. **Kernel: hợp đồng quyền dùng.** `entitlement` (`Feature`, `Limit`, `UsageCounter`,
   `Entitlements`, `TenantEntitlements`, `EntitlementsHandle`, `EntitlementCatalog`); `Module::{features,
   limits, always_enabled, sample_data}`; `StartContext::{entitlements, entitlement_catalog}`; registry
   gắn tay cầm sau `start`; sổ module thêm feature / limit / `requires`; module nền khai
   `always_enabled`. Không có `plans` thì mọi thứ chạy như cũ (test). → ROADMAP mục 1.
2. **Module `plans`: gói là dữ liệu** — bảng gói, giá, gói thêm, ghi đè, hàm gộp quyền dùng, REST +
   MCP quản trị gói, `GET /entitlements`, event. → mục 1.
3. **Kernel cưỡng chế**: lớp route, MCP, worker, dispatcher; mã lỗi; số đo
   `entitlement_denials_total{kind}` (nhãn chỉ `module` / `feature` / `limit` / `read_only` — không
   dùng tenant hay tên module làm nhãn). → mục 2.
4. **Hạn mức**: `plans_usage`, `plans_reservation`, `plans_usage_record`; giữ chỗ / xác nhận / trả lại;
   job hết hạn giữ, đối soát; nối `organizations.units`, `auth.members`, `files.storage_bytes`,
   `kernel.api_requests`, `auth.api_keys`, `content.ai_suggestions`, ví dụ `notes`; hướng dẫn viết
   module (EN + VI). → mục 2.
5. **Vòng đời thuê bao**: trạng thái, job `plans.lifecycle`, đổi / huỷ / tiếp tục, chỉ đọc, xoá sau thời
   hạn lưu, mail nhắc (chưa có thanh toán: kỳ `open` chỉ được `mark-paid`). → mục 3.
6. Đo đường nóng (`scripts/loadtest.sh` trước bước 3 và sau bước 5; một khách có thuê bao, một khách
   không), ghi DECISIONS. Commit, gate đầy đủ. **Chưa phát hành** — bước chưa trọn.
7. **(sau bước 36) Tính tiền SaaS**: hồ sơ thanh toán, kỳ thanh toán → `dyn Invoices`, `pay` qua
   `PaymentGateway` của khách vận hành, nghe event đã thu, tính tiền theo lượng dùng (`metered`). → mục 4.
8. **(sau bước 37) Cấp phát**: đăng ký công khai, xác nhận email, KYC khi gói yêu cầu, chấp nhận điều
   khoản, `plans.provision` từng bước chạy lại được (tạo khách với ID sinh trước → vị trí dữ liệu theo
   gói → migrate → dữ liệu mẫu của module được bật → thuê bao → mời quản trị bằng `api::Invitations`
   với role `admin` của khách mới); `autoxide tenant provision --name … --admin-email … --plan …
   [--price …] [--parent …] [--placement …] [--sample-data]` qua `api::Provisioning`. → mục 5.
9. `docs/PLANS.md` + `PLANS.vi.md`, `docs/openapi.json`, `.env.example`, `CHANGELOG.md`, ROADMAP, gate
   đầy đủ, phát hành (`docs/RELEASING.md`).

## Test bắt buộc

Mọi test chạy trên SQLite và Postgres; test HTTP / MCP dựng bằng `TestApp::builder`.

- Gộp quyền dùng (test thuần): `khong_thue_bao_khong_goi_mac_dinh_la_khong_gioi_han`,
  `khach_van_hanh_luon_khong_gioi_han`, `goi_them_cong_han_muc_va_none_thang`,
  `ghi_de_thay_han_va_het_han_thi_bo`, `module_always_enabled_luon_bat`.
- Cưỡng chế: `route_module_chua_bat_tra_403_module_not_enabled`, `tool_mcp_module_chua_bat_bi_an_va_bi_chan`,
  `job_module_chua_bat_bi_hoan_khong_tinh_luot_thu` (bật lại → job chạy),
  `event_cho_module_chua_bat_bi_bo_qua`, `chi_doc_chan_ghi_cho_doc`, `tool_read_only_van_chay_khi_chi_doc`,
  `doi_quyen_dung_co_hieu_luc_sau_cache`, `che_do_single_khong_doi_gi` (mọi test cũ của module xanh
  khi có `plans` đăng ký mà không có gói).
- **Đồng thời:** `giu_cho_dong_thoi_khong_vuot_han_muc` (40 request, max 33 → đúng 33 thành công,
  `used = 33`); `giu_cho_cung_reference_dong_thoi_chi_mot_lan` (16 request); `tra_lai_dong_thoi_chi_tru_mot_lan`;
  `giu_va_tra_xen_ke_tong_khong_lech` (200 thao tác ngẫu nhiên, `used` = Σ chỗ còn giữ);
  `ghi_luong_dung_trung_reference_dong_thoi`; `chuyen_trang_thai_va_nhan_tien_dong_thoi` (job
  lifecycle và event thanh toán cùng lúc: kết quả là `active`, không mất kỳ);
  `cap_phat_chay_hai_lan_cung_luc_mot_khach` (mục 5).
- Hạn mức: `giu_cho_het_han_duoc_tra`, `doi_soat_sua_so_lech`, `ha_goi_qua_han_muc_chi_chan_moi`,
  `metered_ve_0_dau_ky`, `vuot_quota_tra_429_retry_after`.
- Vòng đời: mỗi dòng của bảng vòng đời một test (đồng hồ giả: truyền `now` vào service), gồm
  `qua_han_an_han_thanh_tam_khoa`, `tam_khoa_qua_han_thanh_huy`, `huy_qua_thoi_han_luu_thanh_xoa`,
  `nang_goi_lap_ky_tinh_ti_le_lam_tron_xuong`, `ha_goi_doi_het_ky`, `mail_nhac_khong_gui_trung`.
- Rò rỉ: bài rò rỉ giữa khách của bước 33 chạy cho `plans` (khách B không đọc được thuê bao, kỳ
  thanh toán, lượng dùng của khách A; endpoint vận hành từ chối người của khách thường).
- Mục 4: `thanh_toan_bang_cong_gia_dat_ky_thanh_da_tra`, `event_da_thu_den_hai_lan_vo_hai`,
  `so_tien_khac_khong_danh_dau_da_tra`, `ky_thanh_toan_tao_mot_chung_tu` (bộ giả `invoices` / cổng
  của bước 36). Mục 5: `dang_ky_tra_loi_nhu_nhau_du_email_da_co`, `goi_can_kyc_doi_event_xac_thuc`,
  `cap_phat_dung_giua_chung_chay_lai_tiep_buoc`, `cap_phat_shared_va_dedicated` (SQLite: file mới;
  Postgres: database mới), `quan_tri_vien_nhan_loi_moi`.

## Tiêu chí xong

- [ ] Mọi mục của bước 35 trong ROADMAP là `[x]` (mục 4, 5 sau khi 36, 37 merge); gate đầy đủ xanh trên
      SQLite và Postgres.
- [ ] Không có `plans`, hay có `plans` mà không có gói mặc định: mọi test cũ xanh, `/api/v1` không đổi
      (gate chặn thay đổi phá vỡ của bước 31 xanh; response 403 mới chỉ là thêm).
- [ ] `cargo xtask check` xanh (bảng `plans_*` đã phân loại mặt điều khiển; mã lỗi có bản dịch; crate
      mới đúng luật ranh giới).
- [ ] `docs/openapi.json`, `.env.example` sinh lại; `docs/PLANS.md` + `PLANS.vi.md` (tạo gói, gói mặc
      định, ghi đè, vòng đời, cấp phát, thanh toán thủ công) và mục "Tính năng và hạn mức" của
      `MODULE_GUIDE` (EN + VI, cùng nội dung).
- [ ] DECISIONS: cưỡng chế ở kernel (tay cầm gắn muộn, `always_enabled`, hành vi job / event); khách
      không thuê bao = không giới hạn; giữ chỗ có `confirm`; HTTP 403 cho quyền dùng, 429 cho quota;
      vòng đời và số ngày mặc định; số đo đường nóng; thứ tự đăng ký `plans`; (mục 4) căn cứ thuế và
      hoá đơn; (mục 5) cấp phát.
- [ ] `CHANGELOG.md` `[Unreleased]`; ROADMAP đánh dấu; phát hành theo `docs/RELEASING.md` khi cả năm
      mục xong.

## Chỗ dễ sai

- **Tràn và âm:** mọi cộng / trừ `used` qua `rows::adjust` (điều kiện ở `WHERE`, D-181) — không đọc
  rồi ghi. `amount ≤ 0` ở `reserve` là lỗi kiểm đầu vào. Proration nhân trước chia sau bằng `i128`.
- **Cache quyền dùng:** route kiểm trên snapshot cache tới `AUTOXIDE_PLANS_CACHE_SECONDS` — chấp nhận
  cho bật / tắt module, nhưng **hạn mức không dựa vào cache**: `reserve` luôn kiểm trong transaction
  (`max` đọc từ snapshot, phần nguyên tử ở `adjust`). Hạ hạn mức đúng lúc đang giữ chỗ có thể lọt
  một lần — ghi rõ, đối soát sửa.
- **Giữ chỗ khác transaction của module:** `reserve` → transaction của module → `confirm`; transaction
  module lỗi thì `release`; tiến trình chết giữa chừng thì chỗ `held` hết hạn tự trả, còn bản ghi đã
  commit mà chưa `confirm` thì đối soát đếm lại. Không gọi `reserve` bên trong transaction ghi của
  module trên cùng database (SQLite: một bên ghi — tự khoá chính mình ở chế độ `single`).
- **Job bị hoãn mãi:** khách huỷ hẳn thì job của module bị chặn hoãn mãi tới khi khách bị xoá — xoá
  khách phải xoá cả job của khách (bước 33). Hoãn không tính lượt thử, nên không vào dead-letter.
- **Event thanh toán đến hai lần / đến trước khi kỳ kịp ghi:** khớp theo `reference`; không thấy kỳ
  thì trả lỗi để dispatcher thử lại (D-054), không bỏ.
- **Đồng hồ:** job và service nhận `now` từ ngoài để test được; mọi mốc lưu UTC; "tháng" của kỳ thanh
  toán tính theo lịch (`chrono` `checked_add_months`), ngày 31 → cuối tháng sau.
- **Khách vận hành tự khoá mình:** không bao giờ áp gói / chỉ đọc cho khách vận hành; test riêng.
- **Một gói mặc định:** hai request đánh dấu `is_default` cùng lúc — khoá bằng `rows::lock_one` trên
  một dòng cố định (vd bảng `plans_settings` một dòng) hay unique có điều kiện tự viết; test đồng thời.
- **Đăng ký công khai lộ email:** như D-154 / D-162 — request chỉ xếp job, cùng câu trả lời.
- **Cấp phát chạy lại:** mỗi bước kiểm "đã làm chưa" bằng trạng thái thật (khách đã có? database đã
  migrate tới bản mới nhất?), không chỉ dựa vào cột `step`; tạo khách bằng ID sinh sẵn.
- **Trait gọi chéo module:** `content` gọi `dyn TextModel` của `ai` khi `ai` bị tắt cho khách → module
  cung cấp trả `module_not_enabled`, bên gọi phải xử lý êm (ẩn gợi ý), không trả 500.

## Căn cứ pháp lý

- Hoá đơn cho phí dịch vụ SaaS: NĐ 123/2020/NĐ-CP (sửa bởi NĐ 70/2025/NĐ-CP) — thời điểm lập hoá đơn
  cung cấp dịch vụ là lúc hoàn thành hay lúc thu tiền trước; việc lập do `invoices` (bước 36). Thuế
  GTGT của dịch vụ phần mềm / điện toán đám mây: có ý kiến thuộc đối tượng không chịu thuế (dịch vụ
  phần mềm), có ý kiến chịu thuế — **cần đối chiếu Luật Thuế GTGT 48/2024/QH15 và văn bản hướng dẫn**;
  vì vậy `plans_price.tax_category` là dữ liệu, không viết cứng.
- Khách không tự giữ tiền người khác: phí SaaS thu vào tài khoản nhà vận hành qua đối tác có giấy phép
  (D-187, NĐ 52/2024/NĐ-CP).
- Xoá dữ liệu sau khi huỷ: Luật BVDLCN 91/2025/QH15 (dữ liệu cá nhân không giữ quá mục đích) — nhưng
  chứng từ kế toán giữ ít nhất 10 năm (Luật Kế toán 88/2015 Điều 41, NĐ 174/2016) — phần kế toán do
  bước 36 giữ khi xoá khách. `retention_days` là dữ liệu của gói.
- Gia hạn tự động và điều kiện giao dịch chung: nếu khách thuê là cá nhân / hộ kinh doanh có thể là
  người tiêu dùng — Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15 và NĐ 55/2024/NĐ-CP (minh bạch
  điều kiện, báo trước khi gia hạn) — **cần đối chiếu**; mail nhắc trước hạn đáp ứng phần báo trước.
- KYC khi gói yêu cầu (vd khách là sàn): Luật TMĐT 122/2025/QH15 Điều 17 (từ 01/7/2026) — do bước 37.

## Agent làm bước này tự quyết

Ghi DECISIONS cho lựa chọn có ảnh hưởng lâu dài (skill `record-decision`).

1. **Vị trí đăng ký của `plans`**: bảng tên chung đặt `plans` ngay sau `address`; đặc tả này khuyến
   nghị đặt sau mọi module nền (sau `notifications`) để `require` được `dyn Invoices`, `dyn
   AdministrativeUnits`, `api::Invitations` và phần KYC; module đăng ký trước `plans` dùng
   `EntitlementsHandle` nên không cần `plans` đứng sớm. Không làm vậy thì phần tính tiền / cấp phát
   phải đi qua event.
2. Mã HTTP cho quyền dùng: khuyến nghị 403 cho `module_not_enabled` / `feature_not_enabled` /
   `limit_exceeded` / `tenant_read_only`, 429 cho `quota_exhausted` (402 chưa có nghĩa chuẩn).
3. Có cưỡng chế cứng `kernel.api_requests` hay chỉ ghi để tính tiền — khuyến nghị chỉ chặn khi gói đặt
   `max_value`; mất tối đa 60 giây số đếm khi tiến trình chết (ghi rõ).
4. Module nào `always_enabled`: khuyến nghị danh sách ở "Kiểu và trait"; `ai` và mọi module nghiệp vụ
   bị gói chi phối.
5. Proration bằng `i128` hay chờ `rust_decimal` của bước 36 — khuyến nghị `i128` (phép nhân rồi chia số
   nguyên, làm tròn xuống, có test biên).
6. Hạ gói: áp ngay hay cuối kỳ — khuyến nghị cuối kỳ (không hoàn tiền); nâng gói: ngay, kỳ tỉ lệ.
7. Database `dedicated` khi cấp phát: tạo mới (SQLite: file trong thư mục cấu hình; Postgres: `CREATE
   DATABASE` bằng URL quản trị) hay lấy từ danh sách dựng sẵn — theo cách bước 33 làm; Postgres cần quyền
   `CREATEDB`, ghi trong `docs/DEPLOY.md`.

## Bổ sung theo đối chiếu thiết kế 2 (D-208…D-218)

> Nguồn: `docs/prompts/ui-trace-roadmap.md`; mã `A-…` / `B-…` trỏ tới `docs/research/ui-trace/`.

- **Xuất toàn bộ dữ liệu một khách (A-b9).** `POST /api/v1/kernel/tenants/{id}/exports` chạy job gom
  bản xuất của mọi module (dùng cơ chế backup / `PersonalData` mở rộng theo khách) thành gói tải về qua link
  ký có hạn; nhà vận hành và quản trị của chính khách đều gọi được; tự đề nghị khi khách huỷ thuê bao, giữ
  gói trong `AUTOXIDE_PLANS_EXPORT_RETENTION_DAYS` (mặc định 30).
- **Tạm khoá khác chỉ đọc (A-3.11).** Thuê bao quá hạn → khách `read_only` (bước 33). Thêm `POST
  /api/v1/plans/tenants/{id}/subscription/suspend` cho nhân sự nền tảng khi cần khoá hẳn (đặt
  `kernel_tenants.status = suspended`), có lý do và audit.
- **Hạn mức nút (A-3.12).** Hạn mức `organizations.units` đếm **mọi nút trừ gốc**; hạn mức theo loại khai
  riêng (`organizations.units.<loại>`, vd `organizations.units.store`) khi gói cần giới hạn số cửa hàng.
- **Danh sách khách (A-c7).** Cung cấp `subscription_status`, `near_limit` cho bộ lọc của bước 33.
- **Thấp (A-b10, B-b14).** Thông báo của nền tảng tới quản trị mọi khách (qua `notifications`, loại
  `Security` / `Transactional`); liên hệ hỗ trợ của nền tảng hiện trong tài khoản khách.
