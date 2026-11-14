# Bước 46 — Sàn (`marketplace`)

> Lộ trình: `docs/ROADMAP.md` bước 46 · Thiết kế: `docs/research/business-foundation.md` mục 3 (trụ 4,
> trụ 5), mục 4, mục 5 (dòng "Sàn"), mục 8, mục 9 · Quyết định: D-185, D-186, D-187, D-188, D-190,
> D-191, D-194 · Đặc tả liên quan: `docs/steps/45-orders.md`

## Mục tiêu

Một khách chạy được sàn kiểu Shopee / Lazada: nhiều người bán, người mua dùng chung, một lần đặt hàng
tách thành đơn theo từng người bán. Người bán là nút loại `seller` của `organizations`, chỉ bán được
khi đã xác thực danh tính (KYC, cổng chặn của bước 37), đã chấp nhận quy chế sàn và hợp đồng với người
bán đúng phiên bản, có đủ giấy tờ cho ngành hàng có điều kiện. Sàn duyệt sản phẩm, thu phí theo ngành
hàng (dữ liệu có ngày hiệu lực), giữ phần của người bán tới hết hạn đổi trả rồi lệnh chi qua
`PayoutGateway` sau khi trừ phí và **thuế khấu trừ thay** (NĐ 117/2025) — sổ cái khớp từng đồng. Có
tranh chấp người mua – người bán, điểm và chế tài người bán, báo cáo cho cơ quan thuế theo kỳ, cung
cấp thông tin khi cơ quan có thẩm quyền yêu cầu, và kho của sàn (sàn giữ hàng của người bán).

Core không giữ tiền (D-187): ở chế độ thu hộ, tiền nằm ở tài khoản thu hộ của đối tác có giấy phép;
`marketplace` chỉ ghi nghĩa vụ và ra lệnh chi.

## Bắt đầu khi

- Bước 45 (`orders`) đã merge — dùng `OrderExtensions::set_seller_split`, `OrderHolds`, `Orders`, event
  `orders.*`.
- Bước 37 đã merge: `compliance` (KYC + cổng chặn, văn bản pháp lý có phiên bản và chấp nhận, báo vi
  phạm, sổ yêu cầu của cơ quan có thẩm quyền), `notifications`.
- Sẵn từ trước: 34 (`OrgDirectory`, nút `seller`, `Address`), 35 (gói: tính năng `marketplace`), 36
  (`Ledger`, `TaxEngine` có thuế khấu trừ thay, `PayoutGateway`, `Invoices`, ba chế độ dòng tiền), 41
  (`catalog`), 42 (`inventory`), 44 (`shipping`).
- Song song được với 47, 48, 49.

## Phạm vi

Crate mới `modules/marketplace` (`autoxide-marketplace`), đăng ký sau `orders`. Bố cục theo tính năng:
`sellers/`, `listings/`, `fees/`, `settlement/` (tách đơn, giữ tiền, chi trả, đối soát), `disputes/`,
`performance/` (điểm, chế tài), `authorities/` (báo cáo thuế, yêu cầu của cơ quan), `fulfillment/`
(kho của sàn), cùng `lib.rs`, `api.rs`, `config.rs`, `permissions.rs`, `events.rs`, `entity/`,
`migration/`, `http/`, `tools.rs`, `tests/`.

Sửa ngoài crate: `Cargo.toml`, `crates/app/Cargo.toml`, `crates/app/src/modules.rs`,
`docs/openapi.json`, `.env.example`, `CHANGELOG.md`, `docs/ROADMAP.md`.

NGOÀI phạm vi: bộ nối `PayoutGateway` thật / cổng chia tiền thật (module riêng, cần secret thật); nộp
hồ sơ lên cổng của cơ quan nhà nước (con người làm; hệ thống xuất dữ liệu); quảng cáo trong sàn — làm ở
bước 64 (module `ads`, D-221); người bán nước ngoài — làm ở bước 63 (D-233): tắt mặc định, điều kiện là gói
luật có ngày hiệu lực; bước đó thêm `residency`, quốc gia vào `marketplace_sellers` bằng migration mới và mở
rộng `SellerGate`, `SellerDisclosure`, chi trả ngoại tệ — bước này chỉ làm người bán trong nước; trang quy
chế sàn và mẫu hợp đồng (nội dung mẫu ở bước 37).

## Thiết kế

### Vòng đời người bán

`draft` → `pending_verification` (chờ KYC, chấp nhận văn bản) → `pending_review` (sàn duyệt hồ sơ) →
`active` → `suspended` (chế tài / KYC hết hạn / văn bản mới chưa chấp nhận) ↔ `active` →
`terminated`. Chỉ `active` được đăng bán, nhận đơn, nhận chi trả mới. Mỗi lần cần quyết "người bán
này được bán không", service gọi một hàm duy nhất `SellerGate::ensure_can_sell(tenant, seller_org)`:
trạng thái `active` **và** `compliance` báo KYC còn hạn **và** đã chấp nhận phiên bản hiện hành của
`marketplace.operating_rules` và `marketplace.seller_agreement`. KYC hết hạn hay văn bản đổi phiên bản
(event của `compliance`) → tự chuyển `suspended` với lý do, báo người bán.

