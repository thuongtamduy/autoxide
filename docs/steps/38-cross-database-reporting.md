# Bước 38 — Cây khách hàng xuyên database và báo cáo gộp

> Lộ trình: `docs/ROADMAP.md` bước 38 · Thiết kế: `docs/research/business-foundation.md` trụ 2 (đoạn "Cây
> khách hàng xuyên database"), mục 5 (mô hình 2 và 4) · Quyết định: D-185, D-190, D-054 (bên nhận event
> chịu nhận lại), D-184 (reference), D-070 (`autoxide copy`), D-183 (chuỗi băm audit), D-210 (tách / gộp qua
> API), D-232 (sandbox, nhóm nút)

## Mục tiêu

Xong bước này, mô hình 2 (một khách nhiều cửa hàng, mỗi cửa hàng database riêng) và mô hình 4 (chuỗi
cha-con nhiều cấp, cha và con chung hay khác database) chạy trọn:

1. **Báo cáo gộp**: số liệu của mọi khách con (kể cả ở database khác) đổ về một kho báo cáo của cây
   khách hàng trong vài giây, mang `tenant_id` và `org_id` nguồn; báo cáo gộp nhiều cửa hàng, nhiều tiền tệ;
   chỉ người có quyền ở khách cha (hay ở chính nhánh đó) mới xem.
2. **Cha thao tác trên con**: cùng một API, vào ngữ cảnh khách con (uỷ quyền của bước 34) chạy được khi con
   ở database khác, audit ghi ở cả hai phía; **phân phối dữ liệu cha → con** bằng job gọi service của con,
   chạy lại được, không tạo bản trùng.
3. **Tách / gộp**: tách một nhánh tổ chức thành khách riêng (database riêng) và gộp ngược lại, giữ nguyên ID
   và lịch sử.
4. **Khách thử nghiệm (sandbox) cách ly** (D-232): chủ khách tạo một bản thử mang **cấu hình** của khách thật
   (không mang dữ liệu cá nhân); trong đó mọi đối tác là bộ giả, mail và webhook không ra ngoài; thứ đã thử
   (trang, theme, mẫu mail, luật tự động hoá…) đẩy sang khách thật thành **bản nháp**.
