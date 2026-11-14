# Bước 59 — Bán sỉ B2B (`wholesale`)

> Lộ trình: `docs/ROADMAP.md` bước 59 · Quyết định: D-221, D-232, D-233 · Đặc tả liên quan: 36 (công nợ, hoá
> đơn), 41 (bảng giá theo nhóm khách), 45 (đơn hàng), 47 (khách hàng), 37 (văn bản pháp lý, KYC — nơi làm phần
> ký điện tử), 46 (sàn, người bán), `modules/webhooks` (gửi chứng từ EDI ra, D-233)

## Mục tiêu

Bán cho doanh nghiệp trên cùng hệ thống bán lẻ: tài khoản công ty có nhiều người mua, giá sỉ và số lượng tối
thiểu, báo giá có hạn, đặt hàng trả chậm (công nợ) trong hạn mức tín dụng, hoá đơn GTGT ghi đúng mã số thuế
người mua, duyệt đơn nội bộ phía công ty mua. Thêm (D-232): hợp đồng khung có cam kết số lượng / giá trị và đơn
gọi hàng theo hợp đồng; mời thầu nhiều người bán khi khách là sàn, giá niêm phong tới hạn; ký hợp đồng điện tử
trong `compliance`, dùng chung cho hợp đồng khung, hợp đồng người bán (46) và thoả thuận tiếp thị liên kết (49).
Thêm (D-233): trao đổi chứng từ B2B tự động (EDI) theo chuẩn mở UBL 2.1 — hệ thống của công ty mua gửi đơn
vào, hệ thống nhận trả lời đơn, báo gửi hàng và tham chiếu hoá đơn ra, không ai phải gõ lại.

## Bắt đầu khi

Bước 45 (đơn hàng) và 47 (khách hàng) xong; dùng sổ cái và hoá đơn của 36, bảng giá theo nhóm khách của 41.
Phần ký điện tử cần 37 (`compliance`); phần mời thầu cần 46 (`marketplace`) (D-232). Phần EDI dùng `webhooks`
(sẵn có) để gửi ra và `invoices` (36) cho tham chiếu hoá đơn (D-233).

## Phạm vi

Làm: công ty mua (tài khoản B2B), người mua thuộc công ty và quyền của họ, bảng giá sỉ, bậc giá theo số
lượng, số lượng tối thiểu / bội số, báo giá (yêu cầu → báo → chấp nhận thành đơn), điều khoản thanh toán
(trả trước / trả chậm N ngày), hạn mức tín dụng, tuổi nợ và nhắc nợ, duyệt đơn phía người mua, đặt hàng nhanh
theo SKU / CSV. Hợp đồng khung và đơn gọi hàng; mời thầu nhiều người bán trên sàn (RFQ); ký hợp đồng điện tử
trong module `compliance`: trait `SignatureProvider` ở kernel, bộ giả trong testkit, migration mới của
`compliance` (D-232). EDI theo UBL 2.1 (OASIS): nhận `Order`, trả và gửi `OrderResponse`, gửi `DespatchAdvice` và
tham chiếu hoá đơn; hồ sơ đối tác, nhật ký chứng từ; gửi ra qua endpoint loại `direct` mới của `webhooks` (D-233).

Không làm: đấu thầu mua sắm công (Luật Đấu thầu 22/2023/QH15 — RFQ ở đây chỉ là mua bán giữa doanh nghiệp tư),
EDIFACT, ANSI X12 — bộ chuyển đổi là module riêng, làm khi có khách cần và có tài liệu / tài khoản thử → dừng
lại hỏi chủ dự án (D-233); truyền qua mạng Peppol (AS4) hay AS2 — EDI ở đây vào bằng HTTPS + API key, ra bằng
webhook có ký (D-233). Bộ nối nhà cung cấp chữ ký số thật (VNPT-CA, Viettel-CA, FPT-CA…; DocuSign cho quốc tế) là
module riêng, cần tài khoản và secret → dừng lại hỏi chủ dự án (D-232). Core không dựng PDF: tệp cần ký do
người dùng tải lên hay hệ thống khác tạo (D-232).

## Thiết kế

- **Công ty mua** (`wholesale_companies`: `tenant_id`, `org_id` của bên bán, `name`, `tax_id` (kiểm như hồ sơ
  thuế bước 36), địa chỉ hoá đơn và giao hàng (`Address` của bước 34), `price_list_id`, `payment_terms`
  (`prepaid` / `net_7` / `net_15` / `net_30` / `net_45`), `credit_limit_minor` + mã tiền tệ, `status`).
  Thành viên `wholesale_company_members` (`user_id` tài khoản người mua của bước 34, vai trò trong công ty:
  `buyer`, `approver`, `admin`, ngưỡng tự đặt không cần duyệt).
- **Giá:** bảng giá sỉ là bảng giá của `catalog` (bước 41) gắn cho công ty; thêm bậc giá theo số lượng
  (`catalog_price_tiers`) và `min_qty`, `qty_step` theo biến thể — `catalog` khai, `wholesale` chỉ chọn bảng giá.
  Giá sỉ **không** hiện công khai; chỉ người mua thuộc công ty đã duyệt thấy.
- **Báo giá:** `wholesale_quotes` (`number` đánh số của kernel, dòng, giá đề nghị, `valid_until`, `status`
  `requested/sent/accepted/expired/rejected/converted`). Chấp nhận → tạo đơn qua `OrderIntake` của 45 với giá đã
  chốt (không tính lại giá công khai); hết hạn → job đánh dấu. Báo giá do trao thầu tạo ra có `source =
  wholesale.rfq:<id>` (D-232).
- **Công nợ:** đơn `net_*` không thanh toán ngay; khi giao (hay khi lập hoá đơn, theo `invoice_timing`) ghi
  `receivable.customer[C]` trong sổ cái (36); hạn = ngày hoá đơn + số ngày. Kiểm hạn mức **trong transaction đặt
  đơn** bằng `UPDATE … SET used = used + x WHERE used + x <= limit` (không đọc rồi ghi). Thu tiền qua `ledger`
  (chuyển khoản / VietQR khớp tự động) trừ công nợ. Báo cáo tuổi nợ (0–30–60–90+), nhắc nợ qua `notifications`.
  Quá hạn quá N ngày → chặn đơn trả chậm mới (chính sách theo nút).
- **Duyệt phía người mua:** đơn vượt ngưỡng của người mua → `pending_buyer_approval`, người `approver` của công
  ty duyệt (cùng cơ chế duyệt của bước 51 P12, phạm vi công ty).
- **Đặt nhanh:** `POST /me/wholesale/quick-order {lines: [{sku, qty}]}` hay CSV qua `imports` (bước 56).

### Hợp đồng khung B2B (D-232)

**Hợp đồng.** `wholesale_contracts` giữa một công ty mua và một nút bán (`org_id`): số `number` (đánh số của
kernel), kỳ hiệu lực `starts_at`–`ends_at` (UTC), tiền tệ, cách định giá `pricing`: `price_list` (bảng giá của
`catalog` gắn riêng cho hợp đồng) hay `fixed` (giá cố định theo biến thể ở `wholesale_contract_lines`); cam kết
tổng giá trị (`committed_value_minor`) và / hoặc theo biến thể (`committed_qty`); `overage_policy` cho phần vượt
cam kết: `block` (chặn) hay `list_price` (phần vượt tính giá thường của công ty — bảng giá sỉ, bậc giá như đơn sỉ
thường); `payment_terms` của hợp đồng (ghi đè điều khoản của công ty cho đơn theo hợp đồng); tham chiếu văn bản
đã ký (`signature_request_id`, `signed_sha256`). `min_qty` / `qty_step` của `catalog` vẫn áp (D-232).

**Trạng thái** (D-232):

| Từ | Sự việc | Sang |
|---|---|---|
| — | tạo (tay, gia hạn, hay trao thầu) | `draft` (chỉ trạng thái này sửa được dòng, giá, cam kết) |
| `draft` | `send-for-signature` gọi `SignatureRequests::create` của `compliance` | `pending_signature` |
| `pending_signature` | `compliance.signature_completed` (subject `wholesale.contract:<id>`) | `active` |
| `pending_signature` | `compliance.signature_declined` / `…_expired` / `…_cancelled` | `draft` |
| `draft` | `mark-signed` — hợp đồng ký trên giấy: tải bản quét, lưu khoá tệp + SHA-256, audit | `active` |
| `active` | tới `ends_at` (job) | `expired` |
| `active` | `terminate` có lý do | `terminated` |

Nghe event của `compliance` lọc theo tiền tố subject `wholesale.contract:`; gọi lại vô hại (đổi trạng thái bằng
`UPDATE … WHERE status = 'pending_signature'`). Đã `active` thì không sửa; đổi điều khoản = hợp đồng mới.

**Đơn gọi hàng (call-off).** `POST /me/wholesale/contracts/{id}/call-offs {lines: [{variant_id, qty}], …}` với
`Idempotency-Key`, người gọi là thành viên công ty có `wholesale.buy`. Hợp đồng phải `active` và `now` trong
[`starts_at`, `ends_at`). Một transaction ghi của `wholesale` (`begin_write` + `write_with_retry`):

1. Chèn `wholesale_call_offs` (`reserved`, `reserved_until = now + AUTOXIDE_WHOLESALE_CALL_OFF_HOLD_MINUTES`).
2. Trừ cam kết từng dòng hợp đồng **theo `id` tăng dần** (tránh deadlock), rồi tổng giá trị trên
   `wholesale_contracts`: `UPDATE wholesale_contract_lines SET remaining_qty = remaining_qty - :x WHERE
   tenant_id = :t AND id = :id AND remaining_qty >= :x` qua `rows::adjust` — không đọc rồi ghi. Trả 0 dòng:
   `block` → rollback, `wholesale.commitment_exceeded` (kèm `remaining`); `list_price` → `rows::lock_one` dòng
   đó (khoá độc quyền, cả hai database), lấy hết phần còn lại theo giá hợp đồng, phần vượt tính giá thường; ghi
   tách hai phần trong dòng call-off.
3. Kiểm hạn mức tín dụng (đơn `net_*`) như mục "Công nợ", cùng transaction.
4. `Outbox::record(wholesale.call_off_reserved)` → commit.