Người bán đăng ký: tài khoản nhân sự (toàn nền tảng, bước 34) tạo hồ sơ → `marketplace` tạo nút
`seller` dưới nút sàn qua `OrgDirectory` (người đăng ký thành thành viên với role chủ gian hàng), hồ sơ
pháp lý theo người bán qua `compliance`, gian hàng (tên, slug, logo, mô tả, địa chỉ lấy hàng
`Address`, cách giao: `self` / `platform`). Role chủ gian hàng gồm quyền quản lý banner của `content` ở độ sâu `unit`
tại nút `seller` — cho banner trong gian hàng của bước 55 (D-233).

### Điểm cắm vào module khác

- `orders`: `set_seller_split(Arc<SellerSplitter>)` — gom dòng giỏ theo `owner_org_id` của biến thể
  (catalog), gọi `ensure_can_sell` cho từng người bán (lỗi `marketplace.seller_not_active` → checkout
  hỏng trước khi giữ chỗ), trả `SellerGroup { seller_org_id, fulfillment_org_id (kho người bán hay kho
  sàn), lines }`. `orders` tạo một đơn cho mỗi nhóm, chung một `checkout_id` và một khoản thanh toán.
- `catalog` (giả định bước 41 có điểm cắm cùng kiểu): `CatalogExtensions::set_listing_gate(Arc<dyn
  ListingGate>)` — sản phẩm của nút `seller` chỉ sang `active` khi `ListingGate::can_publish` đồng ý
  (người bán `active` + sản phẩm đã duyệt + đủ giấy tờ ngành hàng). Nếu bước 41 không có điểm cắm
  này: ghi vào báo cáo cho agent chính, tạm dùng cách nghe `catalog.product_status_changed` rồi gọi
  `Catalog::suspend_listing` — kém hơn vì có khoảng hở.
- `orders::OrderHolds::hold_completion` khi mở tranh chấp; `release_completion` khi đóng.

### Tiền: tách, giữ, chi

Khi đơn của người bán được thanh toán (`orders.order_confirmed`, đơn có `funds_flow` khác `SelfCollect`):
tính và lưu một **bảng quyết toán** cho đơn (`marketplace_settlements`) — ảnh chụp tại thời điểm đó:

```
gross            = tổng đơn người mua trả cho phần của người bán (gồm thuế, sau giảm của người bán)
platform_subsidy = phần giảm giá do sàn chịu (khuyến mại của sàn) — sàn bù cho người bán
fees[]           = phí theo marketplace_fee_rules hiệu lực tại placed_at (hoa hồng theo ngành, phí thanh toán, phí dịch vụ, phí kho)
withholding[]    = TaxEngine::withholding(tenant, seller_org, gross_taxable, placed_at) — GTGT, TNCN của hộ / cá nhân kinh doanh (NĐ 117/2025); 0 với người bán là doanh nghiệp tự kê khai
net_payable      = gross + platform_subsidy − Σ fees − Σ withholding     (Money, checked; chia dòng bằng allocate)
hold_until       = delivered_at + return_window_days (chính sách của người bán, kế thừa) ; null tới khi giao xong
```

Bút toán (tài khoản theo khoá của `ledger`, `org_id` = nút sàn, đối tượng = người bán):

| Lúc | Nợ | Có |
|---|---|---|
| `orders.order_completed` (hết hạn đổi trả, không tranh chấp) | `buyer_prepayments` (gross) · `promo_subsidy_expense` (subsidy) | `seller_payable:<seller>` (net) · `platform_fee_revenue:<loại>` (phí) · `withholding_vat_payable` · `withholding_pit_payable` |
| `orders.refund_completed` trước hoàn tất | — (orders đã đảo `buyer_prepayments`) | quyết toán tính lại theo phần còn lại, chưa ghi gì |
| `orders.refund_completed` sau hoàn tất (hiếm: tranh chấp phán cho người mua) | bút toán đảo theo tỉ lệ dòng hoàn | |
| Lệnh chi thành công | `seller_payable:<seller>` | `partner_collection:<gateway>` |
| Phạt / khoản trừ người bán | `seller_payable:<seller>` | `penalty_revenue` |

Chế độ `GatewaySplit`: cổng chia ngay tại lúc thu, không có lệnh chi; bút toán hoàn tất giống trên,
còn "lệnh chi" được thay bằng đối soát với báo cáo chia tiền của cổng (Nợ `seller_payable` / Có
`partner_collection`). Chế độ `SelfCollect` trên sàn (sàn không có chức năng thanh toán): không quyết
toán tiền, không khấu trừ thay; sàn thu phí bằng hoá đơn định kỳ cho người bán (`Invoices`), và vẫn
phải cung cấp thông tin người bán cho cơ quan thuế.

Chi trả: job `marketplace.build_payouts` theo lịch của sàn (`payout_schedule`: hằng ngày / tuần) gom
mọi quyết toán `releasable` của người bán, trừ số âm còn nợ (phạt, hoàn sau hoàn tất), bỏ qua nếu dưới
`min_payout`; tạo `marketplace_payouts` (`reference` = `marketplace:payout:<id>`) → job
`marketplace.send_payout` gọi `PayoutGateway::payout` ngoài transaction → webhook / truy vấn trạng thái
→ `paid` (bút toán) hoặc `failed` (quyết toán trở lại `releasable`). Chứng từ khấu trừ thuế cho người
bán qua `Invoices` (loại chứng từ khấu trừ — giả định bước 36 có).