5. **Nhóm nút ngoài cây cho báo cáo** (D-232): gom nút bất kỳ của khách thành nhóm tự do (vd "cửa hàng miền
   Bắc") để lọc / gom báo cáo gộp; nhóm **không cấp quyền**.

Mọi thứ chạy và có test trên cả SQLite (mỗi database một file) và Postgres (mỗi database một database).

## Bắt đầu khi

- Bước 34 đã merge: cây khách hàng trong mặt điều khiển (`kernel_tenants` có khách cha), uỷ quyền theo cây
  khách hàng, `Principal` mang tenant + `Scope`, cây tổ chức `organizations` (`dyn OrgDirectory`, bảng tổ
  tiên – con cháu, cài đặt kế thừa: múi giờ, tiền tệ), dữ liệu nghiệp vụ có `org_id`.
- Bước 33: `DatabaseRegistry` (mở database của khách bất kỳ, mở lười, có giới hạn), `kernel_placements`
  (`shared` / `dedicated`), worker và bộ phát event chạy trên từng database, `Delivered` mang tenant của
  event, lỗi `tenant_maintenance`, `autoxide copy` mở rộng để dời cả một khách.
- Bước 36 đã merge: `ledger`, `invoices` (fact tiền đầu tiên; tỉ giá qua `dyn Ledger::exchange_rate` — D-237).
- Mục sandbox (D-232) cần thêm bước 35 (hạn mức gói, quyền dùng) và bước 37 (`SmsSender`, `IdentityVerifier`):
  bộ giả của kernel thay đúng các trait đó.
- Làm song song được với 40, 41–45. Bước 39 làm **sau** bước này (rà index phủ cả bảng của 38 — D-237). Module 41–45 tự khai fact của mình theo cơ chế ở đây (bước 45 có mục
  "event cho kho báo cáo gộp").

## Phạm vi

**Kernel**:

| Chỗ | Nội dung |
|---|---|
| `src/report.rs` (mới) | kiểu `Fact`, `Measure`, `FactSource` — hợp đồng trung lập để module khai số liệu cho báo cáo |
| `src/module.rs` | `Module::facts()` và `Module::relocation()` kèm cài đặt mặc định (D-038); `StartContext::fact_sources()` |
| `src/tenancy/distribution.rs` (mới) | khung phân phối cha → con: `Distributor`, `DistributionApply`, `distribution_migration(module)` |
| `src/tenancy/relocate.rs` (mới) | tách / gộp khách: chính sách theo bảng, kế hoạch (dry-run), chép có lọc + đổi `tenant_id`, kiểm, chuyển, dọn; bảng `kernel_relocations` (mặt điều khiển) |
| `src/transfer.rs` | thêm chép **có lọc dòng và ghi đè cột** (dùng lại đọc theo snapshot, ghi theo kiểu cột của đích) |
| `src/tenancy/sandbox/` (mới) | sandbox (D-232): tạo / gia hạn / xoá, sao cấu hình qua `Distributor`, đẩy sang khách thật, `Partners` (sổ đối tác) và bộ giả của đối tác, hộp thư giả, lớp HTTP gắn header sandbox |
| `src/module.rs` (thêm) | `Module::sandbox_copy()`, `Module::promotable()` kèm cài đặt mặc định (D-038, D-232); `StartContext::partners()` |

**Module mới** `modules/reporting` · crate `autoxide-reporting` — đăng ký **cuối cùng** trong
`crates/app/src/modules.rs`. Sở hữu bảng `reporting_…`, permission `reporting.*`, lỗi `reporting.*`.

**Sửa module có sẵn**: `ledger`, `invoices` khai `facts()`; mọi module khai `relocation()` cho bảng cấp khách
không có `org_id` (mặc định suy từ phân loại bảng của bước 33–34, xem dưới); `audit` cài chính sách tách /
gộp riêng (nối vào chuỗi băm của database đích). CLI `crates/app/src/commands/tenant.rs`: `autoxide tenant
split|merge|relocation-status`.

Thêm cho sandbox và nhóm nút (D-232):

- `organizations`: bảng `organizations_group`, `organizations_group_member`, REST / tool MCP nhóm, trait
  `OrgGroups` trong `api`; khai `sandbox_copy` cho cây tổ chức và cài đặt nút.
- `auth`: nghe `kernel.sandbox_created` để tạo uỷ quyền vào sandbox; công bố `SandboxRecipients` (email đã xác
  minh của nhân viên vào được sandbox); khai `sandbox_copy` cho role (không sao người dùng, membership, API key).
- `content`: khai `sandbox_copy` và `promotable` cho trang, menu; `notifications` (bước 37): như vậy cho mẫu mail
  (`notifications_templates`). `kernel`: sao cài đặt khách **không bí mật**.
- `mail`, `webhooks`: tôn trọng sandbox (hộp thư giả, danh sách URL cho phép). `ledger`, `invoices` / `tax`,
  `compliance`, `notifications`: lấy đối tác qua `Partners`, không giữ `Arc` đối tác từ lúc khởi động.
- Module dựng sau tự khai ở bước của mình: `catalog` (41: cấu trúc danh mục, tối đa N sản phẩm, không tồn),
  `promotions` (43: chương trình, nhận ở trạng thái nháp), `automations` (58: luật, nhận ở trạng thái tắt),
  theme và khu dùng chung của `content` (67); `shipping` (44) cho `CarrierRegistry` trả hãng giả khi
  `is_sandbox`.

**Ngoài phạm vi**: dashboard / giao diện (luật 9); báo cáo theo nghiệp vụ của module chưa có (đơn hàng,
tồn kho — module đó tự khai fact khi dựng); kho dữ liệu cột (OLAP) hay database báo cáo ngoài SQLite /
Postgres; đồng bộ hai chiều con → cha của dữ liệu nghiệp vụ (chỉ số liệu báo cáo đi lên, dữ liệu đi xuống
bằng phân phối); sao người mua, đơn, sổ cái, thông báo sang sandbox; bộ nối đối tác thật trong sandbox (D-232).

## Thiết kế

### Kiểu và trait (phác thảo — tra API thật bằng skill `check-library-api`, `cargo check` xác nhận)

**Kernel `report`** — module khai "event nào sinh số liệu gì"; kernel không biết nghĩa nghiệp vụ:

```rust
/// Một số liệu báo cáo, rút từ một event. KHÔNG chứa dữ liệu cá nhân (tên, email, SĐT, địa chỉ, user_id).
pub struct Fact {
    pub kind: &'static str,                 // `<module>.<số liệu>`, vd "invoices.issued_total"
    pub org_id: Option<OrgId>,              // nút sở hữu ở khách nguồn
    pub occurred_at: DateTime<Utc>,
    pub amount: Option<Money>,              // số tiền (tiền gốc)
    pub quantity: Option<i64>,              // số đếm / số lượng nguyên
    pub dimensions: BTreeMap<&'static str, String>, // tập nhỏ cố định do FactSource khai, vd "kind" → "vat_invoice"
}
pub struct FactSource {
    pub event: &'static str,                // tên event của chính module, vd "invoices.document_issued"
    pub kinds: Vec<FactKind>,               // kind, mô tả (EN), các dimension được phép — cho `GET /reporting/kinds`
    pub extract: fn(&Delivered) -> Result<Vec<Fact>, Error>, // thuần, không I/O
}
// module.rs — API mới, mặc định rỗng:
fn facts(&self) -> Vec<FactSource> { Vec::new() }
// StartContext — API mới: mọi FactSource của mọi module đã đăng ký (reporting đăng ký cuối nên thấy đủ).
pub fn fact_sources(&self) -> Vec<(&'static str /* module */, FactSource)>;
```

`Module::facts` của module `x` chỉ được khai event `x.*` (kernel kiểm lúc khởi động, như `Outbox::record`).

**Kernel `tenancy::distribution`** — phân phối cha → con, module nào cũng dùng (bước 41 `catalog` dùng
đầu tiên):

```rust
/// Module cài để áp một bản ghi của khách cha vào khách con — gọi service của chính module, trong ngữ cảnh con.
#[async_trait] pub trait DistributionApply: Send + Sync {
    /// Phải idempotent: tạo bằng `reference = "dist:{parent_tenant}:{record_kind}:{record_id}"` (D-184), cập nhật
    /// chỉ khi `version` lớn hơn `source_version` đang lưu ở con (UPDATE … WHERE source_version < ?).
    async fn apply(&self, child: &TenantContext, actor: Actor, item: &DistributionItem) -> Result<Applied, Error>;
}
pub struct DistributionItem { pub parent: TenantId, pub record_kind: String, pub record_id: Uuid, pub version: i64,
                              pub payload: serde_json::Value }
pub enum Applied { Created(Uuid), Updated(Uuid), Unchanged(Uuid), Skipped { reason: String } }
pub struct Distributor { /* module, JobQueue, DatabaseRegistry */ }
impl Distributor {
    /// Trong transaction GHI của khách cha: ghi bản phân phối + một dòng đích cho mỗi khách con được chọn
    /// + enqueue job `<module>.distribute` (unique_key = "dist:{distribution_id}:{child}"). Trùng (record, version) → bản cũ.
    pub async fn publish<C: ConnectionTrait>(&self, conn: &C, parent: &TenantContext, actor: Actor,
        item: DistributionItem, targets: Targets /* AllChildren | Descendants | Only(Vec<TenantId>) */) -> Result<Uuid, Error>;
    pub async fn status(&self, parent: &TenantContext, distribution: Uuid) -> Result<DistributionStatus, Error>;
    pub async fn retry_failed(&self, parent: &TenantContext, distribution: Uuid) -> Result<u64, Error>;
    /// Bộ chạy job cho `Module::jobs` — module chỉ đưa vào `Arc<dyn DistributionApply>` của mình.
    pub fn job(&self, kind: &'static str, apply: Arc<dyn DistributionApply>) -> JobRegistration;
}
pub fn distribution_migration(module: &'static str) -> Box<dyn MigrationTrait>; // bảng `<module>_distributions`, `<module>_distribution_targets`
```

**Kernel `tenancy::relocate`** — chính sách theo bảng cho tách / gộp:

```rust
pub enum TablePolicy {
    ByOrg { column: &'static str },                       // dòng có org thuộc nhánh thì đi theo nhánh (mặc định khi bảng có `org_id`)
    FollowParent { parent_table: &'static str, column: &'static str }, // dòng con theo dòng cha (vd `ledger_lines.entry_id`)
    TenantWide { on_split: Split /* Stay | Copy */, on_merge: Merge /* Import | Skip */ },
    MustBeEmpty,                                          // outbox, jobs: phải rỗng phần của khách trước khi chép
    Custom(Arc<dyn RelocateTable>),                       // audit, kernel_sequences, organizations
}
pub struct Relocation { pub tables: Vec<(&'static str, TablePolicy)> }
// module.rs — API mới; mặc định suy từ phân loại bảng (bước 33/34): có `org_id` → ByOrg, outbox/jobs → MustBeEmpty,
// bảng cấp khách khác → TenantWide { Stay, Import } kèm CẢNH BÁO trong kế hoạch.
fn relocation(&self) -> Relocation { Relocation::inferred() }
#[async_trait] pub trait RelocateTable: Send + Sync {
    async fn plan(&self, ctx: &RelocationContext) -> Result<TablePlan, Error>;    // số dòng, xung đột unique, tham chiếu vắt qua ranh giới
    async fn copy(&self, ctx: &RelocationContext, target: &DatabaseTransaction) -> Result<u64, Error>;
    async fn clean(&self, ctx: &RelocationContext, source: &DatabaseTransaction) -> Result<u64, Error>;
}
```

**Kernel `tenancy::sandbox`** (D-232) — khách thử nghiệm và sổ đối tác:

```rust
pub enum TenantKind { Production, Sandbox }             // cột `kernel_tenants.kind`
impl TenantContext {
    pub fn is_sandbox(&self) -> bool;
    pub fn sandbox_of(&self) -> Option<TenantId>;       // khách gốc; None với khách thật
}
/// Sổ đối tác. Module hỏi theo khách của TỪNG lần gọi, không giữ `Arc` đối tác lấy lúc `start`.
pub struct Partners { /* contributions của mọi module + bộ giả của kernel */ } // StartContext::partners()
impl Partners {
    /// Khách thật: đối tác đã `contribute` / `provide`. Sandbox: LUÔN bộ giả của kernel, kể cả khi có bộ nối thật.
    pub fn get<T: ?Sized + Partner>(&self, tenant: &TenantContext) -> Vec<Arc<T>>;
}
/// Cài cho `dyn PaymentGateway`, `dyn PayoutGateway`, `dyn BankFeed`, `dyn EInvoiceProvider` (bước 36),
/// `dyn SmsSender`, `dyn IdentityVerifier`, `dyn ZnsSender`, `dyn MobilePushSender` (bước 37): bộ giả không gọi
/// mạng, ghi hộp thư giả.
pub trait Partner: Send + Sync + 'static { fn sandbox_double(inbox: SandboxInbox) -> Arc<Self>; }
/// Hộp thư giả (bảng `kernel_sandbox_messages` của kernel): `mail`, `webhooks` và bộ giả ghi qua đây, không
/// ghi thẳng bảng. Gọi với khách thật → lỗi nội bộ. StartContext::sandbox_inbox().
#[derive(Clone)] pub struct SandboxInbox { /* … */ }
impl SandboxInbox {
    pub async fn record(&self, sandbox: &TenantContext, message: SandboxMessage) -> Result<(), Error>;
}
/// Late-bound như `EntitlementsHandle`: `auth` cài; chưa ai cài thì mọi mail của sandbox vào hộp thư giả.
#[async_trait] pub trait SandboxRecipients: Send + Sync {
    async fn allowed(&self, sandbox: &TenantContext, emails: &[String]) -> Result<Vec<bool>, Error>;
}

// module.rs — API mới, mặc định: không sao gì, không đẩy được gì (D-038).
fn sandbox_copy(&self) -> Option<Arc<dyn SandboxCopy>> { None }
fn promotable(&self) -> Vec<Promotable> { Vec::new() }

#[async_trait] pub trait SandboxCopy: Send + Sync {
    /// Bản ghi CẤU HÌNH để sao, theo trang (con trỏ `after`); áp ở sandbox bằng `DistributionApply` của chính
    /// module (job `<module>.distribute`). Không bao giờ trả dữ liệu cá nhân hay giao dịch.
    async fn items(&self, source: &TenantContext, options: &SandboxOptions, after: Option<Uuid>)
        -> Result<Vec<DistributionItem>, Error>;
}
pub struct SandboxOptions { pub products: u32 /* 0..=AUTOXIDE_TENANCY_SANDBOX_MAX_PRODUCTS */ }
pub struct Promotable { pub kind: &'static str /* `<module>.<loại>`, vd "content.page" */,
                        pub description: &'static str, pub apply: Arc<dyn PromoteApply> }
#[async_trait] pub trait PromoteApply: Send + Sync {
    /// Ghi bản ghi `id` của sandbox sang khách thật qua service của module, LUÔN ở dạng nháp / tắt.
    /// Idempotent theo `reference = "promote:{sandbox}:{kind}:{id}"` (D-184).
    async fn promote(&self, sandbox: &TenantContext, target: &TenantContext, actor: Actor, id: Uuid)
        -> Result<Promoted, Error>;
}
pub enum Promoted { CreatedDraft(Uuid), UpdatedDraft(Uuid), Unchanged(Uuid),
                    Conflict { field: String, message: String } }
```

`Distributor::publish` nhận thêm đích `Targets::Sandbox(TenantId)`: hợp lệ khi `sandbox_of` của đích là khách
gửi (D-232); kiểm "còn là con cháu" của job phân phối chấp nhận đích này.

**`organizations::api::OrgGroups`** (D-232) — `organizations` công bố, `reporting` `require` (đăng ký sau):

```rust
#[async_trait] pub trait OrgGroups: Send + Sync {
    /// Nút thành viên của nhóm; `None` khi khách không có nhóm này. KHÔNG lọc theo quyền — bên gọi giao với
    /// phạm vi của người xem.
    async fn members(&self, tenant: &TenantContext, group: Uuid) -> Result<Option<Vec<OrgId>>, Error>;
}
```

**API mới cần thêm từ bước 33/34** (nếu bước đó chưa có đúng tên này, dùng tên của nó và sửa đặc tả):
`TenantDirectory::root_of(tenant)`, `ancestors(tenant)`, `descendants(tenant)` (đọc `kernel_tenants`);
`DatabaseRegistry::context_for(tenant) -> TenantContext` cho code hệ thống (job, event handler) cần chạy
service của một khách khác; testkit `TestDatabase::another()` — một database trống thứ hai cùng loại để test
khách `dedicated`.

### Bảng dữ liệu

Cột như bước 36: UUIDv7, UTC, tiền = `<tên>_minor` + `<tên>_currency`, JSON là `TEXT`; mọi unique / index bắt
đầu bằng `tenant_id`; migration có `down`.

**`reporting`** — cấp khách, có ở mọi database dữ liệu:

| Bảng | Cột | Khoá / index |
|---|---|---|
| `reporting_local_facts` | `id`, `tenant_id` (khách **nguồn**), `source_event_id`, `fact_index` (thứ tự trong event), `kind`, `org_id`, `occurred_at`, `local_day` (ngày theo múi giờ của nút nguồn), `amount_minor`, `amount_currency`, `quantity`, `dimensions` (JSON), `created_at`, `forwarded_at` (null = chưa đẩy) | unique (`tenant_id`, `source_event_id`, `fact_index`); index (`tenant_id`, `forwarded_at`, `id`) |
| `reporting_facts` | kho của cây — chỉ ở database của **khách gốc** của cây: `id` (= id fact cục bộ), `tenant_id` (khách gốc), `source_tenant_id`, `kind`, `org_id`, `occurred_at`, `local_day`, `amount_*`, `quantity`, `dimensions`, `received_at` | unique (`tenant_id`, `source_tenant_id`, `id`); index (`tenant_id`, `kind`, `local_day`); index (`tenant_id`, `source_tenant_id`, `kind`, `local_day`) |
| `reporting_daily` | cộng dồn theo ngày: `tenant_id` (gốc), `source_tenant_id`, `org_id` (nút gốc = ID khách khi số liệu không nêu nút — D-20261118-step38-report-facts), `kind`, `local_day`, `currency` (chuỗi rỗng khi không có tiền), `amount_minor`, `quantity`, `fact_count` | PK (`tenant_id`, `source_tenant_id`, `org_id`, `kind`, `local_day`, `currency`) |
| `reporting_fact_dimensions` | chiều của số liệu trong kho (D-20261118-step38-report-facts): `tenant_id` (gốc), `fact_id`, `name`, `value` | PK (`tenant_id`, `fact_id`, `name`); index (`tenant_id`, `name`, `value`, `fact_id`) |
| `reporting_rehomes` | nhật ký dời kho khi cây đổi (tách, gộp, đổi khách cha): `id`, `tenant_id` (khách bị dời), `from_root`, `to_root`, `status`, `moved_facts`, `started_at`, `finished_at` | index (`tenant_id`, `started_at`) |

Không có `user_id` hay dữ liệu cá nhân trong fact → kho gộp không phải xoá gì khi có `USER_DELETED`, và
được nằm ở database của khách gốc dù khách con chọn vị trí khác (vẫn ghi rõ trong tài liệu: số liệu tổng hợp
của con nằm ở database của gốc).

**Kernel `tenancy::distribution`** — bảng của module dùng nó (như `outbox_migration`), ở database của khách cha:

| Bảng | Cột | Khoá |
|---|---|---|
| `<module>_distributions` | `id`, `tenant_id` (cha), `record_kind`, `record_id`, `version`, `payload` (JSON), `created_by`, `created_at` | unique (`tenant_id`, `record_kind`, `record_id`, `version`) |
| `<module>_distribution_targets` | `id`, `tenant_id` (cha), `distribution_id`, `target_tenant_id`, `status` (`pending`/`applied`/`skipped`/`failed`), `applied_record_id`, `outcome`, `attempts`, `last_error`, `applied_at` | unique (`tenant_id`, `distribution_id`, `target_tenant_id`) |

Ở khách con, bảng của module lưu `source_tenant_id`, `source_record_id`, `source_version` trên bản ghi nhận
được (cột của module, không phải bảng chung) cùng `reference` theo D-184.

**Kernel `tenancy::relocate`** — mặt điều khiển: `kernel_relocations` (`id`, `kind` `split`/`merge`,
`source_tenant_id`, `target_tenant_id`, `org_id` (gốc nhánh tách / nút nhận khi gộp), `placement`
(`shared`/`dedicated` + database), `state` (`planned` → `frozen` → `copied` → `verified` → `switched` →
`cleaned` → `done`; hoặc `failed`; `requested` cho yêu cầu của khách), `plan` (JSON: bảng, số dòng, cảnh báo, xung
đột), `options` (slug, tên, lý do, khách gốc trước / sau), `error`, `runner`, `created_by`, `created_at`, `updated_at`)
— migration kernel `m0035_create_relocations`. Khách hàng có thêm trạng thái `merged` (không thêm cột `merged_into`:
khách nhận đọc từ lượt gộp — D-20261118-step38-relocation).

**Kernel `tenancy::sandbox`** (D-232):

- `kernel_tenants` (mặt điều khiển) thêm cột bằng migration mới: `kind` (`production` / `sandbox`, NOT NULL,
  dòng cũ nhận `production`), `sandbox_of` (uuid NULL, khách gốc; không FK), `sandbox_expires_at` (NULL với khách
  thật), `sandbox_options` (JSON `SandboxOptions`, NULL). Index `(sandbox_of)`; `(kind, sandbox_expires_at)` cho
  lịch hết hạn. Sandbox có `parent_id` NULL: không phải khách con, không nằm trong `descendants` của ai.
- Cấp khách, có ở mọi database dữ liệu:

| Bảng | Cột | Khoá / index |
|---|---|---|
| `kernel_sandbox_messages` | hộp thư giả: `tenant_id` (sandbox), `id`, `channel` (`mail` / `sms` / `zns` / `webhook` / `partner`), `recipient` (email, số, URL hay tên đối tác), `subject` (NULL), `body` (`text`, cắt 64 KiB), `reason` (`recipient_not_allowed` / `url_not_allowed` / `partner_double`), `created_at` | PK (`tenant_id`, `id`); index (`tenant_id`, `created_at`, `id`); index (`tenant_id`, `channel`, `created_at`) |

**`organizations`** (D-232) — cấp khách:

| Bảng | Cột | Khoá / index |
|---|---|---|
| `organizations_group` | `tenant_id`, `id`, `name`, `name_folded`, `description` (NULL), `created_by`, `created_at`, `updated_at` | PK (`tenant_id`, `id`); unique (`tenant_id`, `name_folded`) |
| `organizations_group_member` | `tenant_id`, `group_id`, `org_id` (nút bất kỳ của khách, không theo cây; không FK sang `kernel_org_units`), `added_by`, `added_at` | PK (`tenant_id`, `group_id`, `org_id`); index (`tenant_id`, `org_id`) |

### Luồng 1 — báo cáo gộp

1. **Rút fact (ở database nguồn)**: `reporting` đăng ký `Subscription::new(source.event, handler)` cho mỗi
   `FactSource`. Bộ phát event của database nguồn giao event → handler gọi `extract` → tính `local_day` theo
   múi giờ của nút (cài đặt kế thừa của bước 34; không có nút thì múi giờ khách) → INSERT
   `reporting_local_facts` ON CONFLICT DO NOTHING (nhận lại event không tạo bản trùng — D-054). Không đụng
   database khác ở bước này, nên database của gốc đang bảo trì không làm hỏng event.
2. **Đẩy lên kho của cây**: lịch `reporting.forward` (mỗi `AUTOXIDE_REPORTING_FORWARD_SECONDS`, mặc định 5,
   chạy trên từng database): với mỗi khách có fact `forwarded_at IS NULL`, lấy tối đa 500 dòng theo `id`, xác
   định gốc (`root_of`), mở database của gốc qua `DatabaseRegistry`, trong **một transaction ở database gốc**:
   INSERT `reporting_facts` ON CONFLICT DO NOTHING từng dòng; dòng nào thật sự được chèn
   (`rows_affected == 1`) thì cộng vào `reporting_daily` bằng `INSERT … ON CONFLICT (khoá) DO UPDATE SET
   amount_minor = amount_minor + excluded.amount_minor, …` (chạy trên SQLite ≥ 3.24 và Postgres) → commit; rồi
   ở database nguồn đặt `forwarded_at`. Chết giữa hai bước → lần sau đẩy lại, unique chặn đếm hai lần. Khách
   gốc chính là khách nguồn (khách đứng một mình, hay cây chung database) thì vẫn đi đúng đường này.
3. **Dọn**: lịch `reporting.prune_local` (24 giờ) xoá fact cục bộ đã đẩy quá `AUTOXIDE_REPORTING_LOCAL_RETENTION_DAYS`
   (mặc định 35).
4. **Xem**: service đọc ở database của gốc. Người gọi ở khách X cần `reporting.read`:
   - phạm vi `tenant` ở X → mọi khách trong `descendants(X) ∪ {X}`; đi kèm uỷ quyền theo cây của bước 34
     (cha thấy mọi thứ của con — mô hình 4);
   - phạm vi `subtree` / `unit` ở nút N của X → chỉ fact của X có `org_id` trong nhánh / nút đó;
   - lọc `tenant_ids` / `org_ids` của client là **giao** với tập được phép, không bao giờ mở rộng; khách ngoài
     cây hay ngoài phạm vi → `reporting.tenant_not_found` (404, không lộ khách có tồn tại hay không).
5. **Nhiều tiền tệ**: tổng luôn trả theo từng tiền gốc; nếu client xin `currency=VND` thì quy đổi **từng nhóm
   (tiền, ngày)** bằng tỉ giá của **khách người xem** ngày đó (`dyn Ledger::exchange_rate`, gọi trước, ngoài
   transaction), làm tròn `HalfUp` theo nhóm rồi cộng; thiếu tỉ giá thì liệt kê trong `missing_rates` và
   **không** cộng nhóm đó vào tổng quy đổi (không đoán, không bỏ im lặng).
6. **Fact đầu tiên** (bước này): `invoices.document_issued` → `invoices.issued_total` (tiền = tổng chứng từ,
   dimension `kind`), `invoices.issued_count`; `ledger.entry_posted` → `ledger.revenue` (tổng dòng có của tài
   khoản `revenue.*`, dimension `purpose`) và `ledger.tax_output` (`tax.vat_output`). Bước 45 thêm
   `orders.*` theo cùng cách.

### Luồng 2 — cha thao tác trên con

- **Vào ngữ cảnh con** dùng đúng cơ chế bước 34 (cùng endpoint, chọn khách con). Việc của bước này: chạy được
  khi con ở database khác — mọi handler / tool MCP lấy database qua `TenantContext` (không qua
  `StartContext::database()` giữ từ lúc khởi động); `Actor` của thay đổi mang khách nhà của người làm; event
  ghi ở database của con (audit của con thấy "người của khách cha X"), và một event
  `tenancy.delegated_access` ghi ở database của cha (audit của cha thấy ai đã vào con nào) — nếu bước 34 đã
  ghi hai phía thì chỉ thêm test xuyên database.
- **Phân phối** (`Distributor`): module của cha gọi `publish` trong transaction ghi bản ghi của cha → job
  `<module>.distribute` (ở database cha) chạy cho từng đích: `context_for(child)` → kiểm khách con còn là con
  cháu và đã bật module (bước 35; chưa bật → `Skipped`) → `DistributionApply::apply` (ghi database của con qua
  service của con, có event của con) → cập nhật dòng đích ở database cha. Hai database, hai transaction: nếu
  chết sau khi con đã ghi mà trước khi cha cập nhật, lần chạy lại gặp `reference` cũ ở con → `Unchanged` →
  đích thành `applied`. Thứ tự đến lộn (v3 tới trước v2) → v2 bị `source_version` chặn → `Unchanged`. Ghi đè
  cục bộ ở con (vd cửa hàng tự đổi giá) do module quyết trong `apply` (khuôn kế thừa của bước 34: trường con đã
  tự đặt thì không ghi đè).
- Ở bước này chưa có module nghiệp vụ dùng phân phối: test dùng một module mẫu khai trong test (đăng ký qua
  `TestApp::builder(…).module(…)`); `catalog` (41) dùng thật.

### Luồng 3 — tách và gộp

Nguồn và đích luôn là **hai database khác nhau** (`tenancy.relocation_same_database`): muốn tách vào database chung
hay gộp hai khách cùng database thì dời một khách trước bằng `autoxide tenant move` (D-20261118-step38-relocation). Khách
mới có ID = ID nút gốc N.

**Tách** nhánh gốc N của khách T thành khách T' (`autoxide tenant split --tenant T --org N --name … --placement
dedicated|shared [--database-url …] [--dry-run]`):

1. `planned`: thu `relocation()` của mọi module; với mỗi bảng cấp khách: số dòng sẽ đi, dòng `TenantWide`
   (giữ / chép), **xung đột** — tham chiếu từ dòng ở lại tới dòng đi và ngược lại (module báo qua `plan`), sổ
   cái có tài khoản đối tượng là nút trong nhánh mà sổ ở ngoài nhánh (không chặn: sổ của cha giữ nghĩa vụ với
   khách mới), bản ghi phân phối. `--dry-run` dừng ở đây và in kế hoạch. Có xung đột chặn → `failed` với báo cáo.
2. `frozen`: đặt T vào bảo trì (lỗi `tenant_maintenance` 503 cho riêng T — bước 33), chờ outbox và job của T
   trên database nguồn rỗng (bảng `MustBeEmpty`; quá `AUTOXIDE_RELOCATION_DRAIN_SECONDS` thì `failed`, mở lại T).
3. `copied`: tạo T' (`kernel_tenants`, khách cha = T theo tuỳ chọn `--keep-parent`, mặc định có),
   `kernel_placements`, migrate database đích; chép trong **một transaction ở đích** (đọc nguồn bằng
   `begin_snapshot`): mọi dòng theo chính sách, **giữ nguyên khoá chính**, đổi `tenant_id` T → T'; nút N thành
   gốc của cây tổ chức T' (`organizations` cài `Custom`: tính lại bảng tổ tiên – con cháu); `kernel_sequences`
   có scope chứa nút trong nhánh đi theo (`Custom`); `audit` (`Custom`): ghi lại các dòng audit về nhánh vào
   chuỗi băm của database đích bằng đúng đường của recorder (giữ `id` event, `occurred_at`; chuỗi mới, nguồn
   giữ nguyên dòng cũ nên chuỗi nguồn không gãy).
4. `verified`: so số dòng và băm danh sách khoá chính từng bảng nguồn (phần đi) với đích; sổ cái: bảng cân đối
   số phát sinh từng sổ đã chuyển bằng nhau, chuỗi niêm phong `ledger-v1` kiểm được ở đích (không băm `tenant_id`
   — bước 36).
5. `switched`: một transaction ở mặt điều khiển — tên miền gắn với nút trong nhánh, API key gắn nút trong nhánh,
   thành viên của các nút đó chuyển sang T'; T' hoạt động; T ra khỏi bảo trì.
6. `cleaned`: xoá phần đã đi khỏi database nguồn (theo thứ tự ngược với thứ tự tạo bảng); `reporting` dời kho
   (bản ghi `reporting_rehomes`: chép fact của T' từ kho gốc cũ sang kho gốc mới nếu khác, xoá ở kho cũ) → `done`.

Mỗi trạng thái **chạy lại được** (`autoxide tenant split --resume <relocation_id>`): chép là một transaction ở
đích (hỏng → đích trống, chép lại); chuyển là một transaction; dọn chỉ xoá dòng đã có ở đích.

**Gộp** khách T' vào khách T dưới nút M (`autoxide tenant merge --tenant T' --into T --under-org M`): cùng các
trạng thái theo chiều ngược: đổi `tenant_id` T' → T, gốc của T' thành con của M; dòng `TenantWide` theo
`on_merge`; **khoá chính trùng** (dòng audit đã chép khi tách trước đó) thì bỏ qua vì là cùng một lịch sử;
**unique nghiệp vụ trùng** (vd hai slug giống nhau) → `failed` trong kế hoạch, liệt kê từng dòng (module có thể
cài `Custom` để đổi tên kèm báo cáo); T' thành `merged`, `merged_into = T`, tên miền / API key / thành viên
chuyển sang T, database `dedicated` của T' được giữ tới khi người vận hành xoá tay.

Chạy được với mọi cặp vị trí: SQLite → SQLite, Postgres → Postgres, chung ↔ riêng; chép kiểu cột như
`transfer.rs` nên cặp SQLite ↔ Postgres cũng chạy (gate chỉ chạy một loại mỗi lượt — test cặp chéo khi có cả
hai URL, như test của `autoxide copy`).

### Luồng 4 — khách thử nghiệm (sandbox) cách ly (D-232)

**Tạo.** Chủ khách (`kernel.sandboxes.manage` ở khách thật) gọi `POST /api/v1/kernel/tenants/{id}/sandboxes`
`{name, products?, role_id?}`:

1. Kiểm `{id}` là khách hiện tại của người gọi (hay nhân sự nền tảng), là khách thật (sandbox của sandbox →
   `tenancy.sandbox_nested`), và giữ chỗ hạn mức gói `kernel.sandboxes` (`Limit::count`, gói không đặt thì 1 — kể cả khách không có thuê bao: ngoại lệ có chủ đích của
   "không thuê bao = không giới hạn" ở bước 35, vì mỗi sandbox tốn tài nguyên — D-232;
   vượt → `limit_exceeded` của bước 35) với `reference = "kernel.sandbox:<id>"`.
2. Ghi `kernel_tenants` `Provisioning`, `kind = sandbox`, `sandbox_of = {id}`, `parent_id` NULL,
   `sandbox_expires_at = now + AUTOXIDE_TENANCY_SANDBOX_DAYS`, **cùng placement với khách gốc**: khách gốc ở
   database chung thì sandbox ở database chung, khách gốc có database riêng thì sandbox nằm trong database riêng
   đó — dữ liệu không rời vị trí khách đã chọn (bước 33, 37), không cần URL database mới. Cách ly bằng `tenant_id`.
3. `seed_tenant` của mọi module như khách mới (bước 33), rồi job `kernel.sandbox_copy`: duyệt module theo thứ
   tự đăng ký, gọi `SandboxCopy::items` của từng module, `Distributor::publish(…, Targets::Sandbox(id))` — dùng
   lại hạ tầng phân phối cha → con, nên chạy lại không tạo bản trùng. Trả `Created` (201 + `Location`) với
   `status = provisioning` và id job; tiến độ qua job API chung (D-213). Xong thì `Active`, event
   `kernel.sandbox_created`; `auth` nghe event, tạo `auth_delegation` từ khách gốc sang sandbox với `role_id`
   (mặc định role `admin` của khách gốc), hết hạn cùng sandbox — nhân viên vào sandbox bằng đường uỷ quyền của
   bước 34 (header `Autoxide-Tenant`), không có tài khoản riêng.

**Sao gì.** Chỉ **cấu hình** (Luật 91/2025: tối thiểu hoá dữ liệu cá nhân): cài đặt khách không bí mật, cây tổ
chức và cài đặt nút, role, theme, trang, menu, cấu trúc danh mục và tuỳ chọn tối đa `products` sản phẩm (không
có tồn thật), chương trình khuyến mại ở trạng thái **nháp**, luật tự động hoá ở trạng thái **tắt**, mẫu mail.
**Không sao**: người dùng, người mua, đơn, sổ cái, chứng từ, thông báo, file riêng tư, secret / thông tin kết nối
đối tác, tên miền, API key. Bản ghi ở sandbox có ID mới (UUIDv7) và giữ `source_record_id` (cột của module,
như phân phối); tham chiếu giữa bản ghi đổi theo `source_record_id` — module sao theo thứ tự đăng ký nên bên
được tham chiếu luôn có trước.

**Trong sandbox** (kernel, `mail` và `webhooks` kiểm `TenantContext::is_sandbox`; module nghiệp vụ không tự rẽ
nhánh):

- **Đối tác buộc dùng bộ giả**: thanh toán, chi trả, đọc biến động số dư, SMS, ZNS, đẩy thông báo di động, hãng
  vận chuyển, hoá đơn điện tử, eKYC. `Partners::get` trả bộ giả của kernel cho mọi sandbox, bất kể khách đã cấu
  hình gì; bộ giả trả kết quả xác định (thanh toán thành công, hoá đơn có số giả mang tiền tố `SANDBOX-`, eKYC duyệt), ghi vào hộp
  thư giả kênh `partner` / `sms` / `zns`, không gọi mạng. Kết quả của bộ giả không bao giờ là căn cứ pháp lý ở
  khách thật (không đẩy sang được).
- **Mail**: chỉ gửi tới email đã xác minh của nhân viên vào được sandbox (`SandboxRecipients` của `auth`); người
  nhận khác vào hộp thư giả kênh `mail` (`recipient_not_allowed`), không gửi. Một mail nhiều người nhận thì tách.
- **Webhook**: chỉ gửi tới URL có tiền tố nằm trong cài đặt khách `kernel.sandbox.webhook_urls` của sandbox (mặc
  định rỗng); URL khác vào hộp thư giả kênh `webhook` (`url_not_allowed`), lần giao ghi `sandboxed`.
- **Website công khai**: chỉ ở tên miền phụ `<slug>.<AUTOXIDE_TENANCY_SANDBOX_DOMAIN>` (trống thì chỉ vào bằng
  header); sandbox không thêm được tên miền riêng (`tenancy.sandbox_forbidden`). Mọi response mang
  `X-Robots-Tag: noindex, nofollow`; `robots.txt` / sitemap của `content` trả "chặn tất cả".
- **Mọi response** (REST, MCP, công khai) của sandbox có header `Autoxide-Sandbox: true` (lớp HTTP của kernel);
  `GET /api/v1/kernel/tenant` trả thêm `kind`, `sandbox_of`, `sandbox_expires_at`.
- **Pháp lý hiển thị**: `GET /public/badge` của `compliance` (bước 37) trả `kind = "sandbox"`, không có link /
  ảnh xác nhận của Bộ Công Thương, nhãn "bản thử"; `GET /public/owner` thêm `sandbox: true` để chân trang ghi
  "bản thử".
- **Quyền dùng**: sandbox đọc quyền dùng (module, feature, hạn mức) của khách gốc nhưng đếm riêng, không có thuê
  bao, không tính tiền (`plans` trả `snapshot` của khách gốc cho sandbox). Không có khách con, không làm khách
  cha của phân phối, không tách / gộp, không có sandbox con.
- **Báo cáo**: fact của sandbox chỉ đổ về kho của chính nó (`root_of(sandbox) = sandbox`), không bao giờ vào kho
  của cây khách gốc.

**Hết hạn và xoá.** Lịch `kernel.expire_sandboxes` (mỗi giờ, database điều khiển): sandbox quá
`sandbox_expires_at` → `Suspended` ngay, rồi xoá theo luồng xoá khách (bước 33, 35); trả chỗ hạn mức
`kernel.sandboxes`. Gia hạn: `POST /api/v1/kernel/sandboxes/{id}/extend` đặt lại `now + SANDBOX_DAYS`. Khách gốc
bị tạm khoá → sandbox của nó cũng bị chặn; khách gốc bị xoá → sandbox bị xoá trước.

**Đẩy sang khách thật.** `POST /api/v1/kernel/sandboxes/{id}/promote {items: [{kind, id}]}` (gọi ở khách gốc,
`kernel.sandboxes.manage`, `Idempotency-Key`, tối đa 100 mục):

- Chỉ loại module khai trong `Module::promotable` (trang, theme, mẫu mail, luật tự động hoá, chương trình khuyến
  mại, khu dùng chung); loại khác → `tenancy.promote_kind_unknown`. Danh sách: `GET /api/v1/kernel/sandboxes/promotable`.
- Mỗi mục: `PromoteApply::promote` trong ngữ cảnh khách thật, transaction riêng, qua service của module (có event,
  audit của khách thật). Bên khách thật nhận **bản nháp**: không tự đăng, không tự bật. Bản ghi sandbox sao từ
  khách thật (có `source_record_id`) thành bản nháp / bản sửa đổi chờ đăng của chính bản ghi gốc; bản ghi tạo
  mới trong sandbox thành bản ghi mới ở khách thật. Bản đang đăng / đang bật không bao giờ bị ghi đè.
- Trùng tên / đường dẫn (slug) / mã ở khách thật → mục đó `conflict` kèm trường và câu báo, mục khác vẫn chạy.
- Chạy lại: `reference` (D-184) tìm thấy bản đã đẩy → `UpdatedDraft` nếu nội dung đổi và bản đó còn là nháp,
  `Unchanged` nếu không đổi — không tạo trùng.
- Trả `Data<PromoteResult>`: mỗi mục `{kind, id, outcome, target_id, conflict}`; event `kernel.sandbox_promoted`
  (số mục theo kết quả) ghi ở khách thật.

### Luồng 5 — nhóm nút ngoài cây cho báo cáo (D-232)

- Nhóm là tập nút tự do trong **một** khách (vd "cửa hàng miền Bắc", "cửa hàng nhượng quyền"), không theo cây,
  một nút ở nhiều nhóm được. Quản lý ở `organizations` (`organizations.groups.manage`, chỉ cấp ở độ sâu `tenant`
  vì nhóm vắt qua nhiều nhánh); tối đa 1.000 nút mỗi nhóm.
- **Nhóm không cấp quyền.** `reporting` nhận `group_ids` ở `/summary`, `/facts` và `group_by=group`: lấy thành
  viên qua `OrgGroups::members` (không đọc bảng của `organizations`), **giao** với phạm vi `reporting.read` của
  người gọi (cùng luật lọc ở Luồng 1 bước 4); nút ngoài phạm vi bị bỏ và response ghi `excluded_org_count` (tổng
  và theo từng nhóm khi `group_by=group`). Nhóm chỉ lọc fact của chính khách sở hữu nhóm (`source_tenant_id` =
  khách người gọi), không lọc khách con.
- `group_by=group` cần `group_ids` (thiếu → 422, D-20261118-step38-groups): một fact thuộc nút ở hai nhóm được tính ở cả hai nhóm — tổng các nhóm có thể lớn hơn tổng
  chung (ghi trong mô tả OpenAPI).
- Xoá nút / lưu trữ nút không xoá thành viên (báo cáo lịch sử vẫn gom được); xoá nhóm không xoá fact.

### API REST

**`/api/v1/reporting`** (`TenantContext` + `Principal`, envelope chuẩn):

| Method + path | Permission | Tóm tắt |
|---|---|---|
| `GET /kinds` | `reporting.read` | các loại fact đã khai (kind, mô tả, đơn vị: tiền / số, dimension được phép) |
| `GET /summary?kind=&from=&to=&group_by=day\|week\|month\|tenant\|org\|group&tenant_ids=&org_ids=&group_ids=&currency=` | `reporting.read` | tổng theo nhóm: `by_currency[]` (`amount`, `quantity`, `count`), `converted` (khi có `currency`), `missing_rates[]`, `excluded_org_count` (khi có `group_ids` / `group_by=group`, D-232); đọc `reporting_daily`; khoảng tối đa 366 ngày khi nhóm theo ngày (`reporting.range_too_large`) |
| `GET /facts?kind=&from=&to=&tenant_ids=&org_ids=&group_ids=&dimensions=<tên>:<giá trị>,…` | `reporting.read` | `Page` fact thô để xem chi tiết (`fetch_page`, `limit`/`cursor`/`order`) |
| `GET /lag` | `reporting.manage` | với từng khách trong cây: số fact chưa đẩy, fact cũ nhất chưa đẩy, lần đẩy cuối |
| `POST /rebuild` | `reporting.manage` | đặt lại `forwarded_at` của fact cục bộ còn giữ cho một khách (đẩy lại; unique chặn trùng); trả số dòng |

Không có trang công khai.

Phân phối không có route chung ở kernel: module dùng nó tự mở `POST /api/v1/<module>/…/distribute` và
`GET /api/v1/<module>/distributions/{id}` (gọi `Distributor::publish` / `status`), permission của module.

Tách / gộp: CLI (`autoxide tenant split|merge|relocation-status`) là cách chính cho người vận hành. Mặt điều
khiển thêm `POST /api/v1/kernel/relocations` (`dry_run` mặc định `true`) và `GET /api/v1/kernel/relocations/{id}`
— chỉ nhân sự nền tảng (`kernel.tenants.relocate` + tài khoản khách nền tảng, D-210, D-20261118-step38-relocation),
chạy bằng job `kernel.relocate`. Khách không tự tách / gộp: khách chỉ gửi yêu cầu
(`POST /api/v1/kernel/tenant/relocation-requests`, `GET …/{id}`, permission `kernel.relocations.request`), yêu cầu vào
hộp việc của nhân sự nền tảng (nguồn việc `kernel.relocation_requests`), nhân sự chạy nó bằng `request_id`.

**`/api/v1/kernel`** — sandbox (D-232):

| Method + path | Permission | Tóm tắt |
|---|---|---|
| `GET /tenants/{id}/sandboxes` | `kernel.sandboxes.read` | `Page` sandbox của khách `{id}` (khách hiện tại, hay nhân sự nền tảng): id, slug, tên, trạng thái, hạn |
| `POST /tenants/{id}/sandboxes` `{name, products?, role_id?}` | `kernel.sandboxes.manage` | tạo sandbox (Luồng 4) → `Created` + id job sao cấu hình; `Idempotency-Key`; hạn mức `kernel.sandboxes` |
| `POST /sandboxes/{id}/extend` | `kernel.sandboxes.manage` | gia hạn thêm `AUTOXIDE_TENANCY_SANDBOX_DAYS` tính từ bây giờ |
| `DELETE /sandboxes/{id}` | `kernel.sandboxes.manage` | xoá ngay theo luồng xoá khách; trả chỗ hạn mức |
| `GET /sandboxes/promotable` | `kernel.sandboxes.read` | các loại đẩy được: `kind`, module, mô tả |
| `POST /sandboxes/{id}/promote` `{items: [{kind, id}]}` | `kernel.sandboxes.manage` | đẩy sang khách thật thành nháp; `Idempotency-Key`; kết quả từng mục |
| `GET /sandbox/messages?channel=` | `kernel.sandboxes.read` | gọi **trong** sandbox: `Page` hộp thư giả (`fetch_page`) |

Sandbox `{id}` không phải sandbox của khách người gọi → `tenancy.sandbox_not_found` (404, không lộ tồn tại).

**`/api/v1/organizations`** — nhóm nút (D-232):

| Method + path | Permission | Tóm tắt |
|---|---|---|
| `GET /groups?q=` | `organizations.units.read` | `Page` nhóm; `member_count` chỉ đếm nút trong phạm vi người gọi, `excluded_org_count` số nút ngoài phạm vi |
| `POST /groups` `{name, description?}` | `organizations.groups.manage` | → `Created` |
| `GET /groups/{id}` · `PATCH /groups/{id}` · `DELETE /groups/{id}` | units.read · groups.manage (`If-Match`) · groups.manage | |
| `GET /groups/{id}/members` | `organizations.units.read` | `Page` nút thành viên **trong phạm vi** người gọi; số nút ngoài phạm vi là `excluded_org_count` của `GET /groups/{id}` (D-20261118-step38-groups) |
| `POST /groups/{id}/members` `{org_ids}` | `organizations.groups.manage` | thêm (gọi lại không trùng); nút không thuộc khách → `organizations.group_member_invalid` |
| `DELETE /groups/{id}/members/{org_id}` | `organizations.groups.manage` | bỏ một nút; không có thì vẫn 204 |

### Tool MCP

`reporting.kinds`, `reporting.summary`, `reporting.facts` (cùng service, cùng permission với REST; nhận
`group_ids`). Không có tool cho tách / gộp (việc của người vận hành, không hoàn tác được bằng một lệnh).

D-232: `kernel.sandboxes.list`, `kernel.sandboxes.create`, `kernel.sandboxes.extend`, `kernel.sandboxes.promote`,
`kernel.sandbox_messages.list` (read-only, gọi trong sandbox) — agent thử cấu hình trong sandbox rồi đẩy sang;
không có tool xoá sandbox (chỉ REST có người). `organizations.list_groups` (read-only),
`organizations.create_group`, `organizations.add_group_members`, `organizations.remove_group_member`.

### Event

`reporting` không phát event nghiệp vụ. Kernel: `kernel.relocation_started`, `kernel.relocation_failed` (ghi ở khách
nguồn), `kernel.relocation_finished` (ghi ở khách nhận, kèm khách gốc trước / sau — `reporting` nghe để dời kho), vào
audit (D-20261118-step38-relocation), `tenancy.delegated_access` (nếu bước 34 chưa có).

D-232: kernel `kernel.sandbox_created`, `kernel.sandbox_extended`, `kernel.sandbox_deleted` (ghi ở khách gốc;
payload: id sandbox, hạn — `auth` nghe `created` / `deleted` để tạo / thu hồi uỷ quyền), `kernel.sandbox_promoted`
(ở khách thật). `organizations`: `organizations.group_created`, `organizations.group_updated`,
`organizations.group_deleted`, `organizations.group_members_changed` (payload: id nhóm, số nút thêm / bỏ).

### Job / lịch

`reporting.forward` (lịch, `AUTOXIDE_REPORTING_FORWARD_SECONDS`), `reporting.prune_local` (24 giờ),
`reporting.rehome` (job, khi nghe `kernel.relocation_finished`); `<module>.distribute` (job của module
dùng `Distributor`, `unique_key` theo đích). Mọi bộ chạy chịu được chạy lại.

D-232: `kernel.sandbox_copy` (job, `unique_key = "sandbox_copy:{sandbox}"`, chạy lại tiếp từ module / con trỏ
đã xong), `kernel.expire_sandboxes` (lịch mỗi giờ, database điều khiển, `NO_TENANT`).

### Cấu hình (Setting)

| Biến | Mặc định | Nghĩa |
|---|---|---|
| `AUTOXIDE_REPORTING_FORWARD_SECONDS` | 5 | chu kỳ đẩy fact lên kho của cây (độ trễ báo cáo) |
| `AUTOXIDE_REPORTING_FORWARD_BATCH` | 500 | số fact mỗi transaction ở database gốc |
| `AUTOXIDE_REPORTING_LOCAL_RETENTION_DAYS` | 35 | giữ fact cục bộ đã đẩy để đẩy lại / dời kho |
| `AUTOXIDE_RELOCATION_DRAIN_SECONDS` | 300 | chờ outbox / job của khách rỗng trước khi tách / gộp |
| `AUTOXIDE_TENANCY_SANDBOX_DAYS` | 30 | sandbox không gia hạn thì hết hạn sau chừng này ngày (1–365) (D-232) |
| `AUTOXIDE_TENANCY_SANDBOX_MAX_PRODUCTS` | 50 | trần của tuỳ chọn `products` khi tạo sandbox (0–1000) (D-232) |
| `AUTOXIDE_TENANCY_SANDBOX_DOMAIN` | (trống) | tên miền cha của website sandbox (`<slug>.<giá trị>`); trống = chỉ vào bằng header (D-232) |

Cài đặt theo khách (D-232, `Module::tenant_settings` của kernel): `kernel.sandbox.webhook_urls` — danh sách tiền
tố URL webhook được gửi thật từ sandbox; chỉ có nghĩa ở sandbox.

Sinh lại `.env.example`.

### Permission

`reporting.read` (có phạm vi `unit` / `subtree` / `tenant` như mọi permission của bước 34),
`reporting.manage`. Tách / gộp dùng permission nhân sự nền tảng của bước 33/34 (không phải permission của khách).

D-232: `kernel.sandboxes.read`, `kernel.sandboxes.manage` (chỉ cấp ở độ sâu `tenant`; role `admin` của khách có);
`organizations.groups.manage` (chỉ cấp ở độ sâu `tenant`). Đọc nhóm dùng `organizations.units.read` có sẵn.

### Mã lỗi (`locales/vi.json`)

| Mã | HTTP | Tiếng Việt |
|---|---|---|
| `reporting.kind_unknown` | 422 | Không có loại số liệu `{kind}`. Xem danh sách ở `GET /api/v1/reporting/kinds`. |
| `reporting.range_too_large` | 422 | Khoảng thời gian quá dài cho cách nhóm `{group_by}` (tối đa {max} ngày). |
| `reporting.range_invalid` | 422 | Ngày bắt đầu phải trước hoặc bằng ngày kết thúc. |
| `reporting.dimension_unknown` | 422 | Loại số liệu `{kind}` không có chiều lọc `{dimension}`. |
| `reporting.tenant_not_found` | 404 | Không tìm thấy khách hàng này trong phạm vi bạn được xem. |
| `reporting.store_unavailable` | 503 | Kho báo cáo của cây khách hàng đang bảo trì. Hãy thử lại sau ít phút. |
| `tenancy.distribution_target_invalid` (kernel) | 422 | Khách `{tenant}` không phải khách con của khách gửi. |
| `tenancy.relocation_conflict` (kernel) | 409 | Không tách / gộp được: {count} xung đột, xem báo cáo của lần chạy `{id}`. |
| `tenancy.relocation_in_progress` (kernel) | 409 | Khách này đang có một lần tách / gộp chưa xong (`{id}`). Chạy tiếp bằng `--resume` hoặc chờ nó xong. |
| `tenancy.relocation_not_found`, `…_org_invalid`, `…_same_database`, `…_tenant_exists`, `…_not_active`, `…_platform`, `…_cycle`, `…_schema_differs`, `…_drain_timeout`, `…_count_mismatch` (kernel, D-20261118-step38-relocation) | 404 / 409 | Xem `crates/kernel/locales/vi.json`. |
| `tenancy.sandbox_not_found` (kernel, D-232) | 404 | Không có bản thử `{id}` của khách hàng này. |
| `tenancy.sandbox_nested` (kernel, D-232) | 422 | Không tạo được bản thử từ một bản thử; tạo từ khách hàng thật. |
| `tenancy.sandbox_forbidden` (kernel, D-232) | 403 | Bản thử không làm được việc `{action}`; làm việc này ở khách hàng thật. |
| `tenancy.promote_kind_unknown` (kernel, D-232) | 422 | Không đẩy được loại `{kind}` sang khách thật. Xem danh sách ở `GET /api/v1/kernel/sandboxes/promotable`. |
| `tenancy.promote_too_many` (kernel, D-232) | 422 | Mỗi lần đẩy tối đa {max} mục; chia thành nhiều lần. |
| `reporting.group_not_found` (D-232) | 404 | Không có nhóm nút `{id}` trong khách hàng này. |
| `organizations.group_not_found` (D-232) | 404 | Không có nhóm nút `{id}`. |
| `organizations.group_name_taken` (D-232) | 409 | Tên nhóm `{name}` đã được dùng cho một nhóm khác. |
| `organizations.group_member_invalid` (D-232) | 422 | Các nút sau không thuộc khách hàng này: {ids}. |
| `organizations.group_too_large` (D-232) | 422 | Mỗi nhóm có tối đa {max} nút. |

Vượt hạn mức sandbox dùng `limit_exceeded` (403) của bước 35, không thêm mã.

## Thứ tự làm

1. Kernel `report` (`Fact`, `FactSource`), `Module::facts`, `StartContext::fact_sources`, kiểm tên event lúc
   khởi động → nền cho **mục 1**.
2. Module `reporting`: bảng, handler rút fact, lịch đẩy, cộng dồn ngày, dọn → **mục 1**.
3. `invoices`, `ledger` khai `facts()`; API + tool MCP xem báo cáo, quyền theo phạm vi, nhiều tiền tệ → **mục 1**.
4. Test xuyên database: cha ở database A, con ở database B (testkit `TestDatabase::another`) → **mục 1**.
5. Vào ngữ cảnh con xuyên database: rà handler / tool MCP lấy database qua `TenantContext`, audit hai phía,
   test → **mục 2**.
6. Kernel `tenancy::distribution` (`Distributor`, `DistributionApply`, `distribution_migration`), test với
   module mẫu → **mục 2**.
7. `transfer.rs` chép có lọc + đổi cột; kernel `tenancy::relocate` (chính sách, kế hoạch, dry-run),
   `Module::relocation` + chính sách của mọi module có sẵn (`audit`, `organizations`, `ledger`, `invoices`,
   `kernel`…), `xtask` / testkit kiểm mọi bảng cấp khách có chính sách → **mục 3**.
8. Tách: trạng thái, `--resume`, chuyển mặt điều khiển, dọn, dời kho báo cáo → **mục 3**.
9. Gộp + test khứ hồi (tách rồi gộp lại, so toàn bộ dữ liệu với trước khi tách) → **mục 3**.
10. Route mặt điều khiển `relocations` cho nhân sự nền tảng (dry-run mặc định, job, D-210) + test → **mục D-210**.
11. Nhóm nút: bảng, REST, tool MCP, event của `organizations`; `OrgGroups` trong `api`; `reporting` nhận
    `group_ids` / `group_by=group`, giao với phạm vi, `excluded_org_count` → **mục nhóm nút (D-232)**.
12. Sandbox, phần nền: cột mới của `kernel_tenants`, `TenantKind`, `is_sandbox`, tạo / gia hạn / xoá / hết hạn,
    hạn mức `kernel.sandboxes`, uỷ quyền vào sandbox (`auth`), header `Autoxide-Sandbox` + `noindex`,
    `Targets::Sandbox` → **mục sandbox (D-232)**.
13. Sandbox, cách ly: `Partners` + bộ giả của mọi trait đối tác bước 36–37, `kernel_sandbox_messages`, chặn mail
    (`SandboxRecipients`) và webhook; `ledger`, `invoices`, `tax`, `compliance`, `notifications` chuyển sang
    `Partners`; huy hiệu "bản thử" của `compliance` → **mục sandbox (D-232)**.
14. Sandbox, sao và đẩy: `Module::sandbox_copy` / `promotable`, job `kernel.sandbox_copy`; khai cho `kernel`
    (cài đặt), `organizations`, `auth` (role), `content` (trang, menu), `notifications` (mẫu mail); API promote →
    **mục sandbox (D-232)**.
15. Tài liệu: `docs/MULTI_TENANCY.md` (+ `.vi.md`) — hoặc tài liệu đa khách của bước 33 nếu đã có tên khác —
    thêm phần báo cáo gộp, phân phối, tách / gộp, nhóm nút, sandbox (sao gì, không sao gì, đối tác luôn
    giả); `docs/MODULE_GUIDE.md` (+ `.vi`): khai `facts()`,
    `relocation()`, `sandbox_copy()`, `promotable()`, dùng `Distributor` và `Partners`; `docs/openapi.json`,
    `.env.example`; `CHANGELOG.md`; đánh dấu ROADMAP; gate đầy đủ; phát hành theo `docs/RELEASING.md`.

## Test bắt buộc

Mọi test qua `TestDatabase::run` / `TestApp::builder`, chạy trên cả SQLite và Postgres; "hai database" là hai
database trống cùng loại do testkit cấp.

- Báo cáo: `fact_extracted_once_on_redelivery`, `forward_is_idempotent_after_crash` (đẩy, không đặt
  `forwarded_at`, đẩy lại → tổng không đổi), `forward_survives_root_maintenance` (gốc 503 → fact chờ, gốc mở →
  vào đủ), `child_on_other_database_appears_within_forward_interval`, `daily_rollup_equals_sum_of_facts`
  (sau 200 fact đồng thời từ 4 khách con), `local_day_uses_org_timezone` (23:30 ngày 31 giờ Việt Nam),
  `multi_currency_converted_with_viewer_rates_and_lists_missing`.
- Quyền / rò rỉ: `sibling_tenant_cannot_see_each_other` (hai con của cùng cha), `other_tree_not_found`,
  `subtree_scope_only_sees_own_branch`, `child_sees_only_own_subtree`, `client_filter_cannot_widen_scope`,
  `mcp_tools_same_visibility_as_rest`.
- Vào ngữ cảnh con: `parent_acts_in_child_on_other_database`, `audit_recorded_on_both_sides`,
  `revoked_delegation_blocks_child_access`.
- Phân phối: `distribute_creates_once_on_rerun`, `crash_after_child_write_marks_applied_on_retry`,
  `out_of_order_versions_keep_latest`, `child_without_module_is_skipped`, `concurrent_publish_same_version_once`,
  `non_descendant_target_rejected`.
- Tách / gộp: `split_dry_run_reports_counts_and_conflicts`, `split_keeps_primary_keys_and_rewrites_tenant`,
  `split_moves_ledger_book_and_chain_still_verifies`, `split_reaudits_into_target_chain_source_chain_intact`,
  `split_resume_after_failure_in_each_state`, `split_blocks_writes_only_for_that_tenant`,
  `merge_roundtrip_restores_original_data` (so từng bảng trước tách và sau gộp), `merge_unique_conflict_fails_with_report`,
  `merge_skips_identical_audit_rows`, `reporting_rehomed_after_split`, `every_tenant_table_has_relocation_policy`.
- Cặp chéo SQLite ↔ Postgres cho tách (chỉ khi có cả hai URL, như test của `autoxide copy`).
- Route tách / gộp (D-210): `relocation_api_chi_nhan_su_nen_tang`, `relocation_api_mac_dinh_dry_run`.
- Nhóm nút (D-232): `nhom_gom_nut_hai_nhanh_nguoi_mot_nhanh_chi_thay_phan_minh` (nhóm có nút ở nhánh A và B,
  người chỉ có `reporting.read` ở nhánh A thấy tổng của A, `excluded_org_count` = số nút của B; REST và MCP),
  `nhom_khong_cap_quyen_doc_nut`, `group_by_group_tinh_nut_o_hai_nhom`, `nhom_cua_khach_khac_khong_tim_thay`,
  `them_thanh_vien_goi_lai_khong_trung`, `them_dong_thoi_cung_nut_mot_dong` (đồng thời).
- Sandbox (D-232), mọi test chạy với sandbox ở database chung và ở database riêng của khách gốc:
  `sandbox_khong_sao_du_lieu_nguoi_mua` (khách gốc có người mua, đơn, bút toán, thông báo → sandbox không có
  dòng nào; so mọi bảng cấp khách), `sandbox_sao_cau_hinh_khuyen_mai_nhap_luat_tat`, `sandbox_toi_da_n_san_pham`,
  `doi_tac_that_khong_bao_gio_duoc_goi_trong_sandbox` (đăng ký bộ nối thật đếm số lần gọi cho mọi trait đối tác,
  chạy luồng thanh toán / SMS / eKYC / hoá đơn trong sandbox → 0 lần, hộp thư giả có dòng),
  `mail_toi_nguoi_la_khong_gui` (một mail hai người nhận: nhân viên nhận, người lạ vào hộp thư giả),
  `webhook_ngoai_danh_sach_khong_gui`, `moi_response_sandbox_co_header_va_noindex`,
  `day_sang_chi_tao_ban_nhap` (trang, mẫu mail; bản đang đăng ở khách thật không đổi),
  `day_hai_lan_khong_trung`, `day_dong_thoi_cung_muc_mot_ban` (đồng thời, cùng `items`, khác `Idempotency-Key`),
  `day_trung_duong_dan_bao_xung_dot`, `sandbox_het_han_bi_xoa` (đồng hồ giả qua `SANDBOX_DAYS`),
  `vuot_han_muc_sandbox_tra_limit_exceeded`, `tao_sandbox_tu_sandbox_bi_tu_choi`,
  `ro_ri_giua_sandbox_va_khach_goc` (`leak_check` hai chiều: REST, MCP, báo cáo, con trỏ trang),
  `fact_cua_sandbox_khong_vao_kho_cua_cay`, `sao_lai_sau_khi_job_chet_khong_trung`.

## Tiêu chí xong

- [ ] Mọi mục `- [ ]` của bước 38 trong ROADMAP đánh `[x]` (gồm D-210, sandbox và nhóm nút của D-232).
- [ ] Mọi trait đối tác có bộ giả trong `Partners`; test chứng minh sandbox không gọi bộ nối thật (D-232).
- [ ] `./gate.sh` đầy đủ xanh trên SQLite và Postgres; `cargo xtask check` xanh.
- [ ] Mọi bảng cấp khách của mọi module có chính sách tách / gộp (test hay `xtask` chặn bảng thiếu).
- [ ] Đo độ trễ báo cáo (từ commit event ở con tới có trong `reporting_daily`) và tải của `reporting.forward`
      với ≥ 100 khách con trên một database; ghi số đo kèm máy, ngày, cỡ dữ liệu vào quyết định.
- [ ] `docs/DECISIONS.md` có mục mới: kho báo cáo ở database của khách gốc (hai chặng cục bộ → gốc);
      `Module::facts`; khung phân phối; tách / gộp giữ khoá chính và cách xử lý audit.
- [ ] Tài liệu cho người ngoài EN + VI cùng nội dung; `MODULE_GUIDE` (+ `.vi`); `docs/openapi.json`,
      `.env.example` sinh lại; `CHANGELOG.md`; phát hành theo `docs/RELEASING.md`.

## Chỗ dễ sai

- **Con trỏ theo `id` UUIDv7 bỏ sót dòng**: id sinh lúc ghi, commit có thể theo thứ tự khác; dòng id nhỏ commit
  muộn sẽ lọt sau con trỏ. Vì thế đẩy theo cờ `forwarded_at IS NULL`, không theo "id lớn hơn lần trước".
- **Đếm hai lần** khi cộng dồn: chỉ cộng vào `reporting_daily` khi INSERT fact thật sự chèn dòng (đọc
  `rows_affected`), trong cùng transaction ở database gốc. `ON CONFLICT … DO UPDATE` của upsert cộng dồn chạy
  trên SQLite ≥ 3.24 và Postgres; không dùng `INSERT OR REPLACE` (chỉ SQLite, và xoá dòng cũ).
- **Transaction xuyên hai database không có**: mọi bước hai database phải là "ghi đích idempotent rồi đánh dấu
  nguồn"; không bao giờ giữ transaction ở database này trong lúc chờ database kia (trên SQLite chặn mọi lệnh
  ghi khác của database đó).