Sau commit, **ngoài transaction**, gọi `OrderIntake::import` (`channel = "wholesale"`, `external_ref` = id call-off,
giá đã chốt từng phần) → call-off `placed` + `order_id`. Gửi lại cùng `external_ref` trả `Replayed` nên không tạo
hai đơn. Lỗi tạm thời → job `wholesale.place_call_off` thử lại tới `reserved_until`; lỗi vĩnh viễn hay quá hạn →
transaction trả cam kết (`UPDATE … + x`) và hạn mức, call-off `released`. Đơn bị huỷ (`orders.order_cancelled`,
`channel = wholesale`) → trả cam kết đúng một lần (`UPDATE wholesale_call_offs SET status = 'cancelled' WHERE
status = 'placed'` — 0 dòng thì bỏ qua) (D-232).

**Báo cáo thực hiện.** `GET /contracts/{id}/performance`: theo dòng — cam kết, đã gọi (`committed −
remaining`), phần trăm; tổng giá trị đã gọi so với cam kết; số đơn gọi hàng; số ngày còn lại. Chỉ đọc bảng của
`wholesale` (D-232).

**Nhắc hết hạn và gia hạn.** Job hằng ngày `wholesale.contract_reminders`: hợp đồng `active` có `ends_at` trong
`reminder_days` (cột, mặc định 30) và `expiry_reminded_at` NULL → event `wholesale.contract_expiring`, thông báo
(tin giao dịch qua `notifications`) cho `admin` của công ty và người phụ trách nút bán, ghi `expiry_reminded_at`;
cùng job chuyển hợp đồng quá `ends_at` sang `expired`. Gia hạn: `POST /contracts/{id}/renew {starts_at, ends_at}`
→ hợp đồng `draft` **mới** có `previous_contract_id` = hợp đồng cũ, chép dòng và cam kết (còn lại = cam kết mới);
hợp đồng cũ không đổi (D-232).

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `wholesale_contracts` | `id`, `tenant_id`, `org_id` (nút bán), `company_id`, `number`, `title`, `status` (`draft/pending_signature/active/expired/terminated`), `starts_at`, `ends_at`, `currency`, `pricing` (`price_list`/`fixed`), `price_list_id` NULL, `committed_value_minor` NULL, `remaining_value_minor` NULL, `overage_policy` (`block`/`list_price`), `payment_terms`, `reminder_days`, `expiry_reminded_at` NULL, `previous_contract_id` NULL, `source` (`''` / `wholesale.rfq:<id>`), `signature_request_id` NULL, `signed_file_key` NULL, `signed_sha256` NULL, `signed_at` NULL, `terminated_at` NULL, `termination_reason` NULL, `created_by`, `created_at`, `updated_at` | unique (`tenant_id`,`number`); index (`tenant_id`,`company_id`,`status`,`id`), (`tenant_id`,`org_id`,`status`,`id`), (`tenant_id`,`status`,`ends_at`) |
| `wholesale_contract_lines` | `id`, `tenant_id`, `contract_id`, `variant_id`, `unit_price_minor` NULL (NULL = giá từ bảng giá của hợp đồng), `committed_qty` NULL, `remaining_qty` NULL, `created_at` | unique (`tenant_id`,`contract_id`,`variant_id`) |
| `wholesale_call_offs` | `id`, `tenant_id`, `contract_id`, `company_id`, `placed_by`, `status` (`reserved/placed/released/cancelled`), `lines` JSON (biến thể, số lượng theo giá hợp đồng, số lượng vượt, đơn giá từng phần), `contract_value_minor`, `order_id` NULL, `reserved_until`, `released_at` NULL, `reference` (`Idempotency-Key`), `created_at`, `updated_at` | unique (`tenant_id`,`reference`); index (`tenant_id`,`contract_id`,`id`), (`tenant_id`,`status`,`reserved_until`), (`tenant_id`,`order_id`) |

Tổng giá trị cam kết nằm trên `wholesale_contracts.remaining_value_minor`, trừ bằng cùng kiểu `UPDATE … WHERE
remaining_value_minor >= :x`; giá trị tính trên giá hợp đồng của phần trong cam kết (D-232).

### Mời thầu nhiều người bán trên sàn — RFQ (D-232)

**Chỉ khi khách là sàn.** Khách bật module `marketplace` (`Entitlements::module_enabled`) và tính năng gói
`wholesale.rfq`; không thì `wholesale.rfq_marketplace_only`. Đơn vị bán lẻ (không phải sàn) dùng luồng báo giá
ở trên, không có RFQ nhiều người bán. Công ty mua đăng ký ở nút sàn. **Không dùng cho mua sắm công**: mua sắm
dùng vốn nhà nước theo Luật Đấu thầu 22/2023/QH15 — cần đối chiếu văn bản gốc; tài liệu và mô tả OpenAPI ghi rõ
đây là quy trình mua bán giữa doanh nghiệp tư.

**Tạo và công bố.** Thành viên công ty (`wholesale.buy`, vai trò `admin` hay `approver`) tạo `wholesale_rfqs`
`draft`: dòng hàng (mô tả, `category_id`, số lượng, đơn vị, quy cách), hạn nộp `submission_deadline` (UTC, sau
`now` ít nhất 1 giờ), điều kiện giao (`Address` giao, giao trước ngày, ghi chú), cách mời: `invited` (danh sách
người bán) hay `open` (mọi người bán đủ điều kiện ngành hàng của mọi dòng: `Sellers::ensure_can_sell` và giấy tờ
ngành hàng `approved` — hỏi qua `api` của `marketplace`). `publish` → `open`, báo người bán được mời / đủ điều
kiện. Sau khi công bố không sửa dòng, hạn; muốn đổi thì huỷ và tạo lại (công bằng cho mọi người bán).

**Nộp hồ sơ niêm phong.** Nhân sự nút người bán có `wholesale.bids.submit`: `PUT /rfqs/{id}/bids/me {lines:
[{rfq_line_id, unit_price_minor, quantity, lead_time_days, variant_id?}], valid_until, note}` — mỗi người bán một
hồ sơ, sửa = ghi đè (`version` + 1), `DELETE` = rút (nộp lại được trước hạn). Mỗi lệnh trong một transaction:
`rows::lock_one_shared` dòng RFQ → `status = open` **và** `Utc::now()` (lấy SAU khi có khoá, giờ máy chủ UTC) <
`submission_deadline`, không thì `wholesale.rfq_deadline_passed`. Dòng giá và ghi chú lưu mã hoá trong
`sealed_lines` (`seal` của kernel theo khách, dữ liệu gắn kèm = `rfq_id` + `seller_org_id`). Trước khi mở:
không REST, tool MCP, event, báo cáo (38), bản xuất, log nào trả giá; người mua và nhân sự sàn chỉ thấy số hồ sơ
đã nộp (đếm bằng index); người bán chỉ đọc hồ sơ của mình; đọc giá người khác → `wholesale.bids_sealed`.

**Mở cùng lúc.** Job `wholesale.open_bids` (mỗi phút; `unique_key` `wholesale.open_bids:<rfq_id>` — mỗi RFQ một
lần) lấy RFQ `open` có hạn ≤ `now`: `rows::lock_one` (độc quyền — chờ mọi lệnh nộp đang giữ khoá chia sẻ) →
giải mã mọi hồ sơ `submitted` vào `wholesale_bid_lines`, xoá `sealed_lines`, ghi `opened_at` → RFQ
`evaluating` → event `wholesale.rfq_bids_opened`, báo người mua. Hồ sơ đã commit trước khoá thì có mặt khi mở;
lệnh nộp sau khoá thấy `evaluating` và bị từ chối.

**So sánh và trao thầu.** `GET /me/wholesale/rfqs/{id}/comparison` (chỉ sau `opened_at`): bảng dòng × người bán,
tổng, thời gian giao, hạn hiệu lực. `POST /me/wholesale/rfqs/{id}/award {bid_id, as: "quote" | "contract",
contract?: {starts_at, ends_at, overage_policy}}` (vai trò `approver` / `admin`), một transaction: RFQ `awarded`,
hồ sơ thắng `won`, các hồ sơ khác `lost`; tạo `wholesale_quotes` (`org_id` = nút người bán, `status =
accepted`, giá từ hồ sơ, `valid_until` = hạn hiệu lực của hồ sơ, `source = wholesale.rfq:<id>`) hay
`wholesale_contracts` (`draft`, `pricing = fixed`, dòng từ hồ sơ). Mỗi dòng của hồ sơ thắng phải có `variant_id`
(không thì `wholesale.bid_variant_missing`); hồ sơ quá `valid_until` → `wholesale.bid_expired`. Event
`wholesale.rfq_awarded` → báo người thắng và **mọi người không thắng** (tin giao dịch). Huỷ RFQ (`cancel` có lý
do, trước hay sau khi mở) → báo mọi người bán đã nộp.

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `wholesale_rfqs` | `id`, `tenant_id`, `org_id` (nút sàn), `company_id`, `number`, `title`, `status` (`draft/open/evaluating/awarded/cancelled`), `submission_deadline`, `delivery_terms` JSON (`Address` giao, `deliver_by`, ghi chú), `invitation` (`invited`/`open`), `currency`, `opened_at` NULL, `awarded_bid_id` NULL, `award_ref` NULL (`wholesale.quote:<id>` / `wholesale.contract:<id>`), `cancel_reason` NULL, `created_by`, `created_at`, `updated_at` | unique (`tenant_id`,`number`); index (`tenant_id`,`status`,`submission_deadline`), (`tenant_id`,`company_id`,`id`) |
| `wholesale_rfq_lines` | `id`, `tenant_id`, `rfq_id`, `position`, `description`, `category_id` NULL, `quantity`, `unit`, `specs` | index (`tenant_id`,`rfq_id`,`position`) |
| `wholesale_rfq_invitations` | `tenant_id`, `rfq_id`, `seller_org_id`, `invited_at`, `notified_at` NULL | PK (`tenant_id`,`rfq_id`,`seller_org_id`); index (`tenant_id`,`seller_org_id`,`rfq_id`) |
| `wholesale_bids` | `id`, `tenant_id`, `rfq_id`, `seller_org_id`, `status` (`submitted/withdrawn/won/lost`), `version`, `sealed_lines` NULL (AEAD; NULL sau khi mở), `key_version`, `total_minor` NULL (điền khi mở), `valid_until`, `submitted_by`, `submitted_at`, `withdrawn_at` NULL, `opened_at` NULL, `updated_at` | unique (`tenant_id`,`rfq_id`,`seller_org_id`); index (`tenant_id`,`rfq_id`,`status`), (`tenant_id`,`seller_org_id`,`id`) |
| `wholesale_bid_lines` | `tenant_id`, `bid_id`, `rfq_line_id`, `variant_id` NULL, `unit_price_minor`, `quantity`, `lead_time_days`, `note` | PK (`tenant_id`,`bid_id`,`rfq_line_id`); chỉ có sau khi mở |

