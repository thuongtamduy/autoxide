# Bước 36 — Tiền, sổ cái, thuế, chứng từ

> Lộ trình: `docs/ROADMAP.md` bước 36 · Thiết kế: `docs/research/business-foundation.md` trụ 4, mục 7, mục 9
> · Quyết định: D-180 (Money), D-181 (rows), D-182 (write_with_retry), D-183 (chuỗi băm audit),
> D-184 (reference), D-186, D-187, D-188, D-190, D-194

## Mục tiêu

Xong bước này, một khách (cửa hàng độc lập, chuỗi, SaaS, sàn) có: tính toán thập phân có quy tắc
làm tròn rõ; **sổ cái kép** chỉ thêm, khoá sổ theo kỳ, chống sửa bằng chuỗi băm; nhận tiền qua
**đối tác có giấy phép** (`PaymentGateway`, `PayoutGateway`) hay chuyển khoản **VietQR tự khớp**
(`BankFeed`) theo một trong ba chế độ dòng tiền, mỗi chế độ sinh đúng bút toán; **engine thuế** đọc
biểu thuế có ngày hiệu lực (gói "vn"); **chứng từ** đánh số tuần tự không nhảy số, hoá đơn điện tử qua
`EInvoiceProvider` (kể cả từ máy tính tiền, phát hành thay người bán khi được uỷ nhiệm), tỉ giá, mẫu in
HTML/PDF, xuất dữ liệu kế toán, lưu ≥ 10 năm. **Core không bao giờ tự giữ tiền** (D-187): không có tài
khoản ví, không có số dư "của người mua" nằm ở nền tảng; sổ cái chỉ ghi **nghĩa vụ** và đối soát với
sao kê của đối tác. Module thương mại (41–49) và tính tiền SaaS (35) dựng trên đây.

## Bắt đầu khi

- Bước 34 đã merge: `TenantId`, `OrgId`, `TenantContext`, `Principal` mang tenant và `Scope`, cây tổ
  chức (`dyn OrgDirectory`), cài đặt kế thừa theo cây (múi giờ, tiền tệ, hồ sơ thuế), module `address`
  (kiểu `Address` cho người mua / người bán trên chứng từ).
- Bước 33: kho secret theo khách (thông tin kết nối đối tác của từng khách lưu mã hoá), sổ database
  theo khách `DatabaseRegistry`, phân loại bảng mặt điều khiển / cấp khách hàng.
- Làm song song được với 35 và 37. Mục tính tiền của 35 dùng `PaymentGateway` + `numbering` của bước
  này: làm mục 1–5 của "Thứ tự làm" trước để 35 không phải chờ cả bước.

## Phạm vi

**Kernel (`crates/kernel`)** — chỉ phần trung lập, không có khái niệm sổ cái, hoá đơn hay luật một nước:

| File mới | Nội dung |
|---|---|
| `src/decimal.rs` | re-export `rust_decimal::Decimal`, `Rounding`, cột số thập phân dạng chữ, JSON chuỗi; chuyển `Money` ↔ `Decimal` |
| `src/numbering.rs` | đánh số tuần tự không nhảy số, bảng `kernel_sequences` (cấp khách, có ở MỌI database dữ liệu) |
| `src/payment.rs` | trait `PaymentGateway`, `PayoutGateway`, `BankFeed`, `ExchangeRateSource` + kiểu dữ liệu |
| `src/einvoice.rs` | trait `EInvoiceProvider` + kiểu dữ liệu |
| sửa `src/rows.rs` | thêm `lock_one_shared` (API mới, xem dưới) |
| sửa `src/module.rs` | thêm `StartContext::contribute` / `contributions` (API mới, nhiều bên cùng cung cấp một trait) |

**Testkit**: `FakePaymentGateway`, `FakePayoutGateway`, `FakeBankFeed`, `FakeEInvoiceProvider`,
`FakeExchangeRateSource` (như `FakeTextModel`), kèm hàm dựng webhook đã ký cho test.

**Ba module mới** (thứ tự đăng ký trong `crates/app/src/modules.rs`: … `plans` → **`tax` → `ledger` →
`invoices`** → `compliance` …):

| Crate | Sở hữu | Công bố |
|---|---|---|
| `modules/tax` · `autoxide-tax` | vùng thuế, nhóm thuế, biểu thuế, gói dữ liệu (mặt điều khiển); hồ sơ thuế (cấp khách) | `dyn TaxEngine` |
| `modules/ledger` · `autoxide-ledger` | sổ, tài khoản, bút toán, kỳ, chuỗi băm; thanh toán, webhook đối tác, chi trả, tạm giữ, VietQR, sao kê, hàng chờ sai lệch, cài đặt dòng tiền | `dyn Ledger` |
| `modules/invoices` · `autoxide-invoices` | chứng từ, ký hiệu, uỷ nhiệm lập hoá đơn, hoá đơn điện tử, tỉ giá, mẫu in, bản xuất kế toán, bản lưu trữ | `dyn Invoices` |

Thanh toán nằm trong `ledger` (không tách module `payments`): mọi đồng tiền đi qua đối tác đều phải
thành bút toán và được đối soát với sổ — một module giữ cả hai thì không có transaction xuyên module.

**Ngoài phạm vi**: bộ nối đối tác thật (VNPay, MoMo, ngân hàng, VNPT/Viettel/MISA hoá đơn, Casso/SePay
đọc sao kê…) — cần tài khoản thử, dừng hỏi chủ dự án (D-191); giỏ hàng, đơn hàng, phí sàn theo ngành
hàng, hoàn tiền theo đơn (45, 46); điểm thưởng, thẻ quà tặng (47); báo cáo gộp (38); kê khai thuế nộp
cơ quan thuế (46); bảng tài khoản kế toán đầy đủ theo chế độ kế toán (chỉ ánh xạ mục đích → số hiệu).

## Thiết kế

### Kiểu và trait (phác thảo — tra API thật bằng skill `check-library-api`, `cargo check` xác nhận)

**`autoxide_kernel::decimal`** (D-188; `rust_decimal` 1.43 đã có trong `Cargo.lock` qua
`sea-orm-migration` — khai ở `[workspace.dependencies]`, KHÔNG bật feature `with-rust_decimal` của
sea-orm, ghi `STACK.md`):

```rust
pub use rust_decimal::Decimal;
/// Quy tắc làm tròn — mỗi chỗ dùng chọn một, không có mặc định ngầm.
pub enum Rounding { HalfUp /* MidpointAwayFromZero */, HalfEven, Down /* ToZero */, Up /* AwayFromZero */, Floor, Ceiling }
impl Money {
    pub fn to_decimal(self) -> Decimal;                       // 1999 USD → 19.99, chính xác
    pub fn from_decimal(value: Decimal, currency: Currency, rounding: Rounding) -> Result<Money, Error>; // tràn i64 → money.overflow
    pub fn apply_rate(self, rate: Decimal, rounding: Rounding) -> Result<Money, Error>;   // self × rate
    pub fn convert(self, rate: Decimal, to: Currency, rounding: Rounding) -> Result<Money, Error>;
}
/// Cột số thập phân lưu dạng chữ chuẩn hoá ("0.08", "25345.5") — không dùng kiểu thập phân của database.
pub fn to_column(value: Decimal) -> String;   pub fn from_column(text: &str) -> Result<Decimal, Error>;
/// serde + utoipa: số thập phân trên JSON là CHUỖI, pattern ^-?[0-9]+(\.[0-9]+)?$ (như `Money.amount`).
pub struct DecimalString(pub Decimal);
```

Không bao giờ đi qua `f64`. Bảng làm tròn theo chỗ dùng (là cài đặt của gói thuế / module, không viết
cứng ở chỗ khác):

| Chỗ dùng | Quy tắc mặc định | Ghi chú |
|---|---|---|
| Thuế GTGT trên chứng từ (gói "vn") | `HalfUp` tới đơn vị nhỏ nhất, **theo từng nhóm thuế suất của cả chứng từ**, rồi chia lại về dòng bằng `Money::allocate` theo tỉ lệ giá chưa thuế | tổng dòng luôn bằng tổng chứng từ; cần đối chiếu cách cơ quan thuế kiểm |
| Tách thuế từ giá đã gồm thuế | `net = round(gross / (1 + r))`, `tax = gross − net` | không làm tròn hai lần |
| Thuế khấu trừ thay | `HalfUp` theo từng lần khấu trừ | |
| Quy đổi ngoại tệ | `HalfUp` tới đơn vị nhỏ nhất của tiền đích; nhiều dòng thì quy đổi tổng rồi `allocate` | bút toán luôn cân ở cả tiền gốc và tiền hạch toán |
| Chiết khấu % (module 43) | module tự chọn, mặc định `Down` | |

**`autoxide_kernel::numbering`** — bảng `kernel_sequences` (cấp khách hàng; migration của module nội bộ
`kernel`, chạy ở MỌI database dữ liệu vì số phải cấp **trong cùng transaction** với bản ghi):

```rust
/// Số kế tiếp của dãy `scope` (vd "invoices:{org}:{kind}:{symbol}:{year}") trong transaction GHI `conn`.
/// Hai câu: INSERT … ON CONFLICT DO NOTHING (last_value = 0), rồi
/// UPDATE kernel_sequences SET last_value = last_value + 1, updated_at = ? WHERE tenant_id = ? AND scope = ? RETURNING last_value
/// (Postgres; SQLite ≥ 3.35 — sea-orm đã bật `sqlite-use-returning-for-3_35`). Transaction rollback ⇒ số không bị tiêu.
pub async fn next<C: ConnectionTrait>(conn: &C, tenant: TenantId, scope: &str) -> Result<i64, Error>;
pub async fn current<C: ConnectionTrait>(conn: &C, tenant: TenantId, scope: &str) -> Result<i64, Error>;
```

**API mới cần thêm ở kernel**:

```rust
// rows.rs — khoá chia sẻ: Postgres `SELECT … FOR SHARE`, SQLite câu SELECT thường (BEGIN IMMEDIATE đã giữ quyền ghi).
// Nhiều bút toán cùng giữ khoá chia sẻ trên dòng kỳ kế toán; khoá sổ lấy `lock_one` (độc quyền) nên phải chờ chúng.
pub async fn lock_one_shared<E: EntityTrait, C: ConnectionTrait>(conn: &C, select: Select<E>) -> Result<Option<E::Model>, Error>;
// module.rs — nhiều module bộ nối cùng cung cấp một trait (provide/require chỉ cho một bên).
pub fn contribute<T: ?Sized + Send + Sync + 'static>(&mut self, api: Arc<T>) -> Result<(), Error>;
pub fn contributions<T: ?Sized + Send + Sync + 'static>(&self) -> Vec<Arc<T>>; // của các module đăng ký TRƯỚC
```

