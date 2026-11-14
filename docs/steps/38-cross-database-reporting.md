# Bước 38 — Cây khách hàng xuyên database và báo cáo gộp

> Lộ trình: `docs/ROADMAP.md` bước 38 · Thiết kế: `docs/research/business-foundation.md` trụ 2 (đoạn "Cây
> khách hàng xuyên database"), mục 5 (mô hình 2 và 4) · Quyết định: D-185, D-190, D-054 (bên nhận event
> chịu nhận lại), D-184 (reference), D-070 (`autoxide copy`), D-183 (chuỗi băm audit)

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

Mọi thứ chạy và có test trên cả SQLite (mỗi database một file) và Postgres (mỗi database một database).

## Bắt đầu khi

- Bước 34 đã merge: cây khách hàng trong mặt điều khiển (`kernel_tenants` có khách cha), uỷ quyền theo cây
  khách hàng, `Principal` mang tenant + `Scope`, cây tổ chức `organizations` (`dyn OrgDirectory`, bảng tổ
  tiên – con cháu, cài đặt kế thừa: múi giờ, tiền tệ), dữ liệu nghiệp vụ có `org_id`.
- Bước 33: `DatabaseRegistry` (mở database của khách bất kỳ, mở lười, có giới hạn), `kernel_placements`
  (`shared` / `dedicated`), worker và bộ phát event chạy trên từng database, `Delivered` mang tenant của
  event, lỗi `tenant_maintenance`, `autoxide copy` mở rộng để dời cả một khách.