### Ký hợp đồng điện tử — làm trong module `compliance` (D-232)

`compliance` sở hữu văn bản pháp lý nên giữ luồng ký. Bước này thêm vào `compliance` (đã đăng ký trước
`wholesale`, `marketplace`, `affiliate`) một migration **mới** `m0004_signatures` (có `down`; không sửa
migration đã commit của 37). Module khác gọi qua `api` của `compliance`, không đọc bảng `compliance_…`.

**Kernel và testkit** (phác thảo — tra API thật bằng skill `check-library-api`):

```rust
// crates/kernel/src/signature.rs — như identity.rs: trait trung lập; bộ nối thật là module riêng đăng ký TRƯỚC
// compliance, gọi `context.contribute::<dyn SignatureProvider>(…)` (nhiều nhà cung cấp cùng lúc, như PaymentGateway).
pub struct SignerSpec { pub signer_id: Uuid, pub name: String, pub email: Option<String>,
    pub phone: Option<PhoneNumber>, pub position: u16 }
pub struct SignatureEnvelope { pub tenant: TenantId, pub request_id: Uuid /* nhà cung cấp chống trùng */,
    pub title: String, pub document: Vec<u8>, pub media_type: String, pub sha256: String,
    pub signers: Vec<SignerSpec>, pub expires_at: DateTime<Utc>, pub locale: String }
pub enum SignerOutcome { Pending, Signed { at: DateTime<Utc>, certificate_serial: Option<String>,
    certificate_issuer: Option<String> }, Declined { reason: String } }
pub struct EnvelopeStatus { pub provider_reference: String, pub event_id: String,
    pub signers: Vec<(Uuid, SignerOutcome)>, pub completed: bool }
#[async_trait::async_trait]
pub trait SignatureProvider: Send + Sync {
    fn provider(&self) -> &'static str;                  // "vnpt_ca", "fake"…
    async fn send(&self, envelope: SignatureEnvelope) -> Result<String /* provider_reference */, Error>;
    async fn status(&self, tenant: TenantId, provider_reference: &str) -> Result<EnvelopeStatus, Error>;
    /// Tệp đã ký (chữ ký số nhúng trong tệp) — chỉ khi `completed`.
    async fn signed_document(&self, tenant: TenantId, provider_reference: &str) -> Result<Vec<u8>, Error>;
    async fn cancel(&self, tenant: TenantId, provider_reference: &str) -> Result<(), Error>;
    /// So sánh hằng thời gian, cửa sổ chống phát lại; sai → `signature_callback_invalid` (401).
    fn verify_webhook(&self, tenant: TenantId, headers: &http::HeaderMap, body: &[u8]) -> Result<EnvelopeStatus, Error>;
}
```

Testkit: `FakeSignatureProvider` (`crates/testkit/src/signature.rs`) — giữ trạng thái trong bộ nhớ, không gọi
mạng; test điều khiển "người ký đã ký / từ chối" và sinh webhook có chữ ký. Không có nhà cung cấp → phương thức
`digital` trả `signature_disabled` (503); `simple` vẫn chạy.

**`autoxide_compliance::api`** (phác thảo):

```rust
pub enum SignatureMethod { Simple, Digital }
pub enum SignerIdentity { User(Uuid), Email(String), Phone(PhoneNumber) }   // đã xác thực: OTP gửi tới đó
pub struct NewSigner { pub identity: SignerIdentity, pub name: String, pub party_label: String /* "Bên mua" */,
    pub position: u16 /* cùng số = ký song song */ }
pub struct NewSignatureRequest { pub subject: String /* "wholesale.contract:<id>" */, pub org_id: Option<OrgId>,
    pub title: String, pub file_key: String /* khoá Storage module gọi đã kiểm quyền */, pub method: SignatureMethod,
    pub provider: Option<String>, pub signers: Vec<NewSigner>, pub expires_at: DateTime<Utc>, pub reference: String }
#[async_trait] pub trait SignatureRequests: Send + Sync {
    /// Idempotent theo `reference`. Chép tệp sang khoá riêng của compliance, tính SHA-256, mời người ký lượt đầu.
    async fn create(&self, tenant: &TenantContext, actor: Actor, request: NewSignatureRequest) -> Result<Submitted<SignatureSummary>, Error>;
    async fn get(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<SignatureSummary>, Error>;
    async fn cancel(&self, tenant: &TenantContext, actor: Actor, id: Uuid, reason: &str) -> Result<(), Error>;
}
```

**Tạo.** Kiểm người ký (≥ 1, `position` ≥ 1), tệp (`application/pdf`, tối đa
`AUTOXIDE_COMPLIANCE_SIGNATURE_MAX_FILE_MB`), hạn ký (mặc định `now + AUTOXIDE_COMPLIANCE_SIGNATURE_DEFAULT_DAYS`).
**Ngoài transaction**: đọc tệp từ `Storage`, tính SHA-256, ghi bản chép vào `compliance/signatures/<tenant>/<id>`
(module gọi không sửa được bản này). Rồi một transaction: chèn yêu cầu `sent` + người ký + outbox
`compliance.signature_requested`. `digital`: job `compliance.signature_send` gọi `SignatureProvider::send` ngoài
transaction, ghi `provider_reference`. `simple`: job `compliance.signature_notify` gửi link ký cho lượt có
`position` nhỏ nhất chưa ký (mail qua `dyn Mailer`, SMS qua `dyn SmsSender`; người ký có tài khoản thấy thêm ở
`/me/signatures`). Link mang token HMAC (tenant + id người ký, hết hạn cùng yêu cầu), như link rút đồng ý của 37.

**Ký `simple`** (chữ ký điện tử: OTP + bấm đồng ý). Người ký mở link → xem tiêu đề, các bên, tải tệp, `sha256` →
`POST …/otp` (OTP gửi tới email / số đã xác thực của người ký; với tài khoản: số của
`AccountVerification::verified_phone` hay email tài khoản; cùng cơ chế băm + pepper, số lần, dùng một lần như
`compliance_phone_challenges`) → `POST …/sign {code, agree: true, sha256}`. Trước transaction: đọc lại bản chép,
băm lại; khác mã băm đã lưu → `compliance.signature_file_changed` và yêu cầu chuyển `cancelled` (lý do
`file_changed`); `sha256` client gửi (bản người ký đã xem) khác mã băm đã lưu → cùng lỗi, chỉ từ chối lần ký đó
(một người ký không huỷ được cả yêu cầu). Trong transaction: `rows::lock_one` dòng yêu cầu → trạng thái
`sent` / `in_progress`; `now < expires_at` (không thì `compliance.signature_expired`); người ký còn `pending`;
mọi người ký có `position` nhỏ hơn đã `signed` (không thì `compliance.signature_out_of_order`); OTP đúng và tiêu
một lần → ghi bằng chứng: thời điểm, IP băm (`ip_fingerprint`), `user_agent`, kênh OTP, mã băm tệp. Người ký cuối
→ `completed`: `signed_file_key` = bản chép, `signed_sha256` = mã băm tệp, `evidence` JSON gom bằng chứng mọi người
ký và `evidence_sha256`; event `compliance.signature_completed {request_id, subject, signed_sha256}`.

**Ký `digital`** (chữ ký số qua nhà cung cấp được cấp phép). Nhà cung cấp xác thực người ký. Webhook
`POST /signatures/callbacks/{provider}` (công khai) → `verify_webhook` → chèn `compliance_signature_callbacks`
(unique theo `provider_event_id`: trùng → 204, không ghi lại) → job `compliance.signature_process` cập nhật người
ký (kết quả trái thứ tự bị bỏ, ghi cảnh báo). Hoàn tất → ngoài transaction tải `signed_document`, lưu
`compliance/signatures/<tenant>/<id>/signed` + SHA-256 → `completed` + event. Job `compliance.signature_poll`
(backoff) gọi `status` khi webhook không tới.

**Từ chối, hết hạn, huỷ, kiểm.** Người ký `decline {reason}` → `declined`. Lịch giờ `compliance.signature_expiry`
→ `expired` (gọi `cancel` của nhà cung cấp nếu `digital`). `GET /signature-requests/{id}/verify`: đọc lại tệp gốc và
tệp đã ký, băm lại, trả `{status, file_sha256, file_intact, signed_sha256, signed_intact, evidence_sha256,
signers: [{party_label, position, method, signed_at, certificate_serial}]}` — không trả email / số.

**Ai dùng.** Hợp đồng khung (mục trên). Hợp đồng người bán trên sàn (46) và thoả thuận tiếp thị liên kết (49):
**nâng cấp tuỳ chọn** từ "chấp nhận văn bản" của 37 — khách đặt `compliance_documents.signature_method`
(`none` mặc định / `simple` / `digital`); phiên bản của văn bản cần ký phải có tệp (`file_key`, `file_sha256` khi
phát hành); `POST /me/acceptances` cho văn bản đó → `compliance.signature_required`; thay bằng
`POST /me/acceptances/sign {version_id}` tạo yêu cầu ký (subject `compliance.document_version:<id>`), ký xong ghi
`compliance_acceptances` (`method = signature`, `signature_request_id`) và `AcceptanceGate` mới coi là đã chấp nhận.