Testkit thêm `TestAppBuilder::contribute::<dyn T>(…)` tương ứng.

**`autoxide_kernel::payment`** (trait trong kernel như `TextModel`; bộ nối đối tác là module riêng
chỉ phụ thuộc kernel, đăng ký TRƯỚC `ledger`, gọi `context.contribute::<dyn PaymentGateway>(…)`).
Mọi hàm nhận `tenant: TenantId`: bộ nối tự đọc thông tin kết nối của khách đó từ kho secret theo khách
(bước 33) — không đọc biến môi trường cho thứ thuộc về khách.

```rust
#[async_trait] pub trait PaymentGateway: Send + Sync {
    fn provider(&self) -> &'static str;                       // "vnpay", "fake"…
    fn capabilities(&self) -> GatewayCapabilities;            // methods, refunds, partial_refunds, split, collection_on_behalf, statements
    async fn create_payment(&self, tenant: TenantId, intent: &PaymentIntent) -> Result<GatewayPayment, Error>;
    async fn refund(&self, tenant: TenantId, intent: &RefundIntent) -> Result<GatewayRefund, Error>;
    async fn payment_status(&self, tenant: TenantId, provider_ref: &str) -> Result<GatewayPaymentStatus, Error>;
    /// Kiểm chữ ký (so sánh thời gian hằng), cửa sổ thời gian chống phát lại; lỗi → `payment.signature_invalid`.
    fn verify_webhook(&self, tenant: TenantId, headers: &http::HeaderMap, body: &[u8]) -> Result<GatewayNotification, Error>;
    async fn statement(&self, tenant: TenantId, from: NaiveDate, to: NaiveDate, cursor: Option<String>) -> Result<StatementPage, Error>;
}
#[async_trait] pub trait PayoutGateway: Send + Sync {
    fn provider(&self) -> &'static str;
    async fn payout(&self, tenant: TenantId, intent: &PayoutIntent) -> Result<GatewayPayout, Error>;
    async fn payout_status(&self, tenant: TenantId, provider_ref: &str) -> Result<GatewayPayoutStatus, Error>;
    fn verify_webhook(&self, tenant: TenantId, headers: &http::HeaderMap, body: &[u8]) -> Result<PayoutNotification, Error>;
}
#[async_trait] pub trait BankFeed: Send + Sync {            // ngân hàng hay dịch vụ đọc biến động số dư
    fn provider(&self) -> &'static str;
    fn verify_webhook(&self, tenant: TenantId, headers: &http::HeaderMap, body: &[u8]) -> Result<Vec<BankTransaction>, Error>; // kiểu đẩy
    async fn fetch(&self, tenant: TenantId, account: &BankAccountRef, cursor: Option<String>) -> Result<BankBatch, Error>;      // kiểu kéo
}
#[async_trait] pub trait ExchangeRateSource: Send + Sync {
    fn provider(&self) -> &'static str;
    async fn rates(&self, tenant: TenantId, base: Currency, on: NaiveDate) -> Result<Vec<QuotedRate>, Error>;
}
pub struct PaymentIntent { pub payment_id: Uuid /* gửi làm mã đơn của merchant: đối tác chống trùng */, pub amount: Money,
    pub method: PaymentMethod, pub description: String, pub return_url: Option<String>, pub expires_at: DateTime<Utc>,
    pub splits: Vec<Split> /* chỉ khi capabilities.split */ }
pub struct BankTransaction { pub provider_txn_id: String, pub account_number: String, pub direction: Direction,
    pub amount: Money, pub memo: String, pub booked_at: DateTime<Utc> }
```

Mã lỗi chung của nhóm này ở kernel: `payment.signature_invalid` (401), `payment.provider_unavailable`
(503), `payment.provider_rejected` (422), `payment.not_supported` (422).

**`autoxide_kernel::einvoice`**:

```rust
#[async_trait] pub trait EInvoiceProvider: Send + Sync {
    fn provider(&self) -> &'static str;
    fn capabilities(&self) -> EInvoiceCapabilities; // kinds (Vat, Sales, CashRegister, WithholdingCertificate…), assigns_numbers, adjust, replace, cancel
    async fn issue(&self, tenant: TenantId, invoice: &EInvoice) -> Result<IssuedEInvoice, Error>; // provider_ref, number? , authority_code?, lookup_code, xml
    async fn status(&self, tenant: TenantId, provider_ref: &str) -> Result<EInvoiceStatus, Error>;  // Pending | Accepted{authority_code} | Rejected{reason}
    async fn cancel(&self, tenant: TenantId, request: &EInvoiceCancel) -> Result<(), Error>;        // thông báo sai sót
    async fn download(&self, tenant: TenantId, provider_ref: &str, format: EInvoiceFormat) -> Result<bytes::Bytes, Error>;
}
pub struct EInvoice { pub kind: EInvoiceKind, pub seller: Party, pub on_behalf_of: Option<Party> /* uỷ nhiệm */,
    pub buyer: Party, pub series: String, pub number: Option<i64>, pub issue_date: NaiveDate, pub currency: Currency,
    pub exchange_rate: Option<Decimal>, pub lines: Vec<EInvoiceLine>, pub totals: Vec<TaxTotal>,
    pub related: Option<RelatedInvoice> /* điều chỉnh / thay thế */, pub payment_method: String }
```

Lỗi: `einvoice.provider_unavailable` (503), `einvoice.rejected` (422), `einvoice.not_supported` (422).

**`autoxide_tax::api`**:

```rust
#[async_trait] pub trait TaxEngine: Send + Sync {
    /// Tính thuế, chỉ đọc, KHÔNG gọi trong transaction ghi. Kết quả mang `snapshot` để lưu vào chứng từ.
    async fn quote(&self, tenant: &TenantContext, request: TaxQuoteRequest) -> Result<TaxQuote, Error>;
    async fn withholding(&self, tenant: &TenantContext, request: WithholdingRequest) -> Result<WithholdingQuote, Error>;
    async fn profile(&self, tenant: &TenantContext, org: OrgId) -> Result<Option<TaxProfile>, Error>; // kế thừa theo cây
    async fn invoice_requirements(&self, tenant: &TenantContext, org: OrgId, on: NaiveDate) -> Result<InvoiceRequirements, Error>;
}
pub struct TaxQuoteRequest { pub jurisdiction: String /* "VN" */, pub tax_point: NaiveDate, pub seller: OrgId,
    pub buyer_kind: BuyerKind /* Consumer | Business */, pub price_mode: PriceMode /* Inclusive | Exclusive */,
    pub currency: Currency, pub lines: Vec<TaxableLine> /* line_id, category, amount: Money, quantity: Option<Decimal> */ }
pub struct TaxQuote { pub lines: Vec<TaxedLine> /* net, taxes: Vec<TaxComponent>, gross */, pub totals: Vec<TaxTotal>,
    pub invoice_type: InvoiceType, pub snapshot: serde_json::Value /* rate_id, rate_percent, pack_version, rounding */ }
```

**`autoxide_ledger::api`**:

```rust
#[async_trait] pub trait Ledger: Send + Sync {
    /// Ghi một bút toán; `reference` bắt buộc (D-184). Cùng `reference` + cùng nội dung → Replayed.
    async fn post(&self, tenant: &TenantContext, actor: Actor, entry: NewEntry) -> Result<Submitted<Entry>, Error>;
    async fn reverse(&self, tenant: &TenantContext, actor: Actor, entry: Uuid, reversal: NewReversal) -> Result<Submitted<Entry>, Error>;
    async fn ensure_account(&self, tenant: &TenantContext, key: AccountKey) -> Result<Uuid, Error>;
    async fn balance(&self, tenant: &TenantContext, key: AccountKey, as_of: Option<NaiveDate>) -> Result<Money, Error>;
    /// Sinh bút toán theo chế độ dòng tiền của nút (tự thu / thu hộ qua đối tác / chia tại cổng).
    async fn settle(&self, tenant: &TenantContext, actor: Actor, event: FlowEvent) -> Result<Submitted<Vec<Entry>>, Error>;
    async fn create_payment(&self, tenant: &TenantContext, actor: Actor, payment: NewPayment) -> Result<Submitted<Payment>, Error>;
    async fn refund(&self, tenant: &TenantContext, actor: Actor, refund: NewRefund) -> Result<Submitted<Refund>, Error>;
    async fn create_payout(&self, tenant: &TenantContext, actor: Actor, payout: NewPayout) -> Result<Submitted<Payout>, Error>;
    async fn journal(&self, tenant: &TenantContext, query: JournalQuery) -> Result<Page<JournalLine>, Error>; // cho xuất kế toán
}
pub struct AccountKey { pub book: OrgId, pub purpose: Purpose, pub counterparty: Option<Counterparty>, pub currency: Currency }
pub struct Counterparty { pub kind: String /* "org" | "customer" | "carrier" | "partner" */, pub id: String }
```

`Purpose` là chuỗi ổn định (hợp đồng với module khác). Bộ cơ bản trong `api`: `cash.bank`, `cash.on_hand`,
`clearing.gateway` (phải thu đối tác thanh toán), `clearing.carrier_cod`, `receivable.customer`,
`payable.seller_held`, `payable.seller_available`, `payable.supplier`, `revenue.sales`,
`revenue.platform_fee`, `revenue.shipping`, `expense.gateway_fee`, `tax.vat_output`, `tax.vat_input`,
`tax.excise_payable`, `tax.withheld_vat`, `tax.withheld_pit`, `liability.customer_credit`,
`equity.opening`, `fx.gain_loss`, `rounding`. Module khác thêm mục đích riêng có tiền tố
`<module>.` (vd `customers.points_liability`). Ánh xạ mục đích → số hiệu tài khoản theo chế độ kế toán
Việt Nam là dữ liệu (`modules/ledger/data/vn-accounts.json`, vd `revenue.sales` → 511, `tax.vat_output`
→ 33311, `tax.withheld_pit` → 3335 — **cần đối chiếu TT 99/2025/TT-BTC và TT 133/2016/TT-BTC**), dùng
khi xuất kế toán; khách sửa được ánh xạ của sổ mình.

**`autoxide_invoices::api`**:

```rust
#[async_trait] pub trait Invoices: Send + Sync {
    async fn create_document(&self, tenant: &TenantContext, actor: Actor, draft: NewDocument) -> Result<Submitted<Document>, Error>;
    async fn issue(&self, tenant: &TenantContext, actor: Actor, document: Uuid) -> Result<Document, Error>; // gọi lại → cùng số
    async fn document(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<Document>, Error>;
    async fn exchange_rate(&self, tenant: &TenantContext, base: Currency, quote: Currency, on: NaiveDate) -> Result<Option<ExchangeRate>, Error>;
    async fn render(&self, tenant: &TenantContext, document: Uuid, format: RenderFormat, locale: &str) -> Result<Rendered, Error>;
    /// Gói lưu trữ kế toán của khách trước khi xoá khách (bước 35 gọi). Trả id bản lưu ở mặt điều khiển.
    async fn archive(&self, tenant: &TenantContext, actor: Actor) -> Result<Uuid, Error>;
}
```

`NewDocument.kind`: `vat_invoice` (hoá đơn GTGT), `sales_invoice` (hoá đơn bán hàng), `cash_register_invoice`
(từ máy tính tiền), `retail_receipt` (hoá đơn bán lẻ / phiếu tính tiền, không phải hoá đơn điện tử),
`delivery_note`, `goods_receipt`, `cash_receipt` (phiếu thu), `cash_payment` (phiếu chi),
`withholding_certificate` (chứng từ khấu trừ thuế TNCN — module 49 dùng), `adjustment`, `replacement`.

### Bảng dữ liệu

Quy ước chung: ID UUIDv7 sinh trong ứng dụng; thời gian UTC (`timestamp_with_time_zone`); ngày nghiệp vụ
`date` (Postgres `DATE`, SQLite chữ `YYYY-MM-DD` — sea-orm xử lý); tiền là cặp `<tên>_minor BIGINT` +
`<tên>_currency` (3 ký tự); số thập phân (thuế suất, tỉ giá, số lượng) là **chữ chuẩn hoá** qua
`decimal::to_column`; JSON lưu `TEXT`. Mọi bảng cấp khách có `tenant_id`, mọi unique / index bắt đầu bằng
`tenant_id`; bảng theo nút có `org_id`. Migration có `down`. Không foreign key sang bảng module khác.

**`tax`** — mặt điều khiển (dữ liệu luật dùng chung mọi khách, như `address`; đọc qua database của mặt
điều khiển, cache trong bộ nhớ theo `pack_version`):

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `tax_packs` | `id`, `code` ("vn"), `version` (i64 tăng dần), `source_note`, `imported_at`, `imported_by`, `content_sha256` | unique (`code`, `version`) |
| `tax_jurisdictions` | `code` ("VN"), `name`, `default_currency`, `pack_code` | PK `code` |
| `tax_categories` | `id`, `jurisdiction`, `code` (vd `vat.standard_reducible`, `vat.standard`, `vat.reduced_5`, `vat.zero`, `vat.exempt`, `excise.beer`), `name_en`, `name_vi`, `tax_kind` (`vat`/`excise`/`environment`) | unique (`jurisdiction`, `code`) |
| `tax_rates` | `id`, `jurisdiction`, `category_code`, `tax_kind`, `basis` (`percent`/`per_unit`), `rate_percent` (chữ, vd "8"), `per_unit_minor`/`per_unit_currency` (null được), `stage` (thứ tự cộng dồn: TTĐB 1 → GTGT 2), `effective_from` (gồm), `effective_to` (không gồm, null = chưa hết), `legal_basis`, `pack_version` | index (`jurisdiction`, `category_code`, `effective_from`) |
| `tax_withholding_rates` | `id`, `jurisdiction`, `payer_kind` (`marketplace`/`affiliate`), `payee_regime`, `business_line` (`goods`/`services`/`production_transport`/`other`), `tax_kind` (`vat`/`pit`), `rate_percent`, `threshold_minor`/`_currency` (null được), `effective_from`, `effective_to`, `legal_basis`, `pack_version` | index (`jurisdiction`, `payer_kind`, `effective_from`) |
| `tax_rules` | `id`, `jurisdiction`, `code` (vd `einvoice.cash_register_required`), `value` (JSON: ngưỡng doanh thu…), `effective_from`, `effective_to`, `legal_basis` | index (`jurisdiction`, `code`, `effective_from`) |

**`tax`** — cấp khách: `tax_profiles` (`id`, `tenant_id`, `org_id`, `tax_code` (MST, kiểm 10 hoặc 10-3
số), `regime` (`vat_credit` khấu trừ / `vat_direct` trực tiếp trên doanh thu / `household` hộ kinh doanh /
`exempt`), `business_lines` (JSON), `annual_revenue_minor`/`_currency` (ước tính, cho ngưỡng),
`legal_name`, `address` (JSON `Address`), `updated_at`) — unique (`tenant_id`, `org_id`); nút không có
hồ sơ thì dùng hồ sơ của tổ tiên gần nhất (khuôn kế thừa của bước 34).

**`ledger`** — cấp khách:

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `ledger_books` | `id`, `tenant_id`, `org_id` (đơn vị kế toán: gốc khách, công ty con, người bán tự hạch toán), `base_currency` (tiền hạch toán, mặc định VND), `fiscal_year_start_month`, `created_at` | unique (`tenant_id`, `org_id`) |
| `ledger_accounts` | `id`, `tenant_id`, `book_id`, `purpose`, `counterparty_kind`, `counterparty_id` (null được), `currency`, `normal_side` (`debit`/`credit`), `floor_minor` (null = không giới hạn), `external_code` (null = theo ánh xạ), `name`, `status` (`active`/`closed`), `created_at`, `updated_at` | unique (`tenant_id`, `book_id`, `purpose`, `counterparty_kind`, `counterparty_id`, `currency`) — cột null: dùng chuỗi rỗng thay null để unique chạy giống nhau trên hai database |
| `ledger_account_balances` | `account_id` (PK), `tenant_id`, `balance_minor` (theo bên thường của tài khoản), `currency`, `updated_at` | — ; đổi bằng `rows::adjust` với `Bounds { floor: floor_minor, … }` |
| `ledger_entries` | `id`, `tenant_id`, `book_id`, `period_id`, `effective_date`, `posted_at`, `description` (chữ cố định của module, không dữ liệu cá nhân), `source_module`, `source_kind`, `source_id`, `reference`, `reference_fingerprint`, `reverses_entry_id` (null được), `actor_user_id`, `actor_kind` | unique (`tenant_id`, `source_module`, `reference`); unique (`tenant_id`, `reverses_entry_id`) — chặn đảo hai lần; index (`tenant_id`, `book_id`, `effective_date`, `id`) |
| `ledger_lines` | `id`, `tenant_id`, `entry_id` (FK trong module), `line_no`, `account_id`, `side`, `amount_minor` (> 0), `amount_currency`, `base_minor`, `base_currency`, `exchange_rate` (chữ, null khi cùng tiền), `effective_date` (chép từ bút toán, để index) | index (`tenant_id`, `account_id`, `effective_date`, `id`) |
| `ledger_periods` | `id`, `tenant_id`, `book_id`, `starts_on`, `ends_on`, `status` (`open`/`closed`), `closed_at`, `closed_by` | unique (`tenant_id`, `book_id`, `starts_on`) |
| `ledger_period_balances` | `tenant_id`, `period_id`, `account_id`, `closing_minor`, `currency` | PK (`period_id`, `account_id`) — ghi lúc khoá kỳ, cho số dư đầu kỳ và xuất |
| `ledger_chain_heads` | `tenant_id`, `book_id` (PK), `last_sequence`, `last_hash` | — |
| `ledger_entry_seals` | `tenant_id`, `book_id`, `sequence`, `entry_id` (unique), `entry_hash`, `chain_hash`, `sealed_at` | unique (`tenant_id`, `book_id`, `sequence`) |
| `ledger_flow_settings` | `id`, `tenant_id`, `org_id`, `mode` (`self_collect`/`partner_collect`/`split_at_gateway`), `payment_provider`, `payout_provider`, `hold_days`, `updated_at` | unique (`tenant_id`, `org_id`); kế thừa theo cây |
| `ledger_payments` | `id`, `tenant_id`, `org_id` (bên nhận), `book_id`, `mode`, `method` (`card`/`wallet`/`vietqr`/…), `provider`, `provider_ref`, `amount_minor`/`_currency`, `refunded_minor`, `status` (`pending`/`succeeded`/`failed`/`expired`/`needs_review`), `transfer_code` (VietQR), `bank_account_id`, `qr_payload`, `checkout_url`, `expires_at`, `source_module`, `source_id`, `reference`, `reference_fingerprint`, `created_at`, `updated_at` | unique (`tenant_id`, `source_module`, `reference`); unique (`tenant_id`, `provider`, `provider_ref`); unique (`tenant_id`, `transfer_code`) |
| `ledger_refunds` | `id`, `tenant_id`, `payment_id`, `amount_*`, `status`, `provider_ref`, `reference`, `reference_fingerprint`, … | unique (`tenant_id`, `payment_id`, `reference`) |
| `ledger_payouts` | `id`, `tenant_id`, `book_id`, `payee_org_id`, `amount_*`, `provider`, `provider_ref`, `status` (`pending`/`sent`/`succeeded`/`failed`), `reference`, `reference_fingerprint`, … | unique (`tenant_id`, `reference`) |
| `ledger_holds` | `id`, `tenant_id`, `book_id`, `counterparty_id`, `amount_*`, `release_at`, `status` (`held`/`released`/`cancelled`), `source_module`, `source_id`, `release_entry_id` | index (`tenant_id`, `status`, `release_at`) |
| `ledger_gateway_events` | `id`, `tenant_id`, `provider`, `provider_event_id`, `kind` (`payment`/`payout`), `received_at`, `body` (nguyên văn), `status` (`pending`/`processed`/`ignored`/`failed`), `processed_at`, `error` | unique (`tenant_id`, `provider`, `provider_event_id`) |
| `ledger_bank_accounts` | `id`, `tenant_id`, `org_id`, `bank_bin`, `account_number`, `account_name`, `feed_provider`, `feed_cursor`, `status` | unique (`tenant_id`, `bank_bin`, `account_number`) |
| `ledger_bank_transactions` | `id`, `tenant_id`, `bank_account_id`, `provider`, `provider_txn_id`, `direction`, `amount_*`, `memo`, `booked_at`, `match_status` (`matched`/`unmatched`/`exception`), `payment_id` | unique (`tenant_id`, `provider`, `provider_txn_id`) |
| `ledger_statement_lines` | sao kê cổng thanh toán: `id`, `tenant_id`, `provider`, `provider_line_id`, `provider_ref`, `kind`, `amount_*`, `fee_*`, `booked_on`, `match_status` | unique (`tenant_id`, `provider`, `provider_line_id`) |
| `ledger_exceptions` | hàng chờ xử lý tay: `id`, `tenant_id`, `kind` (`amount_mismatch`/`unmatched_credit`/`duplicate_payment`/`missing_in_statement`/`unknown_in_statement`/`late_payment`), `subject` (`ledger.payment:<id>`…), `details` (JSON), `status` (`open`/`resolved`/`ignored`), `resolution`, `resolved_by`, `created_at`, `updated_at` | index (`tenant_id`, `status`, `created_at`) |