- **Event handler dài**: bộ phát cắt ở 50 giây (D-158) — handler rút fact chỉ ghi cục bộ; đẩy là job.
- **Ngày báo cáo**: `local_day` tính theo múi giờ của nút lúc rút fact; đổi múi giờ sau đó không tính lại fact cũ
  (ghi trong tài liệu).
- **Tổng tiền quy đổi**: không cộng hai loại tiền với nhau trước khi quy đổi; quy đổi theo nhóm (tiền, ngày) rồi
  mới cộng; thiếu tỉ giá thì báo, không coi là 0.
- **Rò rỉ qua lọc**: tham số `tenant_ids` của client chỉ thu hẹp; kiểm khách thuộc cây bằng `descendants` của
  khách người gọi, không tin `tenant_id` trong fact.
- **Giữ khoá chính khi tách / gộp**: UUIDv7 là duy nhất toàn cục nên không đụng nhau — trừ bản đã chép trước
  đó (audit); unique nghiệp vụ (slug, mã) mới là chỗ đụng thật — phát hiện ở kế hoạch, không phải giữa lúc chép.
- **Chuỗi băm**: đổi `tenant_id` không được làm gãy chuỗi sổ cái (bước 36 không băm `tenant_id`); chuỗi audit
  không di chuyển được dòng — ghi lại ở đích, giữ nguyên ở nguồn.