| Bảng (cấp khách hàng, `m0004_signatures`) | Cột chính | Khoá / index |
|---|---|---|
| `compliance_signature_requests` | `id`, `tenant_id`, `org_key`, `subject` (`<module>.<loại>:<id>`), `title`, `file_key` (bản chép), `file_sha256`, `media_type`, `size`, `method` (`simple`/`digital`), `provider` (`''` với `simple`), `provider_reference` (= `id` tới khi nhà cung cấp trả mã), `status` (`sent/in_progress/completed/declined/expired/cancelled`), `expires_at`, `signed_file_key` NULL, `signed_sha256` NULL, `evidence` JSON NULL, `evidence_sha256` NULL, `completed_at` NULL, `cancel_reason` NULL, `reference`, `created_by`, `created_at`, `updated_at` | unique (`tenant_id`,`reference`); unique (`tenant_id`,`provider`,`provider_reference`); index (`tenant_id`,`subject`), (`tenant_id`,`status`,`expires_at`), (`tenant_id`,`org_key`,`id`) |
| `compliance_signature_signers` | `id`, `tenant_id`, `request_id`, `position`, `party_label`, `name`, `identity_kind` (`user`/`email`/`phone`), `user_id` NULL, `contact` NULL (email / số E.164; NULL sau khi xoá theo yêu cầu), `contact_key` (`tenancy::contact_key`), `status` (`pending/notified/signed/declined`), `notified_at` NULL, `signed_at` NULL, `ip_fingerprint` NULL, `user_agent` NULL (≤ 300 ký tự), `otp_channel` NULL, `certificate_serial` NULL, `certificate_issuer` NULL, `decline_reason` NULL, `updated_at` | index (`tenant_id`,`request_id`,`position`), (`tenant_id`,`user_id`,`status`), (`tenant_id`,`contact_key`) |
| `compliance_signature_challenges` | `id`, `tenant_id`, `signer_id`, `channel` (`email`/`sms`), `code_hash`, `attempts`, `expires_at`, `consumed_at` NULL, `created_at` | index (`tenant_id`,`signer_id`,`created_at`) |
| `compliance_signature_callbacks` | `id`, `tenant_id`, `provider`, `provider_event_id`, `received_at`, `body` (nguyên văn, đã kiểm chữ ký), `processed_at` NULL, `request_id` NULL | unique (`tenant_id`,`provider`,`provider_event_id`); index (`tenant_id`,`received_at`) |
| Cột thêm | `compliance_documents.signature_method` (`none`), `compliance_document_versions.file_key` NULL + `file_sha256` NULL (chỉ đặt lúc phát hành — bảng vẫn không UPDATE), `compliance_acceptances.signature_request_id` NULL | |

`compliance` khai `Module::retention` cho tệp và bằng chứng ký (thời hạn lưu là dữ liệu — cần đối chiếu).

### Trao đổi chứng từ B2B tự động — EDI theo UBL 2.1 (D-233)

**Chuẩn.** Theo chuẩn mở, không tự chế: **UBL 2.1** của OASIS (cũng là ISO/IEC 19845:2015), dạng XML. Bước này
làm bốn loại: `Order` (vào), `OrderResponse` (trả đồng bộ và gửi ra), `DespatchAdvice` (ra) và tham chiếu hoá
đơn (ra). Hoá đơn GTGT hợp lệ vẫn là hoá đơn điện tử theo định dạng của cơ quan thuế do `invoices` (36) phát
hành — EDI **không** phát hành UBL `Invoice` thay nó; "tham chiếu hoá đơn" là UBL 2.1 `AttachedDocument` mang ký
hiệu, số, ngày, mã của cơ quan thuế, mã tra cứu và (tuỳ hồ sơ đối tác) đính kèm XML hoá đơn đã cấp mã. Mã trả lời
theo danh mục mà Peppol BIS Ordering 3 dùng (`OrderResponseCode` UNCL 4343: `AB` đã nhận, `AP` chấp nhận, `CA`
chấp nhận có thay đổi, `RE` từ chối; trạng thái dòng UNCL 1229) — kiểm lại danh mục khi làm. EDIFACT / ANSI X12
là bộ chuyển đổi riêng (UBL ⇄ định dạng kia), làm khi có khách cần (D-233).

**Hồ sơ đối tác** (`wholesale_edi_partners`, một hồ sơ cho mỗi cặp công ty mua – nút bán). Nhân sự bên bán
(`wholesale.edi.manage`) tạo: công ty, định dạng `ubl-2.1`, định danh — mã số thuế lấy từ
`wholesale_companies.tax_id`, GLN (GS1, 13 số, kiểm số kiểm tra) của bên mua và bên bán nếu có —, địa điểm giao
(`delivery_locations`: mã địa điểm của đối tác hay GLN → `Address`), endpoint nhận chứng từ ra (`outbound_url`),
loại chứng từ muốn nhận, có đính kèm XML hoá đơn không. `outbound_url` thành một endpoint loại `direct` của
`webhooks` (xem dưới); secret ký trả **một lần** lúc tạo / đổi secret, không lưu ở `wholesale`. Hồ sơ mới ở
`pending_key`: `admin` của công ty mua tạo API key loại `service` (bước 51 P12) có `wholesale.buy` rồi gọi
`PUT /me/wholesale/edi/key {partner_id}` **bằng chính key đó** → ghi `inbound_api_key_id` = `api_key_id` của
principal, hồ sơ `active`; một key chỉ gắn một hồ sơ (đã gắn chỗ khác → `wholesale.edi_key_in_use`). Cách này chứng
minh công ty giữ key mà `wholesale` không phải đọc bảng của `auth` (D-046). Tạm khoá (`suspended`) → chứng từ vào
bị từ chối, không gửi chứng từ ra.

**Vào: `POST /api/v1/wholesale/edi/orders`** (`Content-Type: application/xml`, thân là UBL 2.1 `Order`):

1. **Xác thực:** principal phải là API key `ActorKind::Service`, có `wholesale.buy`, là `inbound_api_key_id` của
   một hồ sơ `active`, người sở hữu key là thành viên công ty; công ty và nút bán lấy từ **hồ sơ**, không từ XML.
   Sai → `wholesale.edi_service_key_required` / `wholesale.edi_key_not_bound` (403).
2. **Trước khi đọc XML:** `Content-Type` khác `application/xml` → 415; thân quá
   `AUTOXIDE_WHOLESALE_EDI_MAX_BODY_KB` → 413 (đọc có giới hạn, không đọc hết rồi mới đo).
3. **Đọc XML an toàn** (ngoài transaction): gặp `<!DOCTYPE` (khai DTD, entity nội hay ngoài) → từ chối ngay
   `wholesale.edi_xml_forbidden` — không bao giờ mở DTD, không giải entity ngoài, không tải gì qua mạng hay đọc
   file (chặn XXE, "billion laughs"); giới hạn độ sâu 32 tầng và số thuộc tính mỗi phần tử. XML hỏng →
   `wholesale.edi_xml_malformed` kèm dòng, cột.
4. **Kiểm theo schema UBL:** namespace `urn:oasis:names:specification:ubl:schema:xsd:Order-2`, `UBLVersionID` =
   `2.1`, phần tử bắt buộc, số lần xuất hiện, kiểu (ngày, số lượng, mã tiền tệ ISO 4217, `unitCode` UN/ECE Rec 20)
   theo **hồ sơ con** của Order mà bước này hỗ trợ — viết thành kiểu Rust có test so với tệp mẫu chính thức của
   OASIS (không có trình kiểm XSD thuần Rust đủ dùng; không thêm thư viện C). Gom **mọi** lỗi, báo một lần (D-083):
   `wholesale.edi_schema_invalid` (422), `errors[]` mỗi lỗi có `line`, `column`, `path` (XPath), `order_line_id`
   (ID dòng đơn nếu lỗi nằm trong một dòng), `code`, `message`.
5. **Đối chiếu bên:** `BuyerCustomerParty` / `SellerSupplierParty` (mã số thuế ở `PartyTaxScheme/CompanyID` hay
   GLN ở `EndpointID` / `PartyIdentification/ID` `schemeID="0088"`) phải khớp hồ sơ → không thì
   `wholesale.edi_party_mismatch`. `DocumentCurrencyCode` phải là tiền tệ của công ty / hợp đồng.
6. **Chống trùng** theo ID chứng từ của đối tác (`Order/ID`) trong **phạm vi hồ sơ đối tác**: transaction ngắn
   chèn `wholesale_edi_messages` (`in`, `order`, `dedupe_key` = `Order/ID`, `received`, `content_sha256`,
   `processing_until` = now + `AUTOXIDE_WHOLESALE_EDI_PROCESSING_LEASE_SECONDS`). Đụng unique → đọc dòng cũ: cùng mã
   băm và đã `processed` → trả lại đúng OrderResponse đã lưu (200, header `Idempotent-Replayed: true`); cùng mã băm
   và còn trong hạn `processing_until` → `wholesale.edi_document_in_progress` (409, `Retry-After`); cùng mã băm, quá
   hạn (lần trước chết giữa chừng) → giành lượt bằng `UPDATE … SET processing_until = :mới WHERE id = :id AND
   processing_until < :now` (0 dòng → 409 như trên) rồi xử lý tiếp (bước 7 idempotent nên không tạo đơn thứ hai);
   khác mã băm →
   `wholesale.edi_document_reused` (422, như `idempotency_key_reused`). Chứng từ bị từ chối ở bước 3–5 ghi nhật
   ký với `dedupe_key` = `id` của dòng, **không giữ** ID của đối tác — đối tác sửa rồi gửi lại cùng ID được nhận.