Kèm `event::outbox_migration("ledger")`, `job::jobs_migration("ledger")`.

**`invoices`** — cấp khách:

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `invoices_series` | `id`, `tenant_id`, `org_id`, `kind`, `symbol` (ký hiệu, vd `1C26TAA`), `year`, `einvoice_provider` (null = chứng từ nội bộ), `status`, `created_at`, `updated_at` | unique (`tenant_id`, `org_id`, `kind`, `symbol`, `year`) |
| `invoices_documents` | `id`, `tenant_id`, `org_id` (bên lập), `on_behalf_of_org_id` (uỷ nhiệm), `kind`, `series_id`, `number` (null khi nháp), `status` (`draft`/`issued`/`submitting`/`accepted`/`rejected`/`cancelled`/`replaced`/`adjusted`), `issue_date`, `price_mode`, `subtotal_*`, `tax_*`, `total_*`, `base_total_minor`/`_currency`, `exchange_rate`, `exchange_rate_id`, `buyer` (JSON ảnh chụp: tên, MST, `Address`, email, loại), `seller` (JSON ảnh chụp), `tax_snapshot` (JSON), `related_document_id`, `provider_ref`, `authority_code` (mã CQT), `lookup_code`, `xml_key`/`pdf_key` (khoá file trong `Storage`), `source_module`, `source_id`, `reference`, `reference_fingerprint`, `retention_until` (date), `issued_by`, `created_at`, `updated_at` | unique (`tenant_id`, `series_id`, `number`); unique (`tenant_id`, `source_module`, `reference`); index (`tenant_id`, `org_id`, `issue_date`, `id`) |
| `invoices_lines` | `id`, `tenant_id`, `document_id`, `line_no`, `kind` (`goods`/`service`/`discount`/`note`), `item_ref`, `description`, `unit`, `quantity` (chữ thập phân), `unit_price_*`, `discount_*`, `net_*`, `tax_category`, `rate_percent`, `tax_*`, `total_*` | index (`tenant_id`, `document_id`, `line_no`) |
| `invoices_mandates` | uỷ nhiệm lập hoá đơn: `id`, `tenant_id`, `principal_org_id` (bên uỷ nhiệm, người bán), `agent_org_id` (bên nhận, sàn), `kinds` (JSON), `valid_from`, `valid_to`, `document_file_id`, `status` (`active`/`revoked`), `created_at` | index (`tenant_id`, `principal_org_id`, `status`) |
| `invoices_exchange_rates` | `id`, `tenant_id`, `base`, `quote`, `rate` (chữ), `effective_date`, `source` (`manual`/`provider:<tên>`), `created_by`, `created_at` | unique (`tenant_id`, `base`, `quote`, `effective_date`, `source`) |
| `invoices_templates` | `id`, `tenant_id`, `org_id` (null = cả khách), `kind`, `locale`, `version`, `body`, `status` (`draft`/`active`), `updated_at` | unique (`tenant_id`, `org_id`, `kind`, `locale`, `version`) — cột null dùng chuỗi rỗng / UUID nil |
| `invoices_exports` | `id`, `tenant_id`, `org_id`, `profile`, `format` (`csv`/`xlsx`), `period_from`, `period_to`, `status`, `file_key`, `error`, `created_by`, `created_at` | index (`tenant_id`, `created_at`) |
| `invoices_archives` | **mặt điều khiển** (khách đã xoá vẫn còn): `id`, `tenant_id`, `created_at`, `retain_until`, `storage_prefix`, `manifest_sha256` | index (`tenant_id`) |

Kèm outbox, jobs migration. Mẫu in mặc định nằm trong repo: `modules/invoices/templates/<kind>.<locale>.html`.

### Sổ cái: luật ghi

1. **Một bút toán = một sổ, một loại tiền gốc**, ≥ 2 dòng, mỗi dòng `amount > 0`; tổng nợ = tổng có
   (lỗi `ledger.unbalanced`) ở cả tiền gốc và tiền hạch toán (`base_*`; ngoại tệ quy đổi bằng tỉ giá
   lấy từ `dyn Invoices::exchange_rate` TRƯỚC transaction, phần lệch làm tròn chia bằng `allocate`).
   Đổi tiền giữa hai loại = hai bút toán qua tài khoản `fx.gain_loss` / tài khoản trung gian.
2. **Chỉ thêm.** Không `UPDATE`/`DELETE` trên `ledger_entries`/`ledger_lines` (xtask hay test kiểm không
   có entity `ActiveModel` update cho hai bảng). Sửa sai = `POST /entries/{id}/reverse`: bút toán mới đảo
   nợ/có, `reverses_entry_id` trỏ bút toán gốc, ngày hiệu lực ở **kỳ đang mở** (mặc định hôm nay theo
   múi giờ của nút); đảo một bút toán đảo là được (ghi lại), đảo hai lần cùng bút toán thì unique chặn
   → `ledger.already_reversed`.
3. **Thứ tự trong transaction** (`write_with_retry` + `reference::submit_once`): tìm `reference` →
   `ensure_period` (INSERT … ON CONFLICT DO NOTHING) → `rows::lock_one_shared` dòng kỳ, `open` mới đi
   tiếp (không thì `ledger.period_closed`) → `rows::adjust` số dư từng tài khoản **theo `account_id`
   tăng dần** (`0` dòng đổi → `ledger.insufficient_balance` nếu tài khoản có sàn) → chèn bút toán, dòng →
   `Outbox::record(ledger.entry_posted)` → commit.
4. **Khoá sổ**: `POST /periods/{id}/close` — kỳ trước phải đã khoá (`ledger.period_order`); `rows::lock_one`
   dòng kỳ (chờ mọi bút toán đang giữ khoá chia sẻ), tính số dư cuối kỳ ghi `ledger_period_balances`,
   đặt `closed`. Không có mở lại kỳ.
5. **Số dư suy từ bút toán**: `ledger_account_balances` chỉ là bản cộng dồn để kiểm sàn nhanh;
   `autoxide doctor` có bước `ledger.balances` tính lại `SUM` từ dòng và báo tài khoản lệch. Số dư tại
   ngày = số dư cuối kỳ khoá gần nhất + tổng dòng sau đó (`CAST(SUM(…) AS BIGINT)` — Postgres trả
   `numeric` cho `SUM(bigint)`).
6. **Chuỗi băm** (mượn D-183, `modules/audit/src/chain.rs`): job `ledger.seal_entries` (lịch 10 giây) lấy
   bút toán chưa niêm phong của từng sổ theo `id`, khoá `ledger_chain_heads`, gán `sequence`, ghi
   `entry_hash` (SHA-256 của id, sổ, ngày, posted_at, reference, reverses, các dòng — **không gồm
   `tenant_id`**, để tách / gộp khách ở bước 38 không làm gãy chuỗi), `chain_hash`, tiền tố `ledger-v1`.
   Đường request không bị nối đuôi. Doctor `ledger.chain`: báo bút toán đầu tiên bị sửa / xoá, và bút toán
   chưa niêm phong quá 10 phút.

### Ba chế độ dòng tiền — bút toán sinh ra (`Ledger::settle`)

Ví dụ: đơn 1.000.000 đ (gồm GTGT), phí sàn 5% = 50.000 đ (gồm GTGT 10% của phí), người bán là hộ kinh
doanh bị khấu trừ 1% GTGT + 0,5% TNCN (tỉ lệ lấy từ `TaxEngine::withholding`, ví dụ minh hoạ — cần đối
chiếu), phí cổng 1.100 đ. `FlowEvent` = `PaymentCaptured` · `FeeCharged` · `TaxWithheld` · `HoldReleased`
· `PayoutSent` · `Refunded` · `GatewayFeeCharged`; mỗi sự kiện mang `reference` riêng.

| Sự kiện | Tự thu (sổ của người bán) | Thu hộ qua đối tác (sổ của sàn) | Chia tại cổng (sổ của sàn) |
|---|---|---|---|
| Tiền vào | Nợ `clearing.gateway` 1.000.000 / Có `revenue.sales` 909.091, `tax.vat_output` 90.909 | Nợ `clearing.gateway` 1.000.000 / Có `payable.seller_held[S]` 1.000.000 | Nợ `clearing.gateway` 65.000 (phần cổng chia cho sàn) / Có `payable.seller_held[S]` 65.000 |
| Phí sàn | không có (sàn thu phí bằng hoá đơn / thuê bao) | Nợ `payable.seller_held[S]` 50.000 / Có `revenue.platform_fee` 45.455, `tax.vat_output` 4.545 | như thu hộ |
| Khấu trừ thay (NĐ 117/2025) | không | Nợ `payable.seller_held[S]` 15.000 / Có `tax.withheld_vat` 10.000, `tax.withheld_pit` 5.000 | như thu hộ |
| Hết hạn đổi trả | — | Nợ `payable.seller_held[S]` 935.000 / Có `payable.seller_available[S]` 935.000 (job `ledger.release_holds`) | không giữ (người bán đã nhận ở cổng) |
| Chi trả | — | Nợ `payable.seller_available[S]` / Có `clearing.gateway` (đối tác chi từ tài khoản thu hộ) | — |
| Phí cổng | Nợ `expense.gateway_fee` / Có `clearing.gateway` | như tự thu, trên sổ của sàn | như tự thu |

Kiểm sau mỗi kịch bản: `clearing.gateway` khớp số đối tác báo trong sao kê; ở chế độ thu hộ, sau chi trả
`payable.seller_*[S]` = 0. Chế độ không hợp với cổng (cổng không có `split`) → `ledger.flow_mode_unsupported`.
`TaxWithheld` ở chế độ tự thu → `ledger.flow_mode_unsupported` (nền tảng không có chức năng thanh toán
thì không khấu trừ — D-187).

### Thanh toán, webhook, đối soát, VietQR

- **Tạo thanh toán** (`create_payment`): transaction ngắn ghi `ledger_payments` (`pending`, có
  `reference`) → commit → **ngoài transaction** gọi `PaymentGateway::create_payment` (mã của merchant =
  `payment_id`, nên gọi lại không tạo hai giao dịch ở đối tác) → transaction ngắn ghi `provider_ref`,
  `checkout_url`. Lỗi mạng: giữ `pending`, client gọi lại cùng `reference` thì thử gọi đối tác lại.