- **Bảng `MustBeEmpty`** (outbox, jobs, idempotency key, phiên đăng nhập): chép event chưa giao sang đích sẽ
  giao hai lần; chờ rỗng (hay huỷ phiên, bắt đăng nhập lại) thay vì chép.
- **Thứ tự bảng khi chép / xoá**: chép theo thứ tự tạo bảng (foreign key trong module), xoá theo thứ tự ngược;
  Postgres kiểm foreign key ngay mỗi câu, SQLite chỉ khi `PRAGMA foreign_keys` bật — test cả hai.
- **Sổ kết nối có giới hạn** (bước 33): lịch đẩy duyệt 100 khách con không được mở 100 kết nối cùng lúc — xử lý
  tuần tự theo database gốc, gom các khách cùng gốc.
- **Khách cha đổi** (gắn con vào cha khác): gốc đổi → phải dời kho báo cáo (`reporting.rehome`), không thì báo cáo
  của gốc mới thiếu lịch sử và gốc cũ vẫn thấy dữ liệu của khách không còn thuộc mình.
- **Sandbox lọt vào cây** (D-232): sandbox có `parent_id` NULL; mọi chỗ duyệt cây (`descendants`, phân phối
  `AllChildren`, báo cáo, uỷ quyền theo cây) phải bỏ `kind = sandbox`, không thì số liệu thử lẫn vào báo cáo
  thật. Test `fact_cua_sandbox_khong_vao_kho_cua_cay` giữ điều này.