### Kiểu và trait (phác thảo — tra API thật bằng skill `check-library-api`)

```rust
// marketplace::api
pub enum SellerStatus { Draft, PendingVerification, PendingReview, Active, Suspended, Terminated }
pub struct SellerSummary { pub org_id: OrgId, pub status: SellerStatus, pub display_name: String,
    pub slug: String, pub fulfillment: FulfillmentMode, pub rating: Option<SellerRating> } // #[non_exhaustive]
#[async_trait] pub trait Sellers: Send + Sync {
    async fn find(&self, tenant: TenantId, seller: OrgId) -> Result<Option<SellerSummary>, Error>;
    async fn ensure_can_sell(&self, tenant: TenantId, seller: OrgId) -> Result<(), Error>;
    /// Người bán đang hoạt động, đã xác thực và đủ giấy tờ cho mọi danh mục trong `category_ids` — mời thầu của
    /// bước 59 dùng (D-232). Trả danh sách người bán đủ điều kiện, phân trang.
    async fn eligible_for_categories(&self, tenant: TenantId, category_ids: &[Uuid], page: PageRequest)
        -> Result<Page<SellerSummary>, Error>;
}
/// Thông tin người bán bắt buộc hiện trên trang sản phẩm (dữ liệu công khai, không có dữ liệu cá nhân thừa).
#[async_trait] pub trait SellerDisclosure: Send + Sync {
    async fn public_profile(&self, tenant: TenantId, seller: OrgId) -> Result<Option<PublicSellerProfile>, Error>;
}
```

Giả định từ bước khác: `compliance::api::{Kyc::status(tenant, subject) -> KycStatus { verified, expires_at }`,
`LegalDocuments::{current_version(kind), has_accepted(tenant, subject, kind, version)}`,
`ViolationReports` (event `compliance.listing_reported`, `compliance.listing_decision`),
`AuthorityRequests::record(...)`}; `tax::api::TaxEngine::withholding(tenant, WithholdingRequest { seller_org, kind: Goods/Services, gross, date }) -> Vec<WithholdingLine { code, rate, base, amount }>`;
kernel `PayoutGateway::payout(PayoutRequest { reference, beneficiary: BankAccount, amount, memo }) -> PayoutStatus`;
`secret` kho theo khách để lưu số tài khoản nhận tiền (mã hoá).

### Bảng dữ liệu