- **Webhook** `POST /api/v1/ledger/webhooks/{provider}` (công khai; khách suy từ tên miền — bước 33; giới
  hạn body qua `Module::body_limits`): đọc thân thô (`Bytes`) → `verify_webhook` (sai → 401
  `payment.signature_invalid`, chỉ log + số đo `ledger_webhook_rejected_total{provider}`, không lưu) →
  INSERT `ledger_gateway_events` ON CONFLICT DO NOTHING + enqueue job `ledger.process_gateway_event`
  (`unique_key` = id event) trong một transaction → trả 200. Job: khoá dòng thanh toán, chuyển trạng thái,
  `settle(PaymentCaptured)` với `reference = "gw:{provider}:{provider_event_id}"`, ghi event
  `ledger.payment_succeeded` cho module nguồn (orders, plans) nhận. Số tiền khác → `needs_review` +
  `ledger_exceptions(amount_mismatch)`.
- **Đối soát** (lịch `ledger.reconcile` mỗi ngày, task `ledger.reconcile date=…`): kéo `statement` của
  đối tác cho ngày trước → `ledger_statement_lines` → khớp theo `provider_ref` + số tiền; thiếu ở sổ /
  thiếu ở sao kê / lệch → `ledger_exceptions`. Phí trong sao kê → `GatewayFeeCharged`.
- **VietQR** (`modules/ledger/src/vietqr.rs`; riêng Việt Nam nên ở module, không ở kernel): chuỗi theo
  EMVCo MPM của NAPAS — tag `00`="01", `01`="12" (động), `38` (GUID `A000000727`, BIN ngân hàng + số tài
  khoản, mã dịch vụ `QRIBFTTA`), `53`="704", `54`=số tiền, `58`="VN", `62` mục `08` = nội dung chuyển
  khoản, `63` = CRC-16/CCITT-FALSE. **Tra lại đặc tả NAPAS và TCCS 03:2018/NHNN trước khi viết**; test
  đối chiếu với mã do app ngân hàng sinh. Nội dung chuyển khoản = `transfer_code` ngắn, chỉ chữ hoa + số
  (vd `AX` + 8 ký tự base32 từ id thanh toán) vì ngân hàng bỏ dấu, cắt ngắn, chèn khoảng trắng. Danh sách
  BIN ngân hàng là dữ liệu (`modules/ledger/data/vn-banks.json`, ghi nguồn và ngày).
- **Khớp biến động số dư**: webhook `POST /api/v1/ledger/bank-feeds/{provider}` (kiểu đẩy) hoặc lịch
  `ledger.poll_bank_feed` (kiểu kéo, theo `feed_cursor`) → `ledger_bank_transactions` (unique theo
  `provider_txn_id`) → job `ledger.match_bank_transaction`: tách mã bằng regex không phân biệt hoa thường,
  bỏ khoảng trắng; đúng mã + đúng số tiền + thanh toán `pending` → `succeeded` + `settle`; sai số tiền →
  `amount_mismatch`; không có mã → `unmatched_credit`; thanh toán đã xong mà có tiền lần hai →
  `duplicate_payment`; thanh toán đã `expired` → `late_payment`. Tiền vào thẳng tài khoản của người bán /
  khách (chế độ tự thu) — nền tảng không giữ.
- **Xử lý tay**: `POST /exceptions/{id}/resolve` {`action`: `match` (`payment_id`) | `ignore` |
  `refunded_outside`, `note`} — `match` chạy cùng đường `settle`.

### Engine thuế

- Tìm thuế suất: `effective_from <= tax_point < effective_to` (nửa mở). Không có → `tax.rate_missing`
  (không đoán). Nhiều tầng: tính tầng 1 (TTĐB) trên giá chưa thuế, tầng 2 (GTGT) trên giá + TTĐB; giá
  gồm thuế thì tách ngược qua tích `(1+r1)(1+r2)`, phần dư dồn vào GTGT.
- Theo `TaxProfile.regime`: `vat_credit` → hoá đơn GTGT, thuế theo nhóm hàng; `vat_direct`/`household` →
  hoá đơn bán hàng, không tách GTGT trên dòng (thuế tính trên doanh thu ở kê khai, ngoài phạm vi);
  `exempt` → không thuế.
- `invoice_requirements`: đọc `tax_rules` (vd `einvoice.cash_register_required` với ngưỡng doanh thu
  1 tỷ đ/năm từ 2025-06-01 — **cần đối chiếu NĐ 70/2025 và lộ trình hạ ngưỡng**) → `cash_register_required`.
- **Gói "vn"** `modules/tax/packs/vn.json` (có `version`, `sources`, `checked_on`): GTGT 0%, 5%, 10%; nhóm
  `vat.standard_reducible` 8% từ 2025-07-01 tới 2027-01-01 (không gồm) rồi 10%; `vat.exempt`; một vài
  nhóm TTĐB mẫu; tỉ lệ khấu trừ thay của sàn theo NĐ 117/2025 (vd hàng hoá 1% GTGT + 0,5% TNCN, dịch vụ
  5% + 2%, sản xuất – vận tải 3% + 1,5%) — **mọi tỉ lệ ghi `legal_basis` và "cần đối chiếu"**. Nhập bằng
  `autoxide task tax.import_pack file=… ` hoặc `POST /api/v1/tax/packs`; chỉ nhận `version` lớn hơn bản đang
  có (`tax.pack_outdated`); nhập trong một transaction, `content_sha256` để biết đã nhập chưa.
- Doctor `tax.expiring_rates`: thuế suất hết hạn trong `AUTOXIDE_TAX_EXPIRY_WARNING_DAYS` ngày mà không
  có bản kế tiếp cho cùng nhóm → cảnh báo kèm gợi ý nhập gói mới. (Hôm nay 13/11/2026: nhóm 8% hết hạn
  01/01/2027 — gói phải có sẵn bản 10% từ 2027-01-01, nếu Quốc hội gia hạn thì sửa gói.)

### Chứng từ và hoá đơn điện tử

- Nháp (`draft`) không có số; `POST /documents/{id}/issue`: trong một transaction — khoá dòng chứng từ
  (`rows::lock_one`), đã `issued` thì trả nguyên (gọi lại không cấp số mới), kiểm ký hiệu `active`, kiểm
  uỷ nhiệm khi `on_behalf_of_org_id` có giá trị (`invoices.mandate_missing`), `numbering::next(scope =
  "invoices:{org}:{kind}:{symbol}:{year}")`, đặt `retention_until` = cuối năm tài chính + 10 năm, event
  `invoices.document_issued`, enqueue `invoices.submit_einvoice` nếu ký hiệu có nhà cung cấp → commit.
  Gọi nhà cung cấp ở job (ngoài transaction). Nhà cung cấp tự cấp số (`assigns_numbers`) thì số nội bộ
  vẫn cấp cho chứng từ, số hoá đơn của nhà cung cấp lưu khi `Accepted` — tài liệu ghi rõ.
- `rejected` (cơ quan thuế / nhà cung cấp từ chối): sửa trường bị báo rồi gửi lại **cùng số**; đã
  `accepted` thì chỉ sửa bằng chứng từ `adjustment` / `replacement` liên kết `related_document_id` (theo
  NĐ 123/2020 Điều 19 sửa bởi NĐ 70/2025 — cần đối chiếu) hay `cancel` theo thông báo sai sót.
- Hoá đơn từ máy tính tiền: kind `cash_register_invoice`, ký hiệu có nhà cung cấp hỗ trợ
  `CashRegister`; không ký số, mã do phần mềm sinh, nhà cung cấp chuyển cơ quan thuế trong ngày.
- Chứng từ ngoại tệ: lấy tỉ giá ngày `issue_date` (nguồn ưu tiên: `manual` rồi nhà cung cấp; không có →
  `invoices.rate_missing`), lưu `exchange_rate` + `exchange_rate_id` + `base_total` — không tính lại.
- Mọi dòng lưu ảnh chụp: mô tả, đơn vị, giá, `rate_percent`, nhóm thuế, `tax_snapshot` của `TaxQuote`.
- **Lưu trữ**: chứng từ `issued` trở đi không xoá được (`invoices.retention_hold`); khi nhận
  `USER_DELETED`, `invoices` **giữ** ảnh chụp người mua (nghĩa vụ pháp lý — ghi lý do trong `PersonalData`
  của module, xuất được), job `invoices.purge_expired` (hằng ngày) ẩn danh hoá ảnh chụp người mua cá nhân
  khi quá `retention_until`. `archive`: ghi vào `Storage` dưới tiền tố của khách các file `ledger/journal.csv`,
  `ledger/accounts.csv`, `ledger/seals.csv`, `documents/*.json`, XML/PDF hoá đơn, `manifest.json` (SHA-256
  từng file) và một dòng `invoices_archives` ở mặt điều khiển; bước 35 gọi trước khi xoá dữ liệu khách.
- **Mẫu in**: engine mẫu có sandbox (không đọc file, không gọi hàm hệ thống, giới hạn bước chạy — khuyến
  nghị `minijinja`, tự động escape HTML) vì mẫu do khách sửa. Dữ liệu đưa vào mẫu là view model cố định
  (`document`, `lines`, `seller`, `buyer`, `totals`, `labels` theo i18n). HTML trả thẳng; PDF qua thư viện
  agent chọn (xem "Agent tự quyết"), cache file PDF trong `Storage` theo (`document_id`, `template version`).
- **Xuất kế toán**: `POST /exports` → job `invoices.build_export` đọc `dyn Ledger::journal` (phân trang) và
  chứng từ của kỳ → CSV (UTF-8 có BOM, RFC 4180, ô bắt đầu bằng `= + - @` thêm `'` chống formula injection)
  hoặc XLSX → `Storage` → `GET /exports/{id}/download`. Hồ sơ xuất là dữ liệu: `generic_journal` (sổ nhật
  ký chung: ngày, số bút toán, số / ngày chứng từ, diễn giải, TK nợ, TK có theo `external_code`, số tiền,
  tiền tệ, quy đổi VND, đối tượng), `generic_documents`; hồ sơ cho phần mềm cụ thể (MISA, Fast…) chỉ thêm
  khi đối chiếu được mẫu nhập họ công bố.

### API REST

Mọi route dưới `/api/v1/<module>`, `TenantContext` + `Principal` (trừ webhook), envelope `Data`/`Page`/
`Problem`, lệnh ghi có `reference` (D-184) hoặc `Idempotency-Key`, tạo mới trả `Created`.

**`/api/v1/tax`**: `GET /jurisdictions` · `GET /categories?jurisdiction=` · `GET /rates?jurisdiction=&category=&on=`
(`tax.read`) · `POST /quote` (`tax.read`; chỉ tính) · `GET /profiles`, `GET /profiles/{org_id}`,
`PUT /profiles/{org_id}` (`tax.profiles_manage`, If-Match) · `GET /packs`, `POST /packs`
(`tax.packs_manage` — chỉ nhân sự nền tảng).