- Bước 36 đã merge: `ledger`, `invoices` (fact tiền đầu tiên; tỉ giá qua `dyn Invoices::exchange_rate`).
- Làm song song được với 39, 40, 41–45. Module 41–45 tự khai fact của mình theo cơ chế ở đây (bước 45 có mục
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

**Module mới** `modules/reporting` · crate `autoxide-reporting` — đăng ký **cuối cùng** trong
`crates/app/src/modules.rs`. Sở hữu bảng `reporting_…`, permission `reporting.*`, lỗi `reporting.*`.

**Sửa module có sẵn**: `ledger`, `invoices` khai `facts()`; mọi module khai `relocation()` cho bảng cấp khách
không có `org_id` (mặc định suy từ phân loại bảng của bước 33–34, xem dưới); `audit` cài chính sách tách /
gộp riêng (nối vào chuỗi băm của database đích). CLI `crates/app/src/commands/tenant.rs`: `autoxide tenant
split|merge|relocation-status`.

**Ngoài phạm vi**: dashboard / giao diện (luật 9); báo cáo theo nghiệp vụ của module chưa có (đơn hàng,
tồn kho — module đó tự khai fact khi dựng); kho dữ liệu cột (OLAP) hay database báo cáo ngoài SQLite /
Postgres; đồng bộ hai chiều con → cha của dữ liệu nghiệp vụ (chỉ số liệu báo cáo đi lên, dữ liệu đi xuống
bằng phân phối).

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
| `reporting_daily` | cộng dồn theo ngày: `tenant_id` (gốc), `source_tenant_id`, `org_id` (UUID nil khi không có), `kind`, `local_day`, `currency` (chuỗi rỗng khi không có tiền), `amount_minor`, `quantity`, `fact_count` | PK (`tenant_id`, `source_tenant_id`, `org_id`, `kind`, `local_day`, `currency`) |
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
`cleaned` → `done`; hoặc `failed`), `plan` (JSON: bảng, số dòng, cảnh báo, xung đột), `error`, `created_by`,
`created_at`, `updated_at`). Khách hàng có thêm trạng thái `merged` + `merged_into` (cột mới trên
`kernel_tenants` — migration mới, không sửa migration của bước 33).

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
   (tiền, ngày)** bằng tỉ giá của **khách người xem** ngày đó (`dyn Invoices::exchange_rate`, gọi trước, ngoài
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

### API REST

**`/api/v1/reporting`** (`TenantContext` + `Principal`, envelope chuẩn):

| Method + path | Permission | Tóm tắt |
|---|---|---|
| `GET /kinds` | `reporting.read` | các loại fact đã khai (kind, mô tả, đơn vị: tiền / số, dimension được phép) |
| `GET /summary?kind=&from=&to=&group_by=day\|week\|month\|tenant\|org&tenant_ids=&org_ids=&currency=` | `reporting.read` | tổng theo nhóm: `by_currency[]` (`amount`, `quantity`, `count`), `converted` (khi có `currency`), `missing_rates[]`; đọc `reporting_daily`; khoảng tối đa 366 ngày khi nhóm theo ngày (`reporting.range_too_large`) |
| `GET /facts?kind=&from=&to=&tenant_ids=&org_ids=&dimension.<tên>=` | `reporting.read` | `Page` fact thô để xem chi tiết (`fetch_page`, `limit`/`cursor`/`order`) |
| `GET /lag` | `reporting.manage` | với từng khách trong cây: số fact chưa đẩy, fact cũ nhất chưa đẩy, lần đẩy cuối |
| `POST /rebuild` | `reporting.manage` | đặt lại `forwarded_at` của fact cục bộ còn giữ cho một khách (đẩy lại; unique chặn trùng); trả số dòng |

Không có trang công khai.

Phân phối không có route chung ở kernel: module dùng nó tự mở `POST /api/v1/<module>/…/distribute` và
`GET /api/v1/<module>/distributions/{id}` (gọi `Distributor::publish` / `status`), permission của module.

Tách / gộp: CLI (`autoxide tenant split|merge|relocation-status`) cho người vận hành; nếu bước 33 đã có
nhóm route mặt điều khiển cho nhân sự nền tảng thì thêm `POST …/relocations` (dry-run mặc định),
`GET …/relocations/{id}` vào đó, permission nhân sự nền tảng của bước 33/34.

### Tool MCP

`reporting.kinds`, `reporting.summary`, `reporting.facts` (cùng service, cùng permission với REST). Không có
tool cho tách / gộp (việc của người vận hành, không hoàn tác được bằng một lệnh).

### Event

`reporting` không phát event nghiệp vụ. Kernel: `tenancy.relocation_started`, `tenancy.relocation_finished`,
`tenancy.relocation_failed` (ghi ở mặt điều khiển, vào audit), `tenancy.delegated_access` (nếu bước 34 chưa có).

### Job / lịch

`reporting.forward` (lịch, `AUTOXIDE_REPORTING_FORWARD_SECONDS`), `reporting.prune_local` (24 giờ),
`reporting.rehome` (job, khi `kernel_relocations` hay đổi khách cha); `<module>.distribute` (job của module
dùng `Distributor`, `unique_key` theo đích). Mọi bộ chạy chịu được chạy lại.

### Cấu hình (Setting)

| Biến | Mặc định | Nghĩa |
|---|---|---|
| `AUTOXIDE_REPORTING_FORWARD_SECONDS` | 5 | chu kỳ đẩy fact lên kho của cây (độ trễ báo cáo) |
| `AUTOXIDE_REPORTING_FORWARD_BATCH` | 500 | số fact mỗi transaction ở database gốc |
| `AUTOXIDE_REPORTING_LOCAL_RETENTION_DAYS` | 35 | giữ fact cục bộ đã đẩy để đẩy lại / dời kho |
| `AUTOXIDE_RELOCATION_DRAIN_SECONDS` | 300 | chờ outbox / job của khách rỗng trước khi tách / gộp |

Sinh lại `.env.example`.

### Permission

`reporting.read` (có phạm vi `unit` / `subtree` / `tenant` như mọi permission của bước 34),
`reporting.manage`. Tách / gộp dùng permission nhân sự nền tảng của bước 33/34 (không phải permission của khách).

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
10. Tài liệu: `docs/MULTI_TENANCY.md` (+ `.vi.md`) — hoặc tài liệu đa khách của bước 33 nếu đã có tên khác —
    thêm phần báo cáo gộp, phân phối, tách / gộp; `docs/MODULE_GUIDE.md` (+ `.vi`): khai `facts()`,
    `relocation()`, dùng `Distributor`; `docs/openapi.json`, `.env.example`; `CHANGELOG.md`; đánh dấu ROADMAP;
    gate đầy đủ; phát hành theo `docs/RELEASING.md`.

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

## Tiêu chí xong

- [ ] Ba mục `- [ ]` của bước 38 trong ROADMAP đánh `[x]`.
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
- **Route mặt điều khiển cho tách / gộp**: chỉ CLI ở bước này, hay thêm API cho nhân sự nền tảng — theo việc
  bước 33 đã dựng nhóm route nào.

## Bổ sung theo đối chiếu thiết kế 2 (D-208…D-218)

> Nguồn: `docs/prompts/ui-trace-roadmap.md`; mã `A-…` / `B-…` trỏ tới `docs/research/ui-trace/`.

- **Tách / gộp (D-210).** Route mặt điều khiển `POST …/relocations` (dry-run mặc định) chỉ cho nhân sự nền
  tảng; khách chỉ gửi yêu cầu (vào hộp việc của nhân sự nền tảng). Tiến độ qua job API chung (D-213).