Quy ước như bước 45: `tenant_id` đầu mọi khoá; UUIDv7; UTC; `Money` hai cột; JSON trong `text`;
trạng thái `string_len(32)`; migration có `down`; `outbox_migration`, `jobs_migration`.

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `marketplace_sellers` | `tenant_id`, `org_id` (nút seller, khoá chính cùng tenant), `status`, `status_reason` NULL, `display_name`, `slug`, `logo_media_id` NULL, `description`, `pickup_address` text (`Address`), `fulfillment` (`self/platform`), `seller_type` (`enterprise/household/individual`), `payout_account_secret` (tham chiếu kho secret, không lưu số tài khoản rõ), `rating_*`, `penalty_points` INT, `created_at`, `updated_at`, `activated_at` NULL | pk (`tenant_id`, `org_id`); unique (`tenant_id`, `slug`); index (`tenant_id`, `status`) |
| `marketplace_seller_documents` | `id`, `tenant_id`, `seller_org_id`, `category_id` NULL, `doc_type` (mã trong danh mục giấy tờ của catalog), `file_key` (Storage, khoá có tiền tố khách), `status` (`submitted/approved/rejected/expired`), `valid_until` NULL, `reviewed_by` NULL, `review_note` NULL, `created_at`, `updated_at` | index (`tenant_id`, `seller_org_id`, `status`), (`tenant_id`, `status`, `valid_until`) |
| `marketplace_listing_reviews` | `id`, `tenant_id`, `seller_org_id`, `product_id`, `revision` INT, `status` (`pending/approved/rejected/delisted`), `findings` text (hàng cấm, thiếu giấy tờ, báo vi phạm), `decided_by` NULL, `decided_at` NULL, `created_at` | unique (`tenant_id`, `product_id`, `revision`); index (`tenant_id`, `status`, `id`) |
| `marketplace_fee_rules` | `id`, `tenant_id`, `fee_kind` (`commission/payment/service/fulfillment/storage`), `category_id` NULL (NULL = mặc định), `seller_type` NULL, `rate_bp` INT, `fixed_*` NULL, `cap_*` NULL, `effective_from`, `effective_to` NULL, `created_by`, `created_at` | index (`tenant_id`, `fee_kind`, `category_id`, `effective_from`); chỉ thêm — sửa = đóng `effective_to` + dòng mới |
| `marketplace_settlements` | `id`, `tenant_id`, `order_id` (unique), `seller_org_id`, `funds_flow`, `gross_*`, `subsidy_*`, `fees_*`, `withholding_*`, `net_*`, `breakdown` text (từng phí, từng thuế kèm tỉ lệ ảnh chụp), `status` (`pending/held/releasable/in_payout/paid/reversed/offset`), `hold_until` NULL, `payout_id` NULL, `ledger_reference` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `order_id`); index (`tenant_id`, `seller_org_id`, `status`, `hold_until`) |
| `marketplace_adjustments` | `id`, `tenant_id`, `seller_org_id`, `kind` (`penalty/refund_after_completion/correction`), `amount_*` (âm = trừ), `reason`, `source_ref`, `payout_id` NULL, `created_by`, `created_at` | unique (`tenant_id`, `source_ref`); index (`tenant_id`, `seller_org_id`, `payout_id`) |
| `marketplace_payouts` | `id`, `tenant_id`, `seller_org_id`, `period_from`, `period_to`, `amount_*`, `status` (`building/sent/paid/failed/cancelled`), `reference`, `external_id` NULL, `failure` NULL, `withholding_certificate_id` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `reference`); index (`tenant_id`, `seller_org_id`, `id`), (`tenant_id`, `status`) |
| `marketplace_disputes` | `id`, `tenant_id`, `number`, `order_id`, `seller_org_id`, `buyer_user_id`, `reason_code`, `claim_*` (số tiền đòi), `status` (`open/seller_responded/escalated/resolved/withdrawn`), `resolution` NULL (`refund_full/refund_partial/reject/return_and_refund`), `resolution_*` NULL, `seller_due_at`, `platform_due_at`, `resolved_by` NULL, `resolved_at` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `number`); unique (`tenant_id`, `order_id`, `open_key`) — một tranh chấp mở mỗi đơn; index (`tenant_id`, `status`, `platform_due_at`) |
| `marketplace_dispute_messages` | `id`, `tenant_id`, `dispute_id`, `author_kind` (`buyer/seller/platform`), `author_id`, `body`, `media_ids` text, `created_at` | index (`tenant_id`, `dispute_id`, `id`) — chỉ thêm |
| `marketplace_seller_metrics` | `tenant_id`, `seller_org_id`, `period` (`YYYY-MM`), `orders`, `cancelled_by_seller`, `late_shipments`, `disputes_lost`, `rating_sum`, `rating_count`, `updated_at` | pk (`tenant_id`, `seller_org_id`, `period`); cập nhật bằng `rows::adjust` |
| `marketplace_penalties` | `id`, `tenant_id`, `seller_org_id`, `rule_code`, `points`, `sanction` (`warning/listing_limit/suspend_days/terminate`), `reason`, `expires_at`, `appealed` BOOL, `created_by`, `created_at` | index (`tenant_id`, `seller_org_id`, `expires_at`) |
| `marketplace_penalty_rules` | `id`, `tenant_id`, `rule_code`, `threshold` text, `points`, `sanction`, `effective_from`, `effective_to` NULL | index (`tenant_id`, `rule_code`, `effective_from`) |
| `marketplace_tax_reports` | `id`, `tenant_id`, `period_kind` (`month/quarter`), `period`, `status` (`draft/final/submitted`), `file_key`, `row_count`, `totals` text, `submitted_at` NULL, `submitted_by` NULL, `created_at` | unique (`tenant_id`, `period_kind`, `period`, `revision`) |
| `marketplace_authority_requests` | `id`, `tenant_id`, `authority`, `document_ref` (số công văn), `received_at`, `scope` text (người bán, khoảng thời gian, loại dữ liệu), `due_at`, `status` (`received/fulfilled/rejected`), `response_file_key` NULL, `handled_by`, `created_at`, `updated_at` | index (`tenant_id`, `status`, `due_at`) — mọi thao tác ghi audit |
| `marketplace_inbound_shipments` | `id`, `tenant_id`, `seller_org_id`, `warehouse_org_id` (kho của sàn), `number`, `lines` text (variant, số khai báo, số nhận, hỏng), `status` (`announced/in_transit/received/discrepancy/closed`), `received_at` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `number`); index (`tenant_id`, `seller_org_id`, `status`) |

Tồn của người bán trong kho của sàn: `inventory` theo biến thể × kho; biến thể thuộc người bán nên tồn
"theo người bán trong kho sàn" suy ra được — nhận hàng nhập kho gọi `Inventory::receive` (giả định
bước 42) với `reference = marketplace:inbound:<id>:<lần>`. Phí lưu kho tính theo `storage` fee rule
bằng job tháng (`marketplace.charge_storage`) thành `marketplace_adjustments`.

### API REST (`/api/v1/marketplace`)