- **Giữ `Arc` đối tác từ lúc khởi động** (D-232): module lấy `contributions::<dyn PaymentGateway>()` một lần
  trong `start` rồi dùng cho mọi khách sẽ gọi đối tác thật trong sandbox. Chỉ lấy qua `Partners::get(&tenant)`
  ở từng lần gọi; `xtask` chặn `contributions` / `optional` / `require` với trait đối tác ngoài `Partners`.
- **Sao cả secret** (D-232): cài đặt khách có `secret = true` (thông tin kết nối đối tác, SMTP) không bao giờ sang
  sandbox — sandbox dùng bộ giả nên không cần, và sao là nhân bản secret.
- **ID trong sandbox**: cùng database với khách gốc, nhiều bảng có PK chỉ `id` — giữ nguyên ID sẽ đụng khoá. Bản
  sao có ID mới, nối về gốc bằng `source_record_id`; đẩy sang dùng nó để tìm bản ghi gốc.
- **Đẩy sang ghi đè bản đang chạy**: `PromoteApply` chỉ tạo / sửa nháp hay bản sửa đổi chờ đăng; khuyến mại và
  luật tự động hoá nhận ở trạng thái tắt dù ở sandbox đang bật. Đăng / bật là thao tác riêng của người ở khách thật.
- **Mail lách qua đường khác**: `notifications`, `newsletter`, `forms`, `comments` đều gửi qua `dyn Mailer` — chặn
  sandbox nằm ở bên cài `dyn Mailer` (module `mail`), không ở từng module gửi; SMS / ZNS / push đi qua `Partners`.