**`/api/v1/ledger`**: `GET|POST /books` · `GET /accounts?book_id=&purpose=&counterparty_kind=&counterparty_id=`,
`POST /accounts`, `GET|PATCH /accounts/{id}`, `GET /accounts/{id}/lines` (Page), `GET /accounts/{id}/balance?as_of=`
· `POST /entries` (`ledger.post`), `GET /entries`, `GET /entries/{id}`, `POST /entries/{id}/reverse`
(`ledger.reverse`) · `GET /periods?book_id=`, `POST /periods/{id}/close` (`ledger.periods_close`) ·
`GET /trial-balance?book_id=&from=&to=` · `GET|PUT /flow-settings/{org_id}` (`ledger.settings_manage`) ·
`POST /payments` (`ledger.payments_create`), `GET /payments`, `GET /payments/{id}`,
`POST /payments/{id}/refunds` (`ledger.refunds_create`) · `POST /payouts` (`ledger.payouts_create`),
`GET /payouts`, `GET /payouts/{id}` · `GET|POST /bank-accounts`, `PATCH /bank-accounts/{id}` ·
`GET /exceptions`, `POST /exceptions/{id}/resolve` (`ledger.reconcile`) · `POST /reconciliations`
(chạy ngay), `GET /reconciliations` · **công khai**: `POST /webhooks/{provider}`, `POST /bank-feeds/{provider}`.
Đọc dùng `ledger.read` (phạm vi theo nút: `unit` / `subtree` / `tenant` lọc theo `org_id` của sổ).

**`/api/v1/invoices`**: `GET|POST /documents`, `GET|PATCH|DELETE /documents/{id}` (PATCH/DELETE chỉ nháp) ·
`POST /documents/{id}/issue` (`invoices.issue`) · `POST /documents/{id}/cancel`, `/adjust`, `/replace`
(`invoices.correct`) · `GET /documents/{id}/render?format=html|pdf&locale=&template_id=` ·
`GET /documents/{id}/einvoice?format=xml|pdf` · `GET|POST /series`, `PATCH /series/{id}` · `GET|POST /mandates`,
`POST /mandates/{id}/revoke` · `GET|POST /templates`, `GET|PUT /templates/{id}`, `POST /templates/preview` ·
`GET /exchange-rates?base=&quote=&on=`, `POST /exchange-rates`, `POST /exchange-rates/fetch` ·
`GET /export-profiles`, `POST /exports`, `GET /exports/{id}`, `GET /exports/{id}/download`.

### Tool MCP

`tax.quote`, `tax.rates` · `ledger.balance`, `ledger.entries`, `ledger.trial_balance`, `ledger.payment`,
`ledger.exceptions`, `ledger.post_entry`, `ledger.resolve_exception` · `invoices.documents`,
`invoices.document`, `invoices.create_draft`, `invoices.issue`, `invoices.exchange_rate` — mỗi tool gọi
cùng service, cùng permission với route tương ứng (luật 8).

### Event

`tax.pack_imported`, `tax.profile_changed` · `ledger.entry_posted`, `ledger.entry_reversed`,
`ledger.period_closed`, `ledger.payment_created`, `ledger.payment_succeeded`, `ledger.payment_failed`,
`ledger.payment_refunded`, `ledger.payout_sent`, `ledger.payout_failed`, `ledger.hold_released`,
`ledger.exception_opened`, `ledger.exception_resolved` · `invoices.document_issued`,
`invoices.document_accepted`, `invoices.document_rejected`, `invoices.document_cancelled`,
`invoices.document_corrected`, `invoices.export_ready`. Payload chỉ ID, số tiền, trạng thái — không tên,
email, số điện thoại. Bên nhận chịu được nhận lại (D-054).

### Job / lịch

`ledger.process_gateway_event`, `ledger.process_payout_event`, `ledger.send_payout`, `ledger.match_bank_transaction`;
lịch `ledger.seal_entries` (10 s), `ledger.poll_bank_feed` (`AUTOXIDE_LEDGER_BANK_FEED_POLL_SECONDS`),
`ledger.release_holds` (5 phút), `ledger.expire_payments` (1 phút), `ledger.reconcile` (24 giờ) ·
`invoices.submit_einvoice`, `invoices.poll_einvoice_status` (lịch 5 phút cho chứng từ `submitting`),
`invoices.build_export`, `invoices.fetch_exchange_rates` (24 giờ), `invoices.purge_expired` (24 giờ).
Mọi bộ chạy chịu được chạy lại (unique theo `reference` / `unique_key`).

### Cấu hình (Setting)

| Biến | Mặc định | Nghĩa |
|---|---|---|
| `AUTOXIDE_TAX_EXPIRY_WARNING_DAYS` | 60 | doctor cảnh báo thuế suất sắp hết hạn |
| `AUTOXIDE_LEDGER_PAYMENT_EXPIRY_MINUTES` | 30 | hạn thanh toán `pending` (kể cả VietQR) |
| `AUTOXIDE_LEDGER_BANK_FEED_POLL_SECONDS` | 60 | chu kỳ kéo biến động số dư |
| `AUTOXIDE_LEDGER_WEBHOOK_MAX_BYTES` | 65536 | giới hạn thân webhook |
| `AUTOXIDE_INVOICES_RETENTION_YEARS` | 10 | không nhận giá trị < 10 (Luật Kế toán Điều 41) |

Cấu hình theo khách (chế độ dòng tiền, ngày tạm giữ, tài khoản ngân hàng, nhà cung cấp) là dữ liệu trong
bảng, không phải biến môi trường. Sinh lại `.env.example`.

### Permission

`tax.read`, `tax.profiles_manage`, `tax.packs_manage` · `ledger.read`, `ledger.post`, `ledger.reverse`,
`ledger.accounts_manage`, `ledger.periods_close`, `ledger.settings_manage`, `ledger.payments_create`,
`ledger.refunds_create`, `ledger.payouts_create`, `ledger.reconcile` · `invoices.read`, `invoices.write`,
`invoices.issue`, `invoices.correct`, `invoices.series_manage`, `invoices.mandates_manage`,
`invoices.templates_manage`, `invoices.rates_manage`, `invoices.export`.

### Mã lỗi (`locales/vi.json`)

| Mã | HTTP | Tiếng Việt |
|---|---|---|
| `decimal.invalid` (kernel) | 422 | Số thập phân không hợp lệ. Gửi dạng chuỗi như "12.5", không dùng dấu phẩy hay số JSON. |
| `tax.jurisdiction_unknown` | 422 | Không có vùng thuế `{jurisdiction}`. Xem danh sách ở `GET /api/v1/tax/jurisdictions`. |
| `tax.category_unknown` | 422 | Không có nhóm thuế `{category}` trong vùng `{jurisdiction}`. |
| `tax.rate_missing` | 422 | Chưa có thuế suất cho nhóm `{category}` vào ngày {date}. Hãy nhập gói dữ liệu thuế mới. |
| `tax.profile_not_found` | 404 | Nút này và các nút cấp trên chưa có hồ sơ thuế. Hãy tạo hồ sơ thuế trước. |
| `tax.pack_invalid` | 422 | Gói dữ liệu thuế không hợp lệ: {reason}. |
| `tax.pack_outdated` | 409 | Đã có gói `{code}` phiên bản {current}; chỉ nhận phiên bản lớn hơn. |
| `ledger.unbalanced` | 422 | Bút toán không cân: tổng nợ {debit} khác tổng có {credit}. |
| `ledger.currency_mixed` | 422 | Mọi dòng của một bút toán phải cùng một loại tiền. |
| `ledger.period_closed` | 409 | Kỳ kế toán chứa ngày {date} đã khoá. Ghi bút toán điều chỉnh vào kỳ đang mở. |
| `ledger.period_order` | 409 | Phải khoá kỳ trước ({previous}) trước khi khoá kỳ này. |
| `ledger.account_not_found` | 404 | Không tìm thấy tài khoản sổ cái này. |
| `ledger.account_closed` | 409 | Tài khoản đã đóng, không nhận bút toán mới. |
| `ledger.insufficient_balance` | 409 | Số dư tài khoản không đủ cho bút toán này. |
| `ledger.entry_not_found` | 404 | Không tìm thấy bút toán này. |
| `ledger.already_reversed` | 409 | Bút toán này đã được đảo. |
| `ledger.flow_mode_unsupported` | 422 | Chế độ dòng tiền `{mode}` không hỗ trợ việc này hoặc cổng `{provider}`. |
| `ledger.gateway_not_configured` | 422 | Khách chưa cấu hình cổng thanh toán `{provider}`. |
| `ledger.payment_not_found` | 404 | Không tìm thấy thanh toán này. |
| `ledger.payment_state` | 409 | Thanh toán đang ở trạng thái `{status}`, không làm được việc này. |
| `ledger.refund_exceeds_payment` | 422 | Số tiền hoàn vượt số còn lại của thanh toán ({remaining}). |
| `ledger.bank_unknown` | 422 | Không có ngân hàng mang mã BIN `{bin}`. |
| `ledger.exception_not_found` | 404 | Không tìm thấy mục chờ xử lý này. |
| `payment.signature_invalid` (kernel) | 401 | Chữ ký của thông báo từ đối tác không hợp lệ. |
| `payment.provider_unavailable` (kernel) | 503 | Đối tác thanh toán đang không trả lời. Hãy thử lại sau. |
| `payment.provider_rejected` (kernel) | 422 | Đối tác thanh toán từ chối: {reason}. |
| `payment.not_supported` (kernel) | 422 | Đối tác `{provider}` không hỗ trợ việc này. |
| `einvoice.provider_unavailable` (kernel) | 503 | Nhà cung cấp hoá đơn điện tử đang không trả lời. Hãy thử lại sau. |
| `einvoice.rejected` (kernel) | 422 | Hoá đơn bị từ chối: {reason}. Sửa rồi gửi lại cùng số. |
| `einvoice.not_supported` (kernel) | 422 | Nhà cung cấp `{provider}` không hỗ trợ loại hoá đơn này. |
| `invoices.document_not_found` | 404 | Không tìm thấy chứng từ này. |
| `invoices.not_draft` | 409 | Chỉ sửa hay xoá được chứng từ nháp. Chứng từ đã phát hành thì lập chứng từ điều chỉnh hoặc thay thế. |
| `invoices.retention_hold` | 409 | Chứng từ kế toán phải lưu tới {until}, không xoá được. |
| `invoices.series_not_found` | 404 | Không tìm thấy ký hiệu chứng từ này. |
| `invoices.series_inactive` | 409 | Ký hiệu `{symbol}` đã ngừng dùng. |
| `invoices.mandate_missing` | 403 | Chưa có uỷ nhiệm còn hiệu lực để lập hoá đơn thay người bán này. |
| `invoices.provider_not_configured` | 422 | Khách chưa cấu hình nhà cung cấp hoá đơn điện tử `{provider}`. |
| `invoices.buyer_tax_code_invalid` | 422 | Mã số thuế người mua phải có 10 hoặc 13 chữ số (dạng 10-3). |
| `invoices.rate_missing` | 422 | Chưa có tỉ giá {base}/{quote} cho ngày {date}. Nhập tỉ giá trước khi phát hành. |
| `invoices.template_invalid` | 422 | Mẫu in lỗi ở dòng {line}: {reason}. |
| `invoices.template_not_found` | 404 | Không tìm thấy mẫu in này. |
| `invoices.render_failed` | 500 | Không dựng được bản in. Hãy thử lại; nếu vẫn lỗi, báo mã request. |
| `invoices.export_not_ready` | 409 | Bản xuất chưa xong (trạng thái `{status}`). |