| Method + path | Permission | Ghi chú |
|---|---|---|
| `POST /sellers` | `marketplace.sell` (nhân sự đăng ký làm người bán) | tạo hồ sơ + nút `seller`; 201 |
| `GET /sellers/me` · `PUT /sellers/me` | `marketplace.sell` (phạm vi `Unit` nút seller) | gian hàng, cách giao, tài khoản nhận tiền (ghi vào kho secret) |
| `POST /sellers/me/submit` | `marketplace.sell` | kiểm đủ KYC + văn bản + giấy tờ → `pending_review`; thiếu → 422 liệt kê từng thứ |
| `POST /sellers/me/documents` | `marketplace.sell` | giấy tờ ngành hàng (file qua `files`) |
| `GET /sellers` · `GET /sellers/{org_id}` | `marketplace.sellers.read` | lọc trạng thái |
| `POST /sellers/{org_id}/transitions` | `marketplace.sellers.manage` | duyệt / từ chối / tạm khoá / chấm dứt, `reason`, `If-Match` |
| `POST /documents/{id}/decision` | `marketplace.sellers.manage` | duyệt giấy tờ |
| `GET /public/sellers/{slug}` | công khai | thông tin người bán bắt buộc (tên, MST / số ĐKKD, địa chỉ, liên hệ) + gian hàng; ETag |
| `GET /listing-reviews` · `POST /listing-reviews/{id}/decision` | `marketplace.listings.review` | duyệt sản phẩm; `findings` tự điền từ danh mục hàng cấm / có điều kiện và báo vi phạm |
| `GET /fee-rules` · `POST /fee-rules` · `POST /fee-rules/{id}/close` | `marketplace.fees.manage` | dữ liệu có ngày hiệu lực; không sửa dòng đã hiệu lực |
| `GET /fee-rules/preview` | `marketplace.fees.manage` / `marketplace.sell` | tính thử phí cho một ngành hàng, giá, ngày |
| `GET /settlements` · `GET /settlements/{order_id}` | `marketplace.finance.read` / `marketplace.sell` (của mình) | bảng đối soát cho người bán |
| `GET /payouts` · `GET /payouts/{id}` | như trên | kèm danh sách quyết toán, chứng từ khấu trừ |
| `POST /payouts/{id}/retry` | `marketplace.finance.manage` | chi lại lệnh `failed` (cùng `reference` mới phiên bản) |
| `GET /statements?from&to` | `marketplace.sell` / `marketplace.finance.read` | sao kê CSV / JSON: đơn, phí, thuế, chi trả — khớp sổ cái |
| `POST /disputes` | `orders.buy` (người mua của đơn) | mở tranh chấp trong hạn; hoãn hoàn tất đơn |
| `POST /disputes/{id}/messages` | bên liên quan | |
| `POST /disputes/{id}/transitions` | người bán (trả lời / đồng ý hoàn), `marketplace.disputes.resolve` (phân xử) | quyết `refund_*` → gọi `orders::api::OrderRefunds::refund` với `reference = marketplace:dispute:<id>` |
| `GET /disputes` | `marketplace.disputes.resolve` / người bán (của mình) | lọc quá hạn |
| `GET /sellers/{org_id}/performance` · `POST /penalties` · `POST /penalties/{id}/appeal` | `marketplace.sellers.manage` / người bán | điểm, chế tài, khiếu nại chế tài |
| `POST /tax-reports` · `GET /tax-reports/{id}` · `POST /tax-reports/{id}/submitted` | `marketplace.tax.report` | dựng báo cáo kỳ (job), tải file, đánh dấu đã nộp |
| `POST /authority-requests` · `POST /authority-requests/{id}/fulfil` | `marketplace.authority.respond` | ghi yêu cầu, xuất dữ liệu đúng phạm vi, audit |
| `POST /inbound-shipments` · `POST /inbound-shipments/{id}/receive` | người bán / `marketplace.fulfillment.manage` | kho của sàn |

### Tool MCP

`list_sellers`, `get_seller` (`read_only`), `decide_seller`, `list_listing_reviews`,
`decide_listing`, `list_fee_rules` (`read_only`), `add_fee_rule`, `list_settlements` (`read_only`),
`list_disputes`, `resolve_dispute`, `seller_performance` (`read_only`), `build_tax_report`. Cùng
service, cùng permission với REST. Không có tool chi tiền thủ công ngoài `retry_payout`
(`destructive`, `marketplace.finance.manage`).

### Event

`marketplace.seller_registered`, `marketplace.seller_status_changed`, `marketplace.document_decided`,
`marketplace.listing_decided`, `marketplace.fee_rule_added`, `marketplace.settlement_created`,
`marketplace.settlement_releasable`, `marketplace.payout_sent`, `marketplace.payout_paid`,
`marketplace.payout_failed`, `marketplace.dispute_opened`, `marketplace.dispute_resolved`,
`marketplace.penalty_applied`, `marketplace.tax_report_built`, `marketplace.authority_request_fulfilled`,
`marketplace.inbound_received`. Payload có `tenant_id`, `org_id` (seller), tiền — cho bước 38.

Nghe: `orders.order_confirmed` (tạo quyết toán `pending`), `orders.order_delivered` (đặt
`hold_until`), `orders.order_completed` (ghi bút toán, `releasable` khi qua `hold_until`),
`orders.refund_completed`, `orders.order_cancelled`, `orders.review_published` (điểm người bán),
`shipping.shipment_*` (giao trễ), `affiliate.commission_approved` / `affiliate.commission_adjusted`
(chỉ khi module `affiliate` có trong chương trình — đăng ký nghe theo tên như `comments` làm với
`content`: hoa hồng do người bán chịu thành `marketplace_adjustments` trừ vào quyết toán), sự kiện KYC / văn bản / báo vi phạm của `compliance` (tạm khoá, gỡ
sản phẩm), `catalog.product_status_changed`. Mọi bộ nghe chịu được nhận lại (khoá theo `order_id` /
`source_ref`).

### Job và lịch