7. **Tạo đơn qua cùng service như REST** (luật kiến trúc 8), ngoài transaction trên: có `Order/Contract/ID` = số
   hợp đồng `active` của công ty → service đơn gọi hàng (call-off, `reference` = `wholesale.edi:<message_id>`);
   không có → service đặt nhanh (`quick-order`, cùng `reference`). Dòng nhận diện biến thể bằng
   `SellersItemIdentification/ID` (SKU) hay `StandardItemIdentification/ID` `schemeID="0160"` (GTIN). Luật nghiệp
   vụ theo **từng dòng**: biến thể không có (`wholesale.edi_item_unknown`), đơn vị không khớp
   (`wholesale.edi_unit_mismatch`), `min_qty` / `qty_step` (`wholesale.min_qty`), không trong hợp đồng
   (`wholesale.variant_not_in_contract`), vượt cam kết (`wholesale.commitment_exceeded`), giá người mua ghi khác giá
   của bên bán (`wholesale.edi_price_mismatch` — giá **luôn** theo bảng giá / hợp đồng của bên bán, giá trong XML
   chỉ để đối chiếu). Dòng hỏng bị từ chối, dòng đúng thành đơn; hạn mức tín dụng, duyệt phía người mua, giữ cam
   kết áp y như REST. Địa điểm giao: `Delivery/DeliveryLocation/ID` tra trong `delivery_locations`; không có phần
   tử → địa chỉ giao mặc định của công ty; mã lạ → `wholesale.edi_delivery_location_unknown` (từ chối cả chứng từ
   — không giao tới địa chỉ tự do trong XML).
8. **Trả lời đồng bộ:** dựng UBL `OrderResponse` (`OrderReference` = `Order/ID`): mọi dòng nhận → `AB` (đã nhận,
   trả lời nghiệp vụ gửi sau khi đơn được xác nhận); một phần dòng bị từ chối → `CA` kèm `OrderLine` từ chối và mã
   lý do; mọi dòng bị từ chối → `RE`, không tạo đơn. Lưu nội dung vào `Storage`, rồi transaction ghi dòng nhật ký
   `processed` (`response_code`, `line_results`, `order_id` / `call_off_id`, `response_key`) + outbox
   `wholesale.edi_message_received` → commit → 201 + `Location: /api/v1/wholesale/edi/messages/{id}` (có đơn) hay
   200 (`RE`), thân là XML `OrderResponse`.

Lỗi xảy ra **trước khi có chứng từ hợp lệ** (bước 1–6) trả `Problem` (RFC 9457) như mọi endpoint. Thân XML ở bước
8 là chứng từ, cùng loại ngoại lệ với tải tệp (`GET …/einvoice?format=xml` của 36, nguồn cấp của 48): OpenAPI khai
`requestBody` và response `200` / `201` với content type `application/xml` (schema chuỗi, kèm ví dụ) (D-233).

**Ra: OrderResponse, DespatchAdvice, tham chiếu hoá đơn.** `wholesale` nghe event, chỉ xử lý đơn có dòng nhật ký
`in` của một hồ sơ `active` có bật loại chứng từ đó:

| Event | Chứng từ ra | `dedupe_key` |
|---|---|---|
| `orders.order_confirmed` | `OrderResponse` `AP` (đúng như đơn) hay `CA` (thiếu hàng `stock_shortage`, dòng đã từ chối) | `order_response:<order_id>` |
| `orders.order_cancelled` khi chưa gửi `OrderResponse` nghiệp vụ (duyệt bị từ chối, quá hạn trả) | `OrderResponse` `RE` | `order_response:<order_id>` (cùng khoá — mỗi đơn **đúng một** trả lời nghiệp vụ) |
| `orders.order_status_changed` sang `shipping` | `DespatchAdvice` (dòng, số lượng của đơn; mã vận đơn nếu có) | `despatch_advice:<order_id>` |
| `invoices.document_accepted` có nguồn là đơn đó | `AttachedDocument` tham chiếu hoá đơn | `invoice_reference:<document_id>` |

Một transaction: chèn dòng nhật ký `out` `queued` (unique trên `dedupe_key` ⇒ event giao lại không tạo bản thứ
hai) + job `wholesale.edi_send` (`unique_key` = id dòng) → commit. Job, **ngoài transaction**: đọc đơn / chứng từ
qua `api` (`Orders::find`, `Invoices::document`), dựng XML, lưu vào `Storage`, băm, rồi gọi
`DirectDeliveries::send` của `webhooks` với `delivery_id` = id dòng nhật ký → `sent`. `webhooks` ký (Standard
Webhooks, như mọi webhook), thử lại có backoff, hết lượt vào dead-letter queue (`autoxide jobs retry`), và phát
`webhooks.direct_delivered` / `webhooks.direct_failed` (subject `wholesale.edi_message:<id>`) → `wholesale` đổi
dòng sang `delivered` / `failed` bằng `UPDATE … WHERE status = 'sent'` (giao lại vô hại). Gửi lại tay:
`POST /edi/messages/{id}/resend` → `DirectDeliveries::redeliver` (cùng nội dung, cùng `delivery_id`).

**Phần thêm vào `webhooks`** (migration **mới** có `down`; không sửa migration đã commit). Hôm nay `webhooks` chỉ
chuyển event tới endpoint quản trị viên khai; EDI cần gửi **một thân do module khác dựng** tới một endpoint riêng:

```rust
// autoxide_webhooks::api (phác thảo — tra API thật bằng skill `check-library-api`)
pub struct NewDirectEndpoint { pub owner: &'static str /* "wholesale" */, pub url: String, pub description: String }
pub struct DirectEndpoint { pub id: Uuid, pub secret: Option<String> /* chỉ có lúc tạo / đổi secret */ }
pub struct DirectDelivery { pub endpoint_id: Uuid, pub delivery_id: Uuid /* chống trùng */,
    pub subject: String /* "wholesale.edi_message:<id>" */, pub content_type: String, pub body: String }
#[async_trait] pub trait DirectDeliveries: Send + Sync {
    async fn create_endpoint(&self, tenant: &TenantContext, endpoint: NewDirectEndpoint) -> Result<DirectEndpoint, Error>;
    async fn update_endpoint(&self, tenant: &TenantContext, id: Uuid, url: Option<String>, disabled: Option<bool>) -> Result<(), Error>;
    async fn rotate_secret(&self, tenant: &TenantContext, id: Uuid) -> Result<DirectEndpoint, Error>;
    /// Idempotent theo (`endpoint_id`, `delivery_id`): ghi lần gửi + job trong transaction của `webhooks`.
    async fn send(&self, tenant: &TenantContext, delivery: DirectDelivery) -> Result<(), Error>;
    async fn redeliver(&self, tenant: &TenantContext, endpoint_id: Uuid, delivery_id: Uuid) -> Result<(), Error>;
}
```

`webhooks_endpoint` thêm `kind` (`events` mặc định / `direct`) và `owner` (`''` / tên module); `webhooks_delivery`
thêm `content_type` (mặc định `application/json`) và `subject` NULL; với `direct`, cột `event_id` giữ
`delivery_id` (unique (`tenant_id`, `endpoint_id`, `event_id`) sẵn có chống trùng). `Fanout` **bỏ qua** endpoint
`direct`; `/api/v1/webhooks` liệt kê chúng nhưng không cho sửa (`webhooks.endpoint_owned`) — sửa qua module sở hữu.
URL đi qua cùng bộ chặn SSRF (`autoxide_kernel::net`). Event mới của `webhooks`: `webhooks.direct_delivered`,
`webhooks.direct_failed` (payload: `endpoint_id`, `delivery_id`, `subject`, `attempts`, mã HTTP cuối — không thân).

**Nhật ký chứng từ** (`wholesale_edi_messages`): mọi chứng từ vào / ra, kể cả bị từ chối. Nội dung (XML gốc, XML
trả lời) nằm trong `Storage` ở `wholesale/edi/<tenant>/<id>.xml`, bảng chỉ giữ khoá, kích thước và SHA-256.
`retain_until` = lúc tạo + `AUTOXIDE_WHOLESALE_EDI_RETENTION_DAYS`; job hằng ngày `wholesale.edi_purge` xoá tệp quá
hạn, đặt `content_key` / `response_key` = NULL, **giữ** dòng và mã băm (vẫn chứng minh được đã nhận gì). Khai
`Module::retention` (`wholesale.edi_documents`, `Accounting`, theo biến cấu hình — thời hạn cần đối chiếu Luật Kế
toán).

**Thư viện XML.** Ít phụ thuộc nhất có thể: một crate thuần Rust đọc và ghi XML (ứng viên: `quick-xml` — không mở
DTD, không giải entity ngoài; hay `roxmltree` chỉ đọc, `allow_dtd = false` mặc định, ghi tự viết). Tra đúng phiên
bản bằng skill `check-library-api`, kiểm bằng test XXE (không tin mô tả thư viện), ghi `STACK.md` (D-233).

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `wholesale_edi_partners` | `id`, `tenant_id`, `org_id` (nút bán), `company_id`, `format` (`ubl-2.1`), `status` (`pending_key/active/suspended`), `buyer_gln` NULL, `seller_gln` NULL, `delivery_locations` JSON (mã / GLN → `Address`), `inbound_api_key_id` NULL, `outbound_endpoint_id` NULL (endpoint `direct` của `webhooks`), `outbound_url` NULL (để hiển thị), `outbound_documents` JSON (`order_response`, `despatch_advice`, `invoice_reference`), `attach_invoice_xml` BOOL, `created_by`, `created_at`, `updated_at` | unique (`tenant_id`,`company_id`,`org_id`); index (`tenant_id`,`inbound_api_key_id`), (`tenant_id`,`org_id`,`status`,`id`) |
| `wholesale_edi_messages` | `id`, `tenant_id`, `org_id`, `partner_id`, `company_id`, `direction` (`in`/`out`), `doc_type` (`order`/`order_response`/`despatch_advice`/`invoice_reference`), `document_id` (ID trong chứng từ; `''` khi đọc không ra), `dedupe_key`, `in_reply_to` NULL (id dòng `in`), `status` (vào: `received/processed/rejected`; ra: `queued/sent/delivered/failed`), `response_code` NULL (`AB/AP/CA/RE`), `line_results` JSON NULL (ID dòng, kết quả, mã lý do), `errors` JSON NULL (dòng, cột, path, mã), `order_id` NULL, `call_off_id` NULL, `source_ref` NULL (`orders.order:<id>` / `invoices.document:<id>`), `content_key` NULL, `response_key` NULL, `content_sha256`, `size`, `processing_until` NULL, `retain_until`, `created_at`, `updated_at` | unique (`tenant_id`,`partner_id`,`direction`,`doc_type`,`dedupe_key`); index (`tenant_id`,`partner_id`,`id`), (`tenant_id`,`order_id`), (`tenant_id`,`org_id`,`status`,`id`), (`tenant_id`,`retain_until`) |