## Thứ tự làm

Mỗi dòng là một commit (hoặc vài commit nhỏ), `./gate.sh --quick` trước mỗi commit. Ánh xạ mục ROADMAP:

1. Kernel `decimal` + `Money::{to_decimal, from_decimal, apply_rate, convert}`, `DecimalString`; khai
   `rust_decimal` trong workspace, ghi `STACK.md`, quyết định mới → **mục 1 (rust_decimal)**.
2. Kernel `numbering` + migration `kernel_sequences` (mọi database dữ liệu) + `rows::lock_one_shared` +
   `StartContext::contribute/contributions` → nền cho **mục 6, 2**.
3. Module `tax`: bảng, nhập gói, gói `vn.json`, `TaxEngine`, API, MCP, doctor → **mục 5**.
4. Module `ledger` lõi: sổ, tài khoản, bút toán, đảo, số dư, kỳ, khoá sổ, chuỗi băm, doctor → **mục 2,
   một nửa mục 7**.
5. Kernel `payment` (trait) + testkit fake; `ledger` thanh toán, webhook, hoàn tiền, đối soát, hàng chờ
   sai lệch → **mục 3** (bước 35 bắt đầu dùng được từ đây).
6. `ledger` cài đặt dòng tiền, `settle`, tạm giữ, chi trả → **mục 4**.
7. `ledger` VietQR, tài khoản ngân hàng, `BankFeed`, khớp tự động → **mục 9**.
8. Kernel `einvoice`; module `invoices`: ký hiệu, chứng từ, phát hành, uỷ nhiệm, điều chỉnh / thay thế,
   máy tính tiền → **mục 6, 8**.
9. `invoices` tỉ giá + `ExchangeRateSource`; ngoại tệ trên chứng từ và bút toán → **mục 10**.
10. Lưu trữ 10 năm: `retention_until`, chặn xoá, `USER_DELETED`, `purge_expired`, `archive` → **nửa còn
    lại mục 7**.
11. Mẫu in HTML + PDF (chọn thư viện, ghi `STACK.md` + quyết định) → **mục 11**.
12. Xuất kế toán CSV/XLSX → **mục 12**.
13. Tài liệu cho người ngoài: `docs/MONEY.md` + `docs/MONEY.vi.md` (ba module, ba chế độ dòng tiền, cách
    viết bộ nối đối tác), cập nhật `docs/MODULE_GUIDE.md` (+ `.vi`) mục "Money and balances" (`decimal`,
    `numbering`, `Ledger::post` thay vì tự làm bảng số dư); sinh lại `docs/openapi.json`, `.env.example`;
    `CHANGELOG.md`; đánh dấu ROADMAP; gate đầy đủ; phát hành theo `docs/RELEASING.md`.

## Test bắt buộc

Mọi test qua `TestDatabase::run` / `TestApp::builder`, chạy trên **cả SQLite và Postgres** (gate đầy đủ),
không bỏ qua bên nào. Tên gợi ý:

- Kernel: `decimal_money_roundtrip_every_currency` (mọi mã trong `CURRENCIES`), `decimal_rounding_strategies`,
  `decimal_from_overflow_is_money_overflow`, `numbering_concurrent_next_is_gapless` (32 transaction đồng
  thời → đúng 1..32), `numbering_rollback_does_not_consume`, `numbering_scopes_and_tenants_independent`,
  `lock_one_shared_blocks_exclusive_lock`.
- Tax: `vn_vat_8_until_2026_then_10`, `inclusive_price_split_sums_exactly`, `document_rounding_allocates_to_lines`,
  `excise_then_vat_compound`, `withholding_marketplace_household`, `profile_inherits_from_ancestor`,
  `pack_import_rejects_older_version`, `doctor_warns_rate_without_successor`.
- Ledger: `post_rejects_unbalanced_and_mixed_currency`, `reference_replay_and_reuse`,
  `concurrent_postings_same_account_balance_exact` (40 request), `floor_never_breached_under_concurrency`,
  `opposite_transfers_no_deadlock`, `reverse_twice_concurrently_one_wins`, `close_period_races_posting`
  (không bút toán nào nằm trong kỳ đã khoá), `close_requires_previous_period`, `balance_as_of_uses_period_snapshot`,
  `doctor_balances_detects_drift`, `seal_chain_detects_edit_and_delete`, `chain_ignores_tenant_id`.
- Dòng tiền: `flow_self_collect_entries`, `flow_partner_collect_hold_release_payout`, `flow_split_at_gateway`
  (bảng số dư mong đợi như ví dụ; tổng mọi tài khoản theo bên = 0), `withholding_rejected_in_self_collect`.
- Thanh toán: `webhook_bad_signature_401_not_stored`, `webhook_duplicate_concurrent_processed_once`,
  `create_payment_gateway_down_then_retry_same_reference`, `reconcile_reports_missing_and_mismatch`,
  `refund_cannot_exceed_payment` (đồng thời).
- VietQR: `vietqr_payload_matches_napas_sample` (CRC đúng), `bank_txn_duplicate_once`, `match_exact`,
  `match_amount_mismatch_to_exception`, `match_late_and_duplicate_payment`, `resolve_exception_match_settles`.
- Invoices: `issue_concurrent_gapless_per_series`, `issue_twice_returns_same_number`,
  `draft_has_no_number_and_deletable`, `issued_cannot_be_deleted`, `rejected_resubmits_same_number`,
  `mandate_required_on_behalf`, `cash_register_kind_with_fake_provider`, `foreign_currency_snapshots_rate`,
  `user_deleted_keeps_invoice_snapshot`, `archive_manifest_hashes_match`, `template_sandbox_denies_io_and_limits_fuel`,
  `render_pdf_vietnamese_text`, `export_csv_escapes_formulas_and_matches_trial_balance`.
- Rò rỉ: `ledger_tenant_leak` / `invoices_tenant_leak` (khách A ghi, người khách B đọc mọi endpoint và tool
  MCP: không thấy), `ledger_sibling_org_leak` (quyền `subtree` ở nhánh X không thấy sổ nhánh anh em Y).
- OpenAPI: mỗi endpoint mới có summary + description từng response (test có sẵn); webhook có tài liệu nói
  rõ không cần xác thực nhưng phải có chữ ký.

## Tiêu chí xong

- [ ] Mọi mục `- [ ]` của bước 36 trong ROADMAP đánh `[x]`, mỗi mục khớp commit ở "Thứ tự làm".
- [ ] `./gate.sh` (đầy đủ, SQLite + Postgres) xanh; `cargo xtask check` xanh (ranh giới `api`, bản dịch đủ mã).
- [ ] Không chỗ nào trong core tự giữ tiền: không bảng / endpoint nhận tiền vào tài khoản của nền tảng
      ngoài qua `PaymentGateway`/`BankFeed`; tài liệu nói rõ (D-187).
- [ ] `docs/openapi.json`, `.env.example` sinh lại; `STACK.md` có `rust_decimal`, engine mẫu, thư viện PDF,
      thư viện XLSX; quyết định mới ghi `docs/DECISIONS.md` (decimal + làm tròn; numbering; contribute;
      payments trong ledger; chuỗi băm sổ cái; chọn thư viện mẫu / PDF / XLSX; căn cứ pháp lý của mục lưu
      trữ, máy tính tiền, khấu trừ thay).
- [ ] `docs/MONEY.md` + `docs/MONEY.vi.md` cùng nội dung; `MODULE_GUIDE` (+ `.vi`) cập nhật; `CHANGELOG.md`.
- [ ] Đo đường nóng ghi bút toán (`scripts/loadtest.sh` thêm một bài `POST /ledger/entries`) — ghi số đo kèm máy, ngày.
- [ ] Phát hành theo `docs/RELEASING.md`.

## Chỗ dễ sai

- **`SUM(bigint)` trên Postgres trả `numeric`** → giải mã `i64` hỏng; luôn `CAST(SUM(x) AS BIGINT)`. SQLite
  `SUM` tràn thì báo lỗi "integer overflow", còn `TOTAL` trả số thực — không dùng `TOTAL`.
- **Unique có cột null**: Postgres coi hai null là khác nhau, nên `counterparty` null trùng không bị chặn —
  dùng chuỗi rỗng / UUID nil thay null cho cột nằm trong khoá unique (cả hai database cùng hành vi).
- **Thứ tự khoá**: kỳ (chia sẻ) → số dư theo `account_id` tăng dần. Khoá ngược thứ tự = deadlock 40P01 trên
  Postgres (`write_with_retry` cứu được nhưng tốn); SQLite không lộ lỗi này — test đồng thời trên Postgres mới thấy.
- **Một lệnh lỗi làm hỏng cả transaction Postgres** (D-158): chèn trùng thì dùng `ON CONFLICT DO NOTHING`
  rồi đọc lại, không bắt lỗi unique giữa chừng.
- **Không gọi mạng trong transaction ghi** (cổng thanh toán, nhà cung cấp hoá đơn, nguồn tỉ giá, `TaxEngine`
  đọc database mặt điều khiển): SQLite chặn mọi lệnh ghi khác trong lúc đó. Gọi trước hoặc sau, trạng thái
  trung gian lưu trong bảng.
- **Số không nhảy**: chỉ cấp số trong transaction phát hành; không cấp ở nháp; không "trả lại" số; không
  gọi nhà cung cấp trước khi commit. `RETURNING` cần SQLite ≥ 3.35 (đã bật feature) — đừng đổi sang đọc rồi ghi.