`marketplace.release_settlements` (mỗi giờ: `held` qua `hold_until` + không tranh chấp →
`releasable`), `marketplace.build_payouts` (theo `payout_schedule`), `marketplace.send_payout`
(enqueue), `marketplace.poll_payouts` (mỗi 15 phút với lệnh `sent` lâu), `marketplace.escalate_disputes`
(mỗi giờ: người bán quá hạn trả lời → `escalated`; sàn quá hạn → cảnh báo), `marketplace.expire_documents`
(mỗi ngày), `marketplace.recompute_performance` (mỗi ngày, áp `marketplace_penalty_rules`),
`marketplace.charge_storage` (mỗi tháng), `marketplace.build_tax_report` (enqueue).

### Cấu hình

`AUTOXIDE_MARKETPLACE_SELLER_RESPONSE_HOURS` (48), `AUTOXIDE_MARKETPLACE_PLATFORM_RESOLUTION_DAYS`
(7), `AUTOXIDE_MARKETPLACE_DISPUTE_WINDOW_DAYS` (bằng hạn đổi trả + 3 → khuyến nghị đặt theo chính
sách), `AUTOXIDE_MARKETPLACE_TAX_REPORT_RETENTION_YEARS` (10). Lịch chi, `min_payout`, cách khấu trừ
là dữ liệu theo khách (bảng `marketplace_settings`: `tenant_id` pk, `payout_schedule`, `min_payout_*`,
`payout_gateway`, `updated_at`). Tính năng: `marketplace.platform_fulfillment`,
`marketplace.disputes`. Hạn mức: `marketplace.sellers` (số người bán).

### Permission

`marketplace.sell`, `marketplace.sellers.read`, `marketplace.sellers.manage`,
`marketplace.listings.review`, `marketplace.fees.manage`, `marketplace.finance.read`,
`marketplace.finance.manage`, `marketplace.disputes.resolve`, `marketplace.tax.report`,
`marketplace.authority.respond`, `marketplace.fulfillment.manage`.

### Mã lỗi

| Code | HTTP | Tiếng Việt |
|---|---|---|
| `marketplace.seller_not_found` | 404 | Không tìm thấy người bán. |
| `marketplace.seller_not_active` | 409 | Người bán {name} hiện chưa được bán hàng trên sàn. |
| `marketplace.kyc_required` | 403 | Cần hoàn tất xác thực danh tính trước khi bán hàng. |
| `marketplace.agreement_not_accepted` | 403 | Cần đồng ý {document} phiên bản {version}. |
| `marketplace.documents_missing` | 422 | Ngành hàng {category} cần giấy tờ: {documents}. |
| `marketplace.slug_taken` | 409 | Tên gian hàng {slug} đã có người dùng. |
| `marketplace.invalid_transition` | 409 | Không chuyển từ {from} sang {to} được. |
| `marketplace.listing_rejected` | 409 | Sản phẩm chưa được duyệt: {reason}. |
| `marketplace.fee_rule_overlap` | 409 | Đã có phí {kind} cho ngành hàng này hiệu lực từ {from}. |
| `marketplace.fee_rule_in_past` | 422 | Không thêm phí có hiệu lực trong quá khứ; ngày sớm nhất là {min}. |
| `marketplace.payout_account_missing` | 422 | Người bán chưa khai tài khoản nhận tiền. |
| `marketplace.payout_not_retryable` | 409 | Lệnh chi đang ở trạng thái {status}, không chi lại được. |
| `marketplace.dispute_window_closed` | 409 | Đã quá hạn mở tranh chấp cho đơn này. |
| `marketplace.dispute_exists` | 409 | Đơn này đang có một tranh chấp chưa xong. |
| `marketplace.dispute_not_found` | 404 | Không tìm thấy tranh chấp. |
| `marketplace.resolution_exceeds_order` | 422 | Số tiền hoàn ({amount}) lớn hơn số đã thu của đơn ({paid}). |
| `marketplace.report_period_open` | 409 | Kỳ {period} chưa kết thúc, chưa dựng báo cáo chính thức được. |
| `marketplace.inbound_not_found` | 404 | Không tìm thấy phiếu gửi hàng vào kho sàn. |

## Thứ tự làm

1. Khung crate, permission, cấu hình, `api.rs`, đăng ký. → chuẩn bị
2. Người bán: hồ sơ, nút `seller`, vòng đời, `SellerGate`, giấy tờ, trang công khai, nghe event
   `compliance`. → **mục 1**
3. Duyệt sản phẩm + `ListingGate` + báo vi phạm. → **mục 2**
4. Phí có ngày hiệu lực + `SellerSplitter` cắm vào `orders` (tách đơn theo người bán, tự giao / sàn
   giao). → **mục 3**
5. Quyết toán, giữ tới hết hạn đổi trả, thuế khấu trừ thay, bút toán, chi trả qua `PayoutGateway`,
   bảng đối soát. → **mục 4**
6. Tranh chấp, điểm và chế tài, yêu cầu của cơ quan có thẩm quyền. → **mục 5**
7. Báo cáo cho cơ quan thuế theo kỳ; kho của sàn (nhận hàng, tồn theo người bán, phí lưu kho). → **mục 6**
8. `docs/openapi.json`, `.env.example`, CHANGELOG, ROADMAP, quyết định.

## Test bắt buộc

Cả SQLite và Postgres; `TestApp` với `orders` và các module nền thật, `PayoutGateway` /
`PaymentGateway` / `IdentityVerifier` giả.

- `unverified_seller_cannot_list_or_sell` — KYC chưa xong: sản phẩm không lên `active`, checkout có
  dòng của người bán đó → `marketplace.seller_not_active`.