- **Nhóm thành lỗ quyền** (D-232): kết quả theo nhóm luôn là giao của thành viên với phạm vi người gọi; không dùng
  "người có quyền quản lý nhóm thì xem được cả nhóm". `member_count` / `excluded_org_count` chỉ lộ số lượng, không
  lộ ID hay tên nút ngoài phạm vi.

## Căn cứ pháp lý

- Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 + NĐ 356/2025 (hiệu lực 01/01/2026): kho gộp không chứa dữ liệu cá
  nhân; tách / gộp chuyển dữ liệu cá nhân giữa hai khách là chuyển giao giữa hai bên kiểm soát dữ liệu khi hai
  khách là hai pháp nhân — người vận hành phải có căn cứ (hợp đồng, đồng ý); lệnh CLI in cảnh báo và audit ghi lý
  do (`--reason` bắt buộc). Cần đối chiếu văn bản gốc / người có chuyên môn pháp lý.
- Luật An ninh mạng 116/2025/QH15 (hiệu lực 01/7/2026) và vị trí lưu dữ liệu theo khách (bước 37): kho báo cáo của
  cây nằm ở database của khách gốc — tài liệu nói rõ để khách con chọn vị trí lưu biết số liệu tổng hợp của mình
  nằm ở đâu.
- Luật Kế toán 88/2015, Điều 41: tách / gộp giữ nguyên chứng từ, sổ cái và chuỗi niêm phong; không xoá bản ở nguồn
  trước khi bước `verified` xong.