### API, MCP, quyền, lỗi

- REST `/api/v1/wholesale`: `companies` (CRUD, duyệt đăng ký), `members`, `quotes` (+ `send`, `accept`, `convert`),
  `credit` (tuổi nợ, điều chỉnh hạn mức có lý do và audit), `/me/wholesale/…` cho người mua.
- MCP: `list_quotes`, `send_quote`, `credit_report` (`read_only`), `adjust_credit_limit` (rủi ro — chờ duyệt
  với key agent).
- Permission: `wholesale.companies.manage`, `wholesale.quotes.manage`, `wholesale.credit.manage`,
  `wholesale.buy` (người mua). Tính năng gói `wholesale`.
- Lỗi: `wholesale.credit_limit_exceeded` (409, kèm còn lại), `wholesale.overdue_block` (409),
  `wholesale.min_qty` (422), `wholesale.quote_expired` (409), `wholesale.not_company_member` (403).

**REST thêm cho hợp đồng khung và RFQ** (`/api/v1/wholesale`, D-232):

| Method + path | Permission | Ghi chú |
|---|---|---|
| `GET/POST /contracts`, `GET/PATCH /contracts/{id}`, `PUT /contracts/{id}/lines` | `wholesale.contracts.read` / `.manage` (theo nút bán) | sửa chỉ khi `draft`; `If-Match` |
| `POST /contracts/{id}/send-for-signature` | `wholesale.contracts.manage` | `{file_id, method, signers[]}` → `pending_signature` |
| `POST /contracts/{id}/mark-signed` | `wholesale.contracts.manage` | `{file_id, signed_on}` — ký trên giấy; audit |
| `POST /contracts/{id}/terminate` · `/renew` | `wholesale.contracts.manage` | lý do bắt buộc khi chấm dứt; `renew` → 201 hợp đồng mới |
| `GET /contracts/{id}/performance` | `wholesale.contracts.read` | thực hiện so với cam kết |
| `GET /me/wholesale/contracts`, `GET /me/wholesale/contracts/{id}`, `…/performance` | `wholesale.buy` | hợp đồng của công ty mình |
| `POST /me/wholesale/contracts/{id}/call-offs`, `GET …/call-offs` | `wholesale.buy` | `Idempotency-Key` bắt buộc; 201 |
| `GET/POST /me/wholesale/rfqs`, `GET/PATCH /me/wholesale/rfqs/{id}` | `wholesale.buy` (`admin`/`approver` của công ty) | sửa chỉ khi `draft` |
| `POST /me/wholesale/rfqs/{id}/publish` · `/cancel` · `/award` | như trên | `award {bid_id, as, contract?}` |
| `GET /me/wholesale/rfqs/{id}/comparison` | như trên | chỉ sau khi mở; trước đó `wholesale.bids_sealed` |
| `GET /rfqs` (người bán: RFQ được mời / đủ điều kiện), `GET /rfqs/{id}` | `wholesale.bids.submit` (theo nút người bán) | không có giá của ai |
| `PUT /rfqs/{id}/bids/me`, `GET /rfqs/{id}/bids/me`, `DELETE /rfqs/{id}/bids/me` | `wholesale.bids.submit` | nộp / sửa / rút trước hạn |
| `GET /platform/rfqs`, `POST /platform/rfqs/{id}/cancel` | `wholesale.rfqs.manage` (nhân sự sàn) | số hồ sơ, không giá trước khi mở |

**REST thêm ở `compliance`** (`/api/v1/compliance`, D-232):

| Method + path | Permission | Ghi chú |
|---|---|---|
| `GET/POST /signature-requests`, `GET /signature-requests/{id}`, `POST /signature-requests/{id}/cancel` | `compliance.signatures.read` / `.manage` | ký văn bản bất kỳ của khách; module khác dùng `api` |
| `GET /signature-requests/{id}/verify` | `compliance.signatures.read` | băm lại tệp, trả trạng thái |
| `GET /me/signatures`, `POST /me/signatures/{signer_id}/otp` · `/sign` · `/decline` | đăng nhập, là người ký | `compliance.signer_mismatch` |
| `GET /public/signatures/{token}`, `POST /public/signatures/{token}/otp` · `/sign` · `/decline` | token ký | người ký không có tài khoản; throttle theo IP |
| `POST /signatures/callbacks/{provider}` | công khai, chữ ký | 204; trùng → 204 không ghi lại; lỗi `signature_callback_invalid` |
| `POST /me/acceptances/sign` | đăng nhập | `{version_id}` cho văn bản có `signature_method ≠ none` |

**Tool MCP thêm** (cùng service, cùng permission): `wholesale.list_contracts`, `get_contract_performance`
(`read_only`), `place_call_off` (lệnh ghi, `Idempotency-Key`), `list_rfqs` (`read_only`, không giá trước khi mở);
`compliance.list_signature_requests`, `verify_signature_request` (`read_only`). **Không** có tool nộp hồ sơ thầu,
trao thầu, ký (ký cần OTP của chính người ký) (D-232).

**Event thêm** (payload chỉ id, trạng thái — **không giá thầu**, không email / số): `wholesale.contract_created`,
`wholesale.contract_status_changed` (`from`, `to`), `wholesale.contract_expiring`, `wholesale.call_off_reserved`,
`wholesale.call_off_placed`, `wholesale.call_off_released`, `wholesale.rfq_published`, `wholesale.bid_submitted`
(`rfq_id`, `seller_org_id`, `version`), `wholesale.bid_withdrawn`, `wholesale.rfq_bids_opened`,
`wholesale.rfq_awarded` (`bid_id` thắng, `award_ref`), `wholesale.rfq_cancelled`. `compliance`:
`compliance.signature_requested`, `…signature_signed` (`request_id`, `signer_id`, `position`),
`…signature_completed`, `…signature_declined`, `…signature_expired`, `…signature_cancelled`. `wholesale` nghe
`compliance.signature_*` (subject `wholesale.contract:`) và `orders.order_cancelled` (D-232).

**Job thêm:** `wholesale.place_call_off` (`unique_key` = id call-off), `wholesale.release_call_offs` (mỗi phút:
`reserved` quá `reserved_until` → trả cam kết), `wholesale.contract_reminders` (hằng ngày; `unique_key`
`wholesale.contract_reminders:<ngày>`), `wholesale.open_bids` (mỗi phút), `wholesale.rfq_notify` (báo người bán
khi công bố, trao thầu, huỷ). `compliance`: `compliance.signature_send`, `compliance.signature_notify`,
`compliance.signature_process`, `compliance.signature_poll`, lịch giờ `compliance.signature_expiry` (D-232).

**Cấu hình thêm:** `AUTOXIDE_WHOLESALE_CALL_OFF_HOLD_MINUTES` (15); `AUTOXIDE_COMPLIANCE_SIGNATURE_URL` (trang
giao diện ký có `{token}`), `AUTOXIDE_COMPLIANCE_SIGNATURE_DEFAULT_DAYS` (14),
`AUTOXIDE_COMPLIANCE_SIGNATURE_MAX_FILE_MB` (20). OTP dùng lại `AUTOXIDE_COMPLIANCE_OTP_*` của 37 (D-232).

**Permission thêm:** `wholesale.contracts.read`, `wholesale.contracts.manage` (nhân sự bên bán, `scoped`),
`wholesale.bids.submit` (người bán trên sàn, `scoped` theo nút người bán), `wholesale.rfqs.manage` (nhân sự sàn);
`compliance.signatures.read`, `compliance.signatures.manage`. Tính năng gói: `wholesale.contracts`,
`wholesale.rfq` (D-232).

**Mã lỗi thêm** (EN ở chỗ tạo lỗi; VI trong `locales/vi.json` của crate, D-232):

| Mã | HTTP | Tiếng Việt |
|---|---|---|
| `wholesale.contract_not_found` | 404 | Không có hợp đồng này. |
| `wholesale.contract_state_invalid` | 409 | Hợp đồng đang ở trạng thái {status}, không làm được việc này. |
| `wholesale.contract_not_in_effect` | 409 | Hợp đồng chỉ có hiệu lực từ {starts_at} đến {ends_at}. |
| `wholesale.commitment_exceeded` | 409 | Vượt khối lượng còn lại của hợp đồng: còn {remaining}. |
| `wholesale.variant_not_in_contract` | 422 | Sản phẩm {variant} không có trong hợp đồng. |
| `wholesale.rfq_marketplace_only` | 403 | Mời thầu nhiều người bán chỉ có trên sàn; hãy dùng yêu cầu báo giá. |
| `wholesale.rfq_not_found` | 404 | Không có yêu cầu mời thầu này. |
| `wholesale.rfq_state_invalid` | 409 | Yêu cầu mời thầu đang ở trạng thái {status}, không làm được việc này. |
| `wholesale.rfq_deadline_invalid` | 422 | Hạn nộp hồ sơ phải sau thời điểm hiện tại ít nhất 1 giờ. |
| `wholesale.rfq_deadline_passed` | 409 | Đã qua hạn nộp hồ sơ ({deadline} UTC). |
| `wholesale.seller_not_eligible` | 403 | Người bán không được mời hoặc chưa đủ điều kiện ngành hàng của yêu cầu này. |
| `wholesale.bids_sealed` | 403 | Hồ sơ dự thầu còn niêm phong tới {deadline} UTC. |
| `wholesale.bid_expired` | 409 | Hồ sơ dự thầu đã hết hiệu lực lúc {valid_until}. |
| `wholesale.bid_variant_missing` | 422 | Người bán chưa gắn sản phẩm cho dòng {line}; chưa trao thầu được. |
| `compliance.signature_not_found` | 404 | Không có yêu cầu ký này. |
| `compliance.signature_state_invalid` | 409 | Yêu cầu ký đang ở trạng thái {status}, không làm được việc này. |
| `compliance.signature_file_changed` | 409 | Tệp đã thay đổi sau khi gửi ký (mã băm không khớp); yêu cầu ký đã huỷ, hãy gửi ký lại. |
| `compliance.signature_out_of_order` | 409 | Chưa tới lượt bạn ký: còn chờ người ký trước. |
| `compliance.signature_expired` | 409 | Yêu cầu ký đã hết hạn lúc {expires_at}. |
| `compliance.signer_mismatch` | 403 | Bạn không phải người ký của yêu cầu này. |
| `compliance.signature_required` | 409 | Văn bản {document} cần ký điện tử, không chấp nhận bằng đánh dấu đồng ý. |
| `compliance.signature_file_invalid` | 422 | Tệp cần ký phải là PDF, tối đa {max_mb} MB. |
| `compliance.signers_invalid` | 422 | Danh sách người ký không hợp lệ: {detail}. |
| `signature_disabled` (kernel) | 503 | Chưa cấu hình nhà cung cấp chữ ký số. |
| `signature_callback_invalid` (kernel) | 401 | Chữ ký callback của nhà cung cấp chữ ký số không hợp lệ. |