- `new_agreement_version_suspends_until_accepted`; `kyc_expiry_suspends_seller`.
- `checkout_splits_into_one_order_per_seller` — 3 người bán, 1 thanh toán, 3 đơn, số đơn liền.
- `fee_rule_effective_dates_apply_at_order_time` — đổi phí giữa hai đơn: mỗi đơn dùng phí của ngày đặt.
- `settlement_is_ledger_exact` — với giảm giá của sàn và của người bán, nhiều thuế suất: Σ nợ = Σ có,
  `net = gross + subsidy − fees − withholding` đúng từng đồng (chia dòng bằng `allocate`).
- `withholding_only_for_household_sellers_in_partner_collect` — doanh nghiệp: 0; hộ kinh doanh: theo
  tỉ lệ của `TaxEngine` (gói "vn").
- `settlement_held_until_return_window_and_dispute` — tranh chấp mở → không `releasable`.
- `refund_before_completion_recomputes_settlement`; `refund_after_completion_offsets_next_payout`.
- `payouts_built_concurrently_include_each_settlement_once` — hai worker chạy `build_payouts` cùng
  lúc: mỗi quyết toán thuộc đúng một lệnh chi.
- `duplicate_payout_webhook_marks_paid_once`; `failed_payout_returns_settlements`.
- `order_events_redelivered_create_one_settlement`.
- `dispute_escalates_after_seller_deadline`; `dispute_resolution_refunds_via_orders`.
- `tax_report_contains_each_seller_once_and_matches_ledger`.
- `authority_request_export_is_audited_and_scoped`.
- `inbound_shipment_receipt_updates_inventory_once`.
- Rò rỉ: `tenant_isolation`; `seller_cannot_see_other_seller_settlements_orders_disputes` (hai nút
  seller anh em).

## Tiêu chí xong

- [ ] Sáu mục của bước 46 có commit và test; gate đầy đủ xanh trên cả hai database.
- [ ] Mỗi đồng của đơn sàn giải thích được bằng bút toán; báo cáo đối soát người bán khớp sổ cái
      (test).
- [ ] Không có đường nào chi tiền ngoài `PayoutGateway`; không lưu số tài khoản ngân hàng dạng rõ.
- [ ] Quyết định trong `docs/DECISIONS.md`: bảng bút toán sàn; căn cứ NĐ 117/2025 (tỉ lệ khấu trừ là dữ
      liệu của `tax`); cách duyệt sản phẩm (`ListingGate`); tranh chấp và hạn xử lý.
- [ ] `docs/openapi.json`, `.env.example`, CHANGELOG, ROADMAP; phát hành theo `docs/RELEASING.md`.

## Chỗ dễ sai

- **Tiền âm của người bán**: hoàn sau hoàn tất hay phạt làm người bán nợ sàn; không chi âm, mang sang
  lệnh sau (`marketplace_adjustments`), báo người bán. Không trừ vào tiền của người mua.
- **Đếm hai lần quyết toán** khi hai worker dựng lệnh chi: khoá các dòng quyết toán theo `id`
  (`rows::lock`) và đổi `status` có điều kiện `WHERE status = 'releasable'`.
- **Phí có hiệu lực chồng nhau**: kiểm khi thêm (không cho hai dòng cùng `fee_kind` + `category_id` +
  `seller_type` phủ cùng ngày); tra phí theo `placed_at` của đơn, không theo ngày tính.
- **Tách đơn và giảm giá cấp đơn**: mã giảm của sàn áp cho cả giỏ phải chia xuống từng đơn bằng
  `Money::allocate` theo net từng dòng, và ghi phần sàn chịu (`subsidy`) riêng với phần người bán chịu.
- **Phí vận chuyển** khi tách: mỗi người bán một kiện → báo giá vận chuyển theo nhóm (đã làm ở
  `orders` qua `SellerGroup.fulfillment_org_id`).
- **Không gọi `PayoutGateway` trong transaction**; lệnh chi gửi lại phải dùng cùng `reference` để đối
  tác không chi hai lần.
- **Dữ liệu cá nhân của người bán là cá nhân / hộ**: trang công khai chỉ hiện trường luật bắt buộc;
  xuất cho cơ quan có thẩm quyền phải có `marketplace_authority_requests` và audit.
- **Kỳ báo cáo thuế** theo múi giờ Việt Nam (UTC+7), không theo UTC — một đơn lúc 23:30 ngày cuối quý
  theo giờ Việt Nam thuộc quý đó; lưu UTC, cắt kỳ theo múi giờ của khách.

## Căn cứ pháp lý

- Luật Thương mại điện tử 122/2025/QH15 (từ 01/7/2026), Điều 17: xác thực danh tính người bán trước
  khi cho bán (có thể qua VNeID); nghĩa vụ của nền tảng trung gian: quy chế, khiếu nại, gỡ hàng vi
  phạm, cung cấp thông tin — nghị định hướng dẫn (các nguồn nêu NĐ 248/2026) **cần đối chiếu văn bản gốc**.
- NĐ 52/2013 sửa bởi NĐ 85/2021 và văn bản thay thế: sàn đăng ký với Bộ Công Thương, quy chế hoạt
  động, thông tin người bán trên trang sản phẩm.