- Sandbox (D-232): Luật 91/2025/QH15 + NĐ 356/2025 (hiệu lực 01/01/2026), nguyên tắc tối thiểu hoá — chỉ sao cấu
  hình, không sao dữ liệu cá nhân; dữ liệu nhân viên nhập thử trong sandbox xoá theo sandbox. NĐ 52/2013/NĐ-CP
  (sửa bởi NĐ 85/2021/NĐ-CP) và Luật Thương mại điện tử 2025: website bán hàng phải thông báo với Bộ Công Thương —
  website sandbox không bán thật (đối tác giả, `noindex`, ghi "bản thử", không gắn logo đã thông báo); có phải
  thông báo website thử hay không: cần đối chiếu văn bản gốc / người có chuyên môn pháp lý. Kết quả eKYC, hoá đơn,
  thanh toán của bộ giả không có giá trị pháp lý và không đẩy sang khách thật được.

## Agent làm bước này tự quyết

Ghi quyết định vào `docs/DECISIONS.md` (skill `record-decision`).

- **Nơi đặt kho báo cáo**: thiết kế này đặt ở database của khách gốc (một cách, không thêm cấu hình). Nếu đo thấy
  kho làm chậm database gốc, thêm tuỳ chọn database báo cáo riêng (`kernel_placements` cho "kho của cây") — ghi
  quyết định kèm số đo.
- **Mức cộng dồn**: theo ngày là đủ cho bước này; cộng dồn theo dimension (sản phẩm, kênh) để module 45 tự thêm
  bảng cộng dồn của mình hay thêm vào `reporting` — khuyến nghị đọc fact thô có index tới khi đo thấy chậm.
- **Khung phân phối ở kernel hay module**: khuyến nghị kernel (nhiều module dùng: `catalog`, bảng giá, khuyến mại),
  bảng thuộc module dùng nó qua `distribution_migration` như outbox.
- **Dữ liệu `TenantWide` có dữ liệu cá nhân** (người mua theo khách — bước 34) khi tách: mặc định `Stay` và báo
  tham chiếu vắt ranh giới trong kế hoạch; chép người mua sang khách mới chỉ khi người vận hành bật cờ và có căn cứ
  pháp lý — chọn và ghi quyết định.
- **Bộ giả của sandbox trả gì** (D-232): kết quả mặc định (thanh toán thành công…) hay cho người thử chọn kịch bản
  (thất bại, hết hạn) qua cài đặt của sandbox — khuyến nghị bắt đầu bằng kết quả xác định, thêm kịch bản khi cần.