OTP sai / quá số lần dùng lại `compliance.otp_invalid`, `compliance.otp_attempts_exceeded` của 37.

**REST thêm cho EDI** (`/api/v1/wholesale`, D-233):

| Method + path | Permission | Ghi chú |
|---|---|---|
| `POST /edi/orders` | API key loại `service` đã gắn hồ sơ `active`, `wholesale.buy` | thân UBL 2.1 `Order` (`application/xml`); trả XML `OrderResponse`: 201 + `Location` khi có đơn, 200 khi `RE` hay gửi lại; lỗi trước khi có chứng từ hợp lệ là `Problem` |
| `GET/POST /edi/partners`, `GET/PATCH /edi/partners/{id}` | `wholesale.edi.read` / `.manage` (theo nút bán) | `outbound_url` → endpoint `direct` của `webhooks`; secret trả một lần; `If-Match` |
| `POST /edi/partners/{id}/rotate-secret` · `/suspend` · `/activate` | `wholesale.edi.manage` | |
| `GET /edi/messages`, `GET /edi/messages/{id}` | `wholesale.edi.read` | lọc `partner_id`, `direction`, `doc_type`, `status`; `line_results`, `errors` |
| `GET /edi/messages/{id}/content` · `/response` | `wholesale.edi.read` | XML gốc / XML trả lời; quá hạn lưu → `wholesale.edi_content_expired` |
| `POST /edi/messages/{id}/resend` | `wholesale.edi.manage` | chỉ chứng từ `out` ở `failed` / `delivered` |
| `PUT /me/wholesale/edi/key` | `wholesale.buy`, vai trò `admin` của công ty, gọi **bằng chính** API key `service` | `{partner_id}` — gắn key vào hồ sơ |
| `GET /me/wholesale/edi/partners`, `GET /me/wholesale/edi/messages` | `wholesale.buy` (`admin`) | hồ sơ và nhật ký của công ty mình |

**Tool MCP thêm cho EDI** (cùng service, cùng permission): `wholesale.list_edi_messages`, `get_edi_message`
(`read_only`, không trả nội dung XML), `resend_edi_message` (lệnh ghi). Không có tool nhận chứng từ vào — agent đặt
hàng bằng REST / tool đặt hàng sẵn có (D-233).

**Event thêm cho EDI** (payload chỉ id, loại, trạng thái, mã trả lời — không nội dung chứng từ):
`wholesale.edi_partner_changed`, `wholesale.edi_message_received` (`message_id`, `response_code`, `order_id`),
`wholesale.edi_message_rejected`, `wholesale.edi_message_queued`, `wholesale.edi_message_delivered`,
`wholesale.edi_message_failed`. `wholesale` nghe thêm `orders.order_confirmed`, `orders.order_status_changed`,
`invoices.document_accepted`, `webhooks.direct_delivered`, `webhooks.direct_failed` (subject
`wholesale.edi_message:`); `orders.order_cancelled` đã nghe (D-233).

**Job thêm cho EDI:** `wholesale.edi_send` (`unique_key` = id dòng nhật ký), `wholesale.edi_purge` (hằng ngày;
`unique_key` `wholesale.edi_purge:<ngày>`) (D-233).

**Cấu hình thêm cho EDI:** `AUTOXIDE_WHOLESALE_EDI_MAX_BODY_KB` (1024), `AUTOXIDE_WHOLESALE_EDI_RETENTION_DAYS`
(1825), `AUTOXIDE_WHOLESALE_EDI_PROCESSING_LEASE_SECONDS` (60) (D-233).

**Permission thêm cho EDI:** `wholesale.edi.read`, `wholesale.edi.manage` (nhân sự bên bán, `scoped`). Bên mua dùng
`wholesale.buy` như REST. Tính năng gói: `wholesale.edi` (D-233).

**Mã lỗi thêm cho EDI** (EN ở chỗ tạo lỗi; VI trong `locales/vi.json` của crate; mã có HTTP "—" chỉ xuất hiện trong
kết quả từng dòng của `OrderResponse` và `line_results`, D-233):

| Mã | HTTP | Tiếng Việt |
|---|---|---|
| `wholesale.edi_service_key_required` | 403 | Chứng từ EDI phải gửi bằng API key loại `service`. |
| `wholesale.edi_key_not_bound` | 403 | API key này chưa gắn với hồ sơ đối tác EDI nào đang hoạt động. |
| `wholesale.edi_key_in_use` | 409 | API key này đã gắn với một hồ sơ đối tác EDI khác. |
| `wholesale.edi_partner_not_found` | 404 | Không có hồ sơ đối tác EDI này. |
| `wholesale.edi_media_type_unsupported` | 415 | Chỉ nhận chứng từ UBL 2.1 dạng XML (`Content-Type: application/xml`). |
| `wholesale.edi_body_too_large` | 413 | Chứng từ vượt {max_kb} KB. |
| `wholesale.edi_xml_forbidden` | 422 | Chứng từ có khai báo DOCTYPE / entity; không được phép. |
| `wholesale.edi_xml_malformed` | 422 | XML không hợp lệ ở dòng {line}, cột {column}: {detail}. |
| `wholesale.edi_schema_invalid` | 422 | Chứng từ không đúng UBL 2.1 {document}: {count} lỗi, xem `errors`. |
| `wholesale.edi_party_mismatch` | 422 | Bên mua hoặc bên bán trong chứng từ không khớp hồ sơ đối tác. |
| `wholesale.edi_delivery_location_unknown` | 422 | Không có địa điểm giao {location} trong hồ sơ đối tác. |
| `wholesale.edi_document_reused` | 422 | ID chứng từ {document_id} đã dùng cho nội dung khác; hãy dùng ID mới. |
| `wholesale.edi_document_in_progress` | 409 | Chứng từ {document_id} đang được xử lý; thử lại sau ít giây. |
| `wholesale.edi_message_not_resendable` | 409 | Chứng từ đang ở trạng thái {status}, không gửi lại được. |
| `wholesale.edi_content_expired` | 410 | Nội dung chứng từ đã hết hạn lưu ngày {retain_until}. |
| `wholesale.edi_item_unknown` | — | Không tìm thấy sản phẩm {item} của dòng {line}. |
| `wholesale.edi_unit_mismatch` | — | Đơn vị {unit} của dòng {line} không khớp đơn vị bán {expected}. |
| `wholesale.edi_price_mismatch` | — | Giá dòng {line} ({price}) khác giá bên bán ({expected}). |
| `webhooks.endpoint_owned` | 409 | Endpoint này do module {owner} quản lý; hãy sửa ở đó. |

## Thứ tự làm

1. Công ty mua, thành viên, giá sỉ, báo giá, đặt nhanh.
2. Công nợ, hạn mức, tuổi nợ, nhắc nợ, duyệt phía người mua.
3. Ký điện tử trong `compliance` (D-232): `SignatureProvider` ở kernel + `FakeSignatureProvider`, migration
   `m0004_signatures`, `api` `SignatureRequests`, ký `simple`, ký `digital` (webhook, poll), hết hạn, `verify`;
   nâng cấp chấp nhận văn bản bằng chữ ký (`signature_method`).
4. Hợp đồng khung (D-232): bảng, trạng thái, gửi ký / ký giấy, đơn gọi hàng trừ cam kết, nhả cam kết, báo cáo
   thực hiện, nhắc hết hạn, gia hạn.
5. RFQ trên sàn (D-232): tạo, công bố, mời / mở theo ngành hàng, nộp niêm phong, mở cùng lúc, so sánh, trao thầu
   thành báo giá / hợp đồng, báo người không thắng; tài liệu ghi rõ không dùng cho mua sắm công.
6. EDI (D-233): chọn thư viện XML (`check-library-api`, `STACK.md`) và test XXE trước; `webhooks` thêm endpoint
   `direct`, `DirectDeliveries`, event `direct_*` (migration mới); hồ sơ đối tác + gắn key; nhận `Order`: đọc an
   toàn, kiểm hồ sơ con UBL, đối chiếu bên, chống trùng, tạo đơn / call-off qua service sẵn có, `OrderResponse` đồng
   bộ; nhật ký + `Storage` + hạn lưu; gửi `OrderResponse` / `DespatchAdvice` / tham chiếu hoá đơn qua `webhooks`;
   OpenAPI khai `application/xml`.

## Test bắt buộc (cả hai database)

`gia_si_khong_lo_cong_khai`, `bac_gia_theo_so_luong_dung`, `bao_gia_chap_nhan_thanh_don_gia_da_chot`,
`han_muc_khong_vuot_khi_dat_dong_thoi`, `qua_han_chan_don_tra_cham`, `thu_tien_tru_cong_no`,
`don_vuot_nguong_cho_nguoi_duyet_cua_cong_ty`, `hoa_don_ghi_ma_so_thue_cong_ty`.

Hợp đồng khung (D-232): `goi_hang_dong_thoi_khong_vuot_cam_ket` (nhiều call-off cùng lúc; tổng đã trừ không
vượt cam kết, không âm), `vuot_cam_ket_chan_theo_dieu_khoan`, `vuot_cam_ket_phan_vuot_tinh_gia_thuong`,
`goi_hang_dung_gia_hop_dong`, `goi_hang_gui_lai_khong_tao_hai_don`, `huy_don_tra_cam_ket_dung_mot_lan`,
`goi_hang_ngoai_ky_hieu_luc_bi_chan`, `hop_dong_chi_active_sau_khi_ky_xong`, `nhac_het_han_khong_gui_trung`,
`gia_han_tao_hop_dong_moi_tham_chieu_cu`, `bao_cao_thuc_hien_khop_cam_ket`.