- **Webhook**: kiểm chữ ký trên **thân thô** trước khi parse JSON (parse rồi serialize lại làm sai chữ ký);
  trả 2xx chỉ sau khi đã ghi bền; xử lý nặng trong job; đối tác gửi lại nhiều lần và không theo thứ tự —
  chuyển trạng thái chỉ theo hướng hợp lệ (`pending → succeeded`, không lùi).
- **Làm tròn hai lần** (làm tròn từng dòng rồi làm tròn tổng) lệch 1 đ so với hoá đơn nhà cung cấp tính;
  cách chia `allocate` ở bảng làm tròn là bắt buộc. VND có 0 chữ số thập phân, USD 2, KWD 3 — không giả định 2.
- **Ngày hiệu lực**: thuế suất dùng nửa mở `[from, to)`; ngày thuế theo múi giờ của nút người bán (cài đặt
  kế thừa), không theo UTC — 23:30 ngày 31/12 giờ Việt Nam vẫn là năm cũ.
- **Mẫu in do khách sửa** = server-side template injection nếu engine không sandbox; giới hạn kích thước
  mẫu và bước chạy.
- **CSV mở bằng Excel**: thiếu BOM thì tiếng Việt vỡ; ô bắt đầu bằng `=` thành công thức.
- **Chuỗi băm**: không băm `tenant_id` hay cột sẽ đổi khi tách / gộp khách (bước 38); định dạng `ledger-v1`
  không sửa, cần đổi thì thêm `v2`.
- **Mô tả bút toán** vào chuỗi băm nên không được chứa dữ liệu cá nhân (không xoá được) — chỉ chữ cố định
  của module, người / khách chỉ bằng ID.
- **`overflow-checks`** bật ở release (D-179): mọi phép cộng tiền dùng `Money::checked_*`, cộng dồn số dư
  dùng `rows::adjust` (chặn tràn ở `WHERE`).

## Căn cứ pháp lý

Nguồn đối chiếu ngày 13/11/2026 qua báo chí / công ty luật — **cần đối chiếu văn bản gốc / người có chuyên
môn pháp lý trước khi chạy thật** (D-186):

- Luật Kế toán 88/2015/QH13, Điều 41 (lưu chứng từ, sổ kế toán ≥ 10 năm), Điều 11 (đơn vị tiền tệ là Đồng
  Việt Nam, ngoại tệ quy đổi); NĐ 174/2016/NĐ-CP, Điều 12 (thời hạn tính từ kết thúc kỳ kế toán năm).
- Chế độ kế toán doanh nghiệp: TT 99/2025/TT-BTC (thay TT 200/2014, hiệu lực 01/01/2026 — cần đối chiếu);
  TT 133/2016/TT-BTC (doanh nghiệp nhỏ và vừa) — chỉ dùng cho ánh xạ số hiệu tài khoản khi xuất.
- NĐ 123/2020/NĐ-CP sửa bởi NĐ 70/2025/NĐ-CP (hiệu lực 01/6/2025): hoá đơn điện tử, hoá đơn khởi tạo từ máy
  tính tiền (ngưỡng doanh thu hộ kinh doanh 1 tỷ đ/năm, lộ trình — để dạng dữ liệu), xử lý hoá đơn sai sót
  (Điều 19); hướng dẫn của Bộ Tài chính về uỷ nhiệm lập hoá đơn (TT 78/2021/TT-BTC và văn bản thay thế — cần
  đối chiếu).
- NĐ 52/2024/NĐ-CP và TT 40/2024/TT-NHNN: dịch vụ hỗ trợ thu hộ, chi hộ cần giấy phép → core không giữ tiền (D-187).
- NĐ 117/2025/NĐ-CP (hiệu lực 01/7/2025): sàn có chức năng thanh toán khấu trừ, nộp thay thuế GTGT, TNCN của
  hộ / cá nhân kinh doanh; tỉ lệ là dữ liệu; cần đối chiếu NĐ 68/2026 và quy định hộ kinh doanh từ 2026.
- Luật Thuế GTGT 48/2024/QH15 (thuế suất 0/5/10%); giảm 2% (mức 8%) từ 01/7/2025 tới hết 31/12/2026 theo nghị
  quyết của Quốc hội và nghị định hướng dẫn — cần đối chiếu danh mục hàng không được giảm.
- Luật Thuế tiêu thụ đặc biệt 66/2025/QH15 (hiệu lực 01/01/2026) — chỉ đưa nhóm mẫu, cần đối chiếu biểu.
- VietQR: đặc tả mã QR của NAPAS dựa trên EMVCo; TCCS 03:2018/NHNN — cần đối chiếu bản mới nhất.
- Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 + NĐ 356/2025: giữ dữ liệu người mua trên chứng từ khi có nghĩa
  vụ pháp lý khác — cần đối chiếu điều khoản ngoại lệ khi xoá.

## Agent làm bước này tự quyết

Ghi mỗi quyết định vào `docs/DECISIONS.md` (skill `record-decision`).

- **Thư viện PDF**: tiêu chí — thuần Rust, không phụ thuộc hệ điều hành, nhúng font Unicode có dấu tiếng Việt
  (font giấy phép OFL, vd Noto Sans), bảng nhiều trang, license qua `cargo deny`. Ứng viên: `typst` (dùng
  như thư viện; mẫu PDF viết bằng Typst → hai định dạng mẫu), `printpdf` (bản có dựng HTML), `krilla`.
  Khuyến nghị: một nguồn mẫu HTML; nếu không thư viện nào dựng HTML đủ tốt thì dùng `typst` cho PDF và giữ
  view model chung. Đo thời gian dựng và kích thước binary trước / sau.
- **Engine mẫu**: khuyến nghị `minijinja` (sandbox, giới hạn fuel, auto-escape); tự viết thay thế chỉ khi
  đủ nhỏ.
- **XLSX**: khuyến nghị `rust_xlsxwriter`; hoặc chỉ CSV ở bước này nếu XLSX kéo thư viện lớn (ROADMAP ghi
  "CSV / Excel" — CSV có BOM mở được bằng Excel).
- **QR dạng ảnh** cho mẫu in: trả chuỗi payload qua API (giao diện tự vẽ); mẫu PDF cần ảnh thì thêm `qrcode`
  không bật feature ảnh (xuất SVG) — hoặc bỏ QR khỏi PDF.
- **Mở lại kỳ đã khoá**: mặc định không có. Nếu thấy cần (khoá nhầm), chỉ cho phép kỳ cuối cùng, permission
  riêng, audit — ghi quyết định.
- **Tách `payments` khỏi `ledger`** thành module riêng: chỉ khi `ledger` vượt cỡ dễ đọc; tên bảng đổi
  theo, ghi quyết định thay mục "payments trong ledger".
- **Dãy số khi nhà cung cấp tự cấp số**: chọn giữ hai số (nội bộ + của nhà cung cấp) hay chỉ số của nhà cung
  cấp cho ký hiệu đó; khuyến nghị theo `EInvoiceCapabilities::assigns_numbers` từng ký hiệu.
- **Ánh xạ tài khoản "vn"** và hồ sơ xuất cho phần mềm kế toán cụ thể: chỉ đưa vào khi đối chiếu được nguồn
  công bố; không thì để hồ sơ chung và cho khách tự ánh xạ.

## Bổ sung theo đối chiếu thiết kế 2 (D-208…D-218)

> Nguồn: `docs/prompts/ui-trace-roadmap.md`; mã `A-…` / `B-…` trỏ tới `docs/research/ui-trace/`.

- **Thống nhất với 45, 46 (D-218, B-b3, B-3.2d).** Bộ tên tài khoản ở đây là bộ duy nhất; tiền thu hộ ghi
  lúc thu vào `clearing.*`; phí sàn và khấu trừ thay ghi lúc `orders.order_completed`; `ledger_holds` là cơ
  chế tạm giữ duy nhất — `api` thêm `Ledger::hold_status(source_module, source_id)` cho `marketplace`.
- **Một hạn thanh toán (D-218, B-b10).** `AUTOXIDE_LEDGER_PAYMENT_EXPIRY_MINUTES` (30) là nguồn duy nhất,
  kể cả VietQR; `api` trả `expires_at` cho `orders`.
- **COD (D-216, B-b1).** `ledger_flow_settings.cod_remittance` (`partner_collection` / `seller_direct`):
  tiền COD về tài khoản thu hộ của đối tác hay thẳng người bán, **không bao giờ** về tài khoản của nhà vận
  hành sàn; `seller_direct` thì phí và khấu trừ thay thu bằng hoá đơn phí định kỳ hay công nợ
  (`receivable.seller[S]`).
- **Mã số thuế 12 số (B-c11).** Hồ sơ thuế nhận cả mã số thuế 10 / 13 số và số định danh cá nhân 12 số dùng
  làm mã số thuế (TT 86/2024/TT-BTC — **cần đối chiếu văn bản gốc**). Tuỳ chọn trait
  `TaxpayerDirectory::lookup(tax_id)` (bản giả cho test; bản nối thật cần tài khoản đối tác — khi đó dừng
  lại hỏi chủ dự án).
- **Bút toán theo chứng từ nguồn (B-c12).** `GET /api/v1/ledger/entries?source_module&source_id`
  (`ledger.entries.read`, phạm vi theo nút).
- **Phí sàn và khấu trừ thay (B-3.2e, f).** Hoá đơn phí sàn tách thuế GTGT riêng; khấu trừ thay chỉ áp cho
  hộ và cá nhân kinh doanh, doanh nghiệp tự kê khai (theo `tax_profile.kind`). Căn cứ NĐ 117/2025/NĐ-CP —
  **cần đối chiếu văn bản gốc**.

## Bổ sung theo đối chiếu thiết kế đợt 2 (D-223…D-227)

> Nguồn: `docs/prompts/ui-phase2-api.md`; màn tương ứng ghi trong `docs/research/ui-traceability.md` (đợt 2).

- **Trạng thái niêm phong.** Bút toán trả `sealed_at` (từ `ledger_entry_seals`, NULL là chưa vào chuỗi băm) và
  `sequence`. `GET /api/v1/ledger/books/{id}/seal-status?period_id` trả `total`, `sealed`, `oldest_unsealed_at` —
  đếm bằng index, không quét bảng. `autoxide doctor` cảnh báo khi bút toán chưa niêm phong quá 10 phút.
- **Số dư theo đối tượng cho module khác.** `Ledger::balances(tenant, keys: &[AccountKey], as_of)` (tối đa 500
  khoá, một query) — `inventory` dùng cho cột "đang nợ" của nhà cung cấp, không đọc bảng sổ cái.
  `Counterparty.kind` thêm `supplier`.