- NĐ 117/2025/NĐ-CP (từ 01/7/2025): sàn có chức năng thanh toán khấu trừ, nộp thay thuế GTGT và TNCN
  của hộ, cá nhân kinh doanh; sàn không có chức năng thanh toán cung cấp thông tin người bán và doanh
  thu cho cơ quan thuế theo kỳ. Tỉ lệ (các nguồn nêu GTGT 1% và TNCN 0,5% với hàng hoá), mẫu báo cáo,
  hạn nộp — **cần đối chiếu văn bản gốc** và NĐ 68/2026 về hộ kinh doanh; tỉ lệ nằm trong gói dữ liệu
  "vn" của `tax`, không viết trong `marketplace`.
- NĐ 52/2024/NĐ-CP (thanh toán không dùng tiền mặt): thu hộ, chi hộ cần giấy phép — tiền chỉ qua đối
  tác (D-187); cần đối chiếu Điều 3, 22 và Thông tư 40/2024/TT-NHNN.
- Luật 19/2023/QH15 + NĐ 55/2024: trách nhiệm của nền tảng số trung gian trong giải quyết khiếu nại
  của người tiêu dùng.
- Luật Kế toán 88/2015 Điều 41: quyết toán, lệnh chi, báo cáo thuế lưu ít nhất 10 năm.

## Agent làm bước này tự quyết

- Giữ tiền tới đâu: khuyến nghị `delivered_at + return_window_days` (chính sách của người bán) và
  thêm điều kiện "không tranh chấp mở"; ghi quyết định.
- Mức phí mặc định khi khách chưa nhập: khuyến nghị không có phí (0) — sàn tự nhập.
- Cách tính điểm người bán: khuyến nghị quy tắc là dữ liệu (`marketplace_penalty_rules`), mặc định
  rỗng.
- Định dạng file báo cáo thuế (CSV / XML theo mẫu cơ quan thuế): làm CSV trung lập trước, thêm định
  dạng mẫu khi có văn bản gốc; ghi quyết định.
- Có cho người bán tự chọn lịch chi trả không: khuyến nghị không — lịch của sàn, giống nhau cho mọi
  người bán.

## Bổ sung theo đối chiếu thiết kế 2 (D-208…D-218)

> Nguồn: `docs/prompts/ui-trace-roadmap.md`; mã `A-…` / `B-…` trỏ tới `docs/research/ui-trace/`.

- **Bút toán và tạm giữ (D-218).** Bảng bút toán của đặc tả này viết lại theo tên tài khoản của 36 khi làm;
  `marketplace_settlements.hold_until` bỏ, trạng thái giữ đọc từ `ledger_holds` (`Ledger::hold_status`).
- **COD (D-216, B-b1, B-c1).** Người bán chọn `cod_remittance`; sàn không nhận tiền COD vào tài khoản của
  mình — chế độ không hỗ trợ trả 422 `marketplace.cod_flow_unsupported`; với `seller_direct`, phí sàn và khấu
  trừ thay thu bằng hoá đơn phí định kỳ hay công nợ của người bán.
- **Người bán công khai, đánh giá theo người bán, theo dõi gian hàng (B-b8, B-c4).** `GET /public/sellers`
  (lọc, phân trang), `GET /public/sellers/{slug}/reviews` (gom từ đánh giá của `orders`), `POST/DELETE
  /me/followed-sellers/{slug}`; bảng `marketplace_followers` (`tenant_id`, `seller_org_id`, `user_id`,
  `created_at`); báo hàng mới của gian hàng qua `notifications` loại `Marketing` — cần đồng ý riêng.
- **Huy hiệu tin cậy (D-214, B-b11).** Chỉ chỉ số có dữ liệu thật; không "tỉ lệ trả lời tin nhắn".

## Bổ sung theo D-222 (ai trả tiền giảm giá)

- **Quyết toán theo `funding`.** Phần giảm người bán trả trừ vào doanh thu của người bán. Phần sàn trả
  (`platform`): người bán vẫn nhận như không có khoản giảm đó; sàn ghi chi phí khuyến mại (tài khoản
  `expense.promotion.platform` trong `ledger`, đặt tên theo D-218). Phần nhãn hàng / đối tác thanh toán trả: khoản
  phải thu của sàn từ bên đó.
- **Gốc tính phí sàn** là dữ liệu có ngày hiệu lực theo sàn (`fee_base`: `after_seller_discount` mặc định — giá
  sau khoản giảm người bán trả, trước khoản giảm sàn trả); bảng đối soát cho người bán hiện từng phần.
- **Voucher sàn và voucher người bán.** Người bán tạo chương trình của mình trong phạm vi nút `seller`; sàn tạo
  chương trình toàn sàn, có thể chia `funding` với người bán (người bán đồng ý qua `promotions_participations`,
  không áp tự động). Người bán đăng ký flash sale / ngày hội của sàn bằng `promotions.participate`.
- **Trần pháp luật** cộng khoản giảm của sàn và của người bán trên một đơn vị (đặc tả 43 mục 11 — cần đối chiếu).
- **Test:** `quyet_toan_voucher_san_nguoi_ban_nhan_du`, `quyet_toan_voucher_shop_tru_doanh_thu`,
  `phi_san_tinh_tren_gia_sau_giam_cua_nguoi_ban`, `chia_funding_sai_bi_tu_choi`.