RFQ (D-232): `rfq_chi_co_khi_khach_la_san`, `gia_niem_phong_truoc_han_ke_ca_nguoi_mua` (REST, MCP, event, báo
cáo), `nop_sau_han_bi_tu_choi`, `nop_sat_han_dong_thoi_khong_lot_ho_so` (nộp đồng thời với job mở: mỗi hồ sơ hoặc
có mặt khi mở, hoặc bị từ chối — không có hồ sơ "đã nhận" mà vắng mặt), `mo_moi_ho_so_cung_luc`,
`sua_rut_truoc_han`, `nguoi_ban_khong_du_dieu_kien_nganh_hang_bi_chan`, `trao_thau_tao_bao_gia_da_chot`,
`trao_thau_tao_hop_dong_khung_nhap`, `nguoi_khong_thang_duoc_bao`.

Ký điện tử trong `compliance` (D-232): `tep_doi_sau_khi_gui_ky_bi_tu_choi`, `ky_sai_thu_tu_bi_chan`,
`webhook_trung_khong_ghi_hai_lan`, `het_han_ky_khong_ky_duoc`, `bo_gia_khong_goi_mang`, `otp_sai_khong_ky_duoc`,
`hai_nguoi_ky_cuoi_dong_thoi_hoan_tat_mot_lan`, `verify_bao_tep_bi_sua`, `van_ban_can_ky_khong_nhan_danh_dau`.

EDI (D-233): `edi_sai_schema_bao_loi_theo_dong` (thiếu phần tử, sai kiểu ở hai dòng đơn: một lần 422, mỗi lỗi có
`line`, `column`, `path`, `order_line_id`), `edi_gui_lai_cung_id_khong_tao_don_thu_hai` (gửi lại tuần tự trả đúng
`OrderResponse` cũ; **đồng thời** hai request cùng ID → đúng một đơn, bên kia nhận bản phát lại hay
`edi_document_in_progress`), `edi_cung_id_noi_dung_khac_bi_tu_choi`, `edi_xxe_bi_chan` (DOCTYPE có entity ngoài
`file://` và `http://`, entity lồng nhau: 422 `edi_xml_forbidden`, không đọc file, không có kết nối ra),
`edi_don_xac_nhan_sinh_dung_mot_order_response` (`orders.order_confirmed` giao lại hai lần, kể cả hai worker cùng
lúc → một dòng nhật ký `out`, một lần gửi của `webhooks`), `edi_tu_choi_dinh_dang_khong_giu_id` (gửi sai rồi sửa,
gửi lại cùng ID → được nhận), `edi_dong_sai_bi_tu_choi_dong_dung_thanh_don` (`CA`), `edi_moi_dong_sai_khong_tao_don`
(`RE`), `edi_gia_theo_ben_ban_khong_theo_xml`, `edi_don_theo_hop_dong_thanh_goi_hang` (trừ cam kết như REST),
`edi_key_khong_phai_service_bi_chan`, `edi_ben_mua_khong_khop_bi_tu_choi`, `edi_huy_truoc_xac_nhan_gui_re_mot_lan`,
`edi_giao_hang_gui_despatch_advice_mot_lan`, `edi_hoa_don_duoc_cap_ma_gui_tham_chieu`,
`edi_noi_dung_het_han_bi_xoa_giu_ma_bam`, `webhooks_endpoint_direct_khong_nhan_event`,
`webhooks_gui_truc_tiep_khong_trung_theo_delivery_id`.

## Chỗ dễ sai

- Trừ cam kết bằng đọc rồi ghi (đọc `remaining`, tính, ghi lại) — hai đơn cùng lúc vượt cam kết. Dùng
  `UPDATE … WHERE remaining >= x`; chỉ nhánh `list_price` mới khoá dòng rồi tách phần (D-232).
- Gọi `OrderIntake` trong transaction giữ cam kết — transaction dài, gọi xuyên module; đúng là giữ trong
  transaction của `wholesale`, tạo đơn sau commit với `external_ref`, nhả khi lỗi (D-232).
- Lấy giờ của database (`now()` / `CURRENT_TIMESTAMP` khác nhau giữa hai database) hay lấy giờ trước khi có khoá
  để kiểm hạn nộp — một lệnh chờ khoá lâu lọt qua hạn. Lấy `Utc::now()` sau khi có khoá (D-232).
- Lộ giá thầu qua đường phụ: event, audit ghi thân request, log, bản xuất, báo cáo của bước 38, thông báo "đã có
  hồ sơ mới" kèm số tiền, sắp xếp danh sách theo giá (D-232).
- Cập nhật cột đếm trên dòng RFQ khi nộp hồ sơ — nhiều người giữ khoá chia sẻ cùng nâng lên khoá ghi gây
  deadlock; đếm bằng index (D-232).
- Băm lại tệp **trong** transaction ký (đọc `Storage` là gọi mạng); phải băm trước rồi mới mở transaction (D-232).
- Cột tham gia unique để `''` cho mọi yêu cầu `simple` (`provider_reference`) — đụng unique; đặt bằng `id` của yêu
  cầu tới khi nhà cung cấp trả mã (D-232).
- Coi RFQ là "đấu thầu" theo Luật Đấu thầu, hay để khách là cơ quan nhà nước dùng cho mua sắm công (D-232).
- Để thư viện XML mở DTD / giải entity (XXE, "billion laughs"), hay đo kích thước sau khi đã đọc hết thân. Từ chối
  mọi `<!DOCTYPE`, đọc có giới hạn, giới hạn độ sâu; test chứng minh (D-233).
- Tin bên mua ghi trong XML để chọn công ty, hay dùng giá trong XML làm giá bán — công ty lấy từ API key, giá từ
  bảng giá / hợp đồng của bên bán; XML chỉ để đối chiếu (D-233).
- Chống trùng theo ID chứng từ trên toàn khách (hai đối tác trùng số đơn) hay giữ ID của chứng từ bị từ chối định
  dạng (đối tác không gửi lại được bản sửa). Khoá là (hồ sơ đối tác, hướng, loại, `dedupe_key`) (D-233).
- Đọc / ghi `Storage`, dựng XML, gọi `OrderIntake` hay `webhooks` trong transaction ghi (D-233).
- Gửi `OrderResponse` nghiệp vụ hai lần (lúc xác nhận và lúc huỷ, hay khi event giao lại) — `AP` / `CA` / `RE` dùng
  chung một `dedupe_key` theo đơn (D-233).
- Trả XML cho lỗi xác thực / XML hỏng (phải là `Problem`), hay khai OpenAPI thân `application/json` cho endpoint
  nhận XML (D-233).
- Endpoint `direct` lọt vào `Fanout` → event nội bộ đi tới đối tác (D-233).

## Căn cứ pháp lý

| Nội dung | Văn bản | Ghi chú |
|---|---|---|
| Hoá đơn GTGT cho người mua là tổ chức (tên, mã số thuế, địa chỉ) | NĐ 123/2020/NĐ-CP, sửa bởi NĐ 70/2025/NĐ-CP | Cần đối chiếu văn bản gốc |
| Hợp đồng mua bán hàng hoá, thanh toán trả chậm, hợp đồng khung | Luật Thương mại 2005, Bộ luật Dân sự 2015 | Ký điện tử làm trong `compliance` (D-232) |
| Thanh toán không dùng tiền mặt cho giao dịch từ 5 triệu đồng (khấu trừ GTGT đầu vào) | Luật Thuế GTGT 48/2024/QH15 | Cần đối chiếu ngưỡng hiện hành |
| Giá trị pháp lý của thông điệp dữ liệu, chữ ký điện tử, hợp đồng điện tử | Luật Giao dịch điện tử 20/2023/QH15 (hiệu lực 01/7/2024) | D-232 |
| Chữ ký điện tử chuyên dùng, chữ ký số, dịch vụ tin cậy | NĐ 23/2025/NĐ-CP | Cần đối chiếu số hiệu, ngày hiệu lực (D-232) |
| Loại hợp đồng bắt buộc chữ ký số hay công chứng / chứng thực | Luật Công chứng, luật chuyên ngành | Cần người có chuyên môn pháp lý; core không tự kết luận (D-232) |
| Mua sắm dùng vốn nhà nước phải theo đấu thầu | Luật Đấu thầu 22/2023/QH15 | RFQ không dùng cho mua sắm công — cần đối chiếu văn bản gốc (D-232) |
| Dữ liệu cá nhân của người ký (email, số điện thoại, IP băm) | Luật 91/2025/QH15; NĐ 356/2025 | Tối thiểu hoá, thời hạn lưu là dữ liệu — cần đối chiếu (D-232) |
| Chứng từ điện tử, thông điệp dữ liệu trao đổi giữa doanh nghiệp (EDI) | Luật Giao dịch điện tử 20/2023/QH15 | Giá trị pháp lý của đơn / trả lời đơn điện tử — cần đối chiếu văn bản gốc (D-233) |
| Hoá đơn điện tử: định dạng do cơ quan thuế quy định | NĐ 123/2020/NĐ-CP, sửa bởi NĐ 70/2025/NĐ-CP | EDI chỉ gửi tham chiếu / bản đính kèm, không thay hoá đơn (D-233) |
| Thời hạn lưu chứng từ EDI | Luật Kế toán 88/2015/QH13 Điều 41 | Mặc định 5 năm là cấu hình — cần người có chuyên môn kế toán xác nhận (D-233) |

## Agent làm bước này tự quyết

Module riêng hay mở rộng `customers`; mặc định điều khoản thanh toán; cách tính tuổi nợ. Trả hàng có cộng lại
cam kết không; phụ lục sửa đổi hợp đồng đang `active`; người mua có thấy số hồ sơ đã nộp trước hạn không; thời
gian giữ call-off (D-232). Thư viện XML; `DespatchAdvice` theo đơn hay theo từng kiện (khi `shipping` cho đọc dòng
của kiện); mã `schemeID` cho mã số thuế Việt Nam trong UBL; hồ sơ con của `Order` hỗ trợ (D-233). Ghi vào
DECISIONS.
