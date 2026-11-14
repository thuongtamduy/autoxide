# Bước 45 — Giỏ hàng, đặt hàng, đơn hàng (`orders`)

> Lộ trình: `docs/ROADMAP.md` bước 45 · Thiết kế: `docs/research/business-foundation.md` mục 3 (trụ 4),
> mục 8, mục 9 · Quyết định: D-180, D-181, D-182, D-184, D-185, D-186, D-187, D-188, D-190, D-191, D-194
> Đã gộp các quyết định: D-201, D-202, D-203, D-204, D-209, D-213, D-215, D-216, D-218, D-222, D-223,
> D-227 ; thêm thẳng vào thân: D-229, D-230, D-232 (D-228).
> Mã `A-…` / `B-…` trỏ tới `docs/research/ui-trace/` (nguồn: `docs/prompts/ui-trace-roadmap.md`); phần của
> đợt 2 có nguồn `docs/prompts/ui-phase2-api.md`, màn tương ứng ở `docs/research/ui-traceability.md`.

## Mục tiêu

Xong bước này, mọi mô hình của D-185 bán được hàng từ đầu tới cuối: người mua (có tài khoản hay vãng
lai) bỏ hàng vào giỏ, máy chủ tự tính lại giá, giảm giá, thuế, phí vận chuyển; đặt hàng là một chuỗi
**giữ chỗ có hạn theo `order_id`** (tồn kho, lượt khuyến mại, số dư nội bộ) → tạo đơn có số tuần tự →
thanh toán theo chế độ dòng tiền của khách → xác nhận; hỏng ở bước nào thì trả mọi chỗ đã giữ. Đơn đi
qua một máy trạng thái có bảng chuyển hợp lệ, mỗi chuyển ghi event và bút toán sổ cái. Có đổi trả /
hoàn tiền với bút toán đảo và hoá đơn điều chỉnh, hoá đơn điện tử, bán tại quầy (POS) cho chuỗi, đánh
giá sau mua đúng Luật 19/2023, chống bom hàng COD, trả góp qua đối tác, bảo hành, báo cáo bán hàng.

Cùng bước này: mua không cần tài khoản trọn vẹn — trạng thái thanh toán công khai, tra đơn bằng OTP, thao
tác trên đơn vãng lai, gắn đơn vào tài khoản (D-201, D-202); nhận tại cửa hàng có người nhận hộ và hạn giữ
hàng; hàng đặt trước (D-204), có trả trước một phần (D-232); coupon và voucher kèm phần ai trả tiền giảm
(D-222); trả hàng kèm ảnh và xem trước tiền hoàn (D-202); đánh giá có phiếu "hữu ích" và bản nháp tóm tắt AI
chờ duyệt (D-201, D-203).

`orders` là module nghiệp vụ (không vào `kernel`). Bước 46–49 (`marketplace`, `customers`, `channels`,
`affiliate`) cắm vào `orders` qua `orders::api` mà không sửa `orders`.

## Bắt đầu khi

- Bước 42 (`inventory`), 43 (`promotions`), 44 (`shipping`) đã merge — dùng `api` của chúng.
- Bước 37 đã có `notifications` (`dyn Notifier`) và `compliance` (văn bản pháp lý có phiên bản, xác
  thực tài khoản NĐ 147/2024, sổ căn cứ xử lý dữ liệu cá nhân).
- Sẵn từ trước: bước 33 (`TenantContext`, `tenant_id`, secret theo khách), 34 (`OrgDirectory`, quyền
  theo độ sâu `Scope`, người mua là tài khoản theo khách, `Address`), 35 (`Entitlements`: tính năng,
  hạn mức), 36 (`Ledger`, `TaxEngine`, `Invoices`, `numbering`, `PaymentGateway`, `BankFeed`/VietQR,
  ba chế độ dòng tiền), 41 (`catalog`).
- Dùng thêm: bước 51 (hộp việc P2, yêu cầu duyệt P12, cờ người dùng P14 — D-207, xuất chung P16 — D-213,
  danh sách lý do P18 — D-223); `media` (ảnh của người mua, D-202); `support` của bước 53 (tuỳ chọn,
  D-204); `TextModel` của module `ai` (tóm tắt đánh giá, D-109).
- Đọc đặc tả `docs/steps/41-…` tới `44-…` trước khi viết code: mục "Kiểu và trait" dưới đây **giả
  định** chữ ký `api` của các bước đó; tên thật lấy theo đặc tả và code đã merge, ghi chỗ lệch vào
  `docs/DECISIONS.md` khi quyết.

## Phạm vi

Crate mới `modules/orders` (`autoxide-orders`), đăng ký trong `crates/app/src/modules.rs` sau
`catalog`, `inventory`, `promotions`, `shipping`, `ledger`, `tax`, `invoices`, `compliance`,
`notifications` và trước `marketplace`, `customers`, `channels`, `affiliate`, `reporting`.

Bố cục (module lớn, nhóm theo tính năng như `auth` / `content`): `lib.rs`, `api.rs`, `config.rs`,
`permissions.rs`, `events.rs`, `entity/`, `migration/`, `http/`, `tools.rs`, và các thư mục
`cart/`, `checkout/`, `lifecycle/` (máy trạng thái), `payments/`, `returns/`, `invoicing/`, `pos/`,
`reviews/`, `risk/` (COD), `warranty/`, `reports/`, `extensions.rs`, `tests/`.

Sửa ngoài crate: `Cargo.toml` gốc (`[workspace.dependencies]`), `crates/app/Cargo.toml`,
`crates/app/src/modules.rs`, `docs/openapi.json`, `.env.example`, `CHANGELOG.md`, `docs/ROADMAP.md`.
Nếu `shipping` (44) chưa nghe event của `orders` theo tên: thêm phần đăng ký có điều kiện ở `shipping`
như `comments` làm với `content` (`OWNER_EVENTS`). Nếu `media` chưa có đường tải ảnh của người mua và
cách hỏi người tải lên: thêm vào `media` (mục "Đổi trả", D-202).

NGOÀI phạm vi: bộ nối cổng thanh toán / trả góp thật (module riêng, cần secret thật — dừng hỏi chủ dự
án); tách đơn theo người bán, phí sàn, chi trả (bước 46 — `orders` chỉ có điểm cắm); điểm, thẻ quà
tặng (47); đơn từ kênh ngoài (48, qua `OrderIntake`); hoa hồng (49); POS chạy ngoại tuyến phía client
(client lo; máy chủ chỉ chống trùng bằng `reference`); bán sỉ B2B (bước 59), sản phẩm bán theo kỳ (bước 62) — D-221;
điều khoản đặt cọc theo Bộ luật Dân sự 2015 Điều 328 cho hàng đặt trước (chính sách riêng của khách, ghi trong
điều kiện giao dịch; core chỉ có "trả trước một phần" — D-232); tạo tài khoản từ số đã xác thực (thuộc `auth`,
D-202); endpoint xuất đơn riêng (xuất qua xuất chung của bước 51, D-213).

## Thiết kế

### Luồng đặt hàng (saga giữ chỗ)

```
POST /checkout (Idempotency-Key bắt buộc)
 1. checkout_reference đã có?  → trả kết quả cũ (Replayed, 200) / 422 nếu nội dung khác
 2. tính lại báo giá ở máy chủ (catalog → promotions → shipping → tax); lệch quote_token → 409 orders.quote_changed
 3. ghi orders_checkouts {id = order_id mới, state = reserving}           (transaction ngắn, commit)
 4. Inventory::reserve(ref "orders:<order_id>", hết hạn = hạn thanh toán + grace)   ┐ ngoài transaction,
 5. Promotions::reserve(ref, …)   (nếu có mã / voucher / giảm có giới hạn lượt)     │ mỗi lệnh chạy lại
 6. Tender::hold(ref, …)          (điểm / số dư — bước 47, nếu người mua dùng)      ┘ không trùng theo ref
    hỏng ở 4–6 → release mọi chỗ đã giữ (gọi trực tiếp), checkouts.state = failed, trả lỗi của bước hỏng
 7. MỘT transaction ghi: numbering::next → orders_orders + lines + taxes + payments(pending)
    + checkouts.state = placed + Outbox(orders.order_placed) + job post_ledger/notify     (commit)
 8. ngoài transaction: PaymentGateway::create_payment / sinh VietQR / không gì (COD, tiền mặt)
    một mã QR duy nhất khi mọi nút cùng đối tác thu hộ (D-216)
    COD đủ điều kiện → chuyển ngay confirmed (cùng cách như thanh toán xong)
 9. trả 201 { orders[], payment_actions[] }   (mỗi chế độ dòng tiền một mục, D-216)
Xác nhận: event thanh toán của bước 36 → Lifecycle::apply(paid) → confirmed → orders.order_confirmed
          (inventory, promotions, tender nghe event này để xác nhận chỗ giữ)
Hết hạn:  job orders.expire_unpaid (hạn đọc từ ledger, D-218) → cancelled → orders.order_cancelled
          (bên giữ chỗ trả lại)
Treo:     job orders.recover_checkouts: checkouts ở `reserving` quá N phút → release theo ref, failed
```

Quy ước một cách làm cho mỗi việc: **giữ chỗ** luôn gọi trực tiếp qua `api` (cần biết kết quả ngay);
**xác nhận / trả lại / nhập lại kho sau khi đơn đã tồn tại** luôn đi qua event của `orders` (outbox cùng
transaction với chuyển trạng thái, giao ít nhất một lần, bên nhận xử lý không trùng theo `ref`);
**trả lại trước khi đơn tồn tại** (bước 4–6 hỏng) gọi trực tiếp `release`, và job hết hạn của bên giữ
chỗ là lưới an toàn cuối cùng. Mã tham chiếu chung: `orders:<order_id>` (UUIDv7 dạng chuỗi).

**Hạn thanh toán (D-218).** Một nguồn duy nhất cho mọi thanh toán `pending`, kể cả VietQR:
`AUTOXIDE_LEDGER_PAYMENT_EXPIRY_MINUTES` của đặc tả 36 (mặc định 30). `orders` không có hạn riêng: đọc
`expires_at` từ `ledger` làm `payment_due_at`. Chuyển khoản chậm hơn hạn vẫn được `ledger` khớp vào hàng
chờ sai lệch của bước 36, không mất tiền.

**Nhiều chế độ dòng tiền trong một checkout (D-216, B-b2, B-c2).** Một giỏ có nhiều nút bán: checkout
trả `payment_actions[]`, mỗi chế độ dòng tiền một mục (`kind`, `amount`, `qr` / `redirect_url` / `cod`),
giao diện hiện từng bước; chỉ một mã QR khi mọi nút cùng đối tác thu hộ. QR luôn trỏ tới tài khoản của
đối tác có giấy phép hay của người bán, không bao giờ của nền tảng (D-187, D-201).

**Trạng thái thanh toán công khai (D-201).** `GET /public/payments/{token}` cho màn QR: token sống bằng hạn
thanh toán; giao diện hỏi định kỳ, hay dùng SSE nếu bước 40 đã có.

### Máy trạng thái đơn

Trạng thái (`status`): `pending_payment`, `confirmed`, `preparing`, `shipping`, `delivered`,
`delivery_failed`, `completed`, `cancelled`. Hai trường phụ: `payment_status` (`unpaid`,
`partially_paid` (đã trả trước một phần, D-232), `cod_pending`, `paid`, `partially_refunded`, `refunded`) và
`fulfillment_status` (`unfulfilled`, `partial`, `fulfilled`, `returned_partial`, `returned`). Bảng chuyển là
một hằng số trong `lifecycle/transitions.rs`, mọi chuyển đi qua `Lifecycle::apply` (khoá dòng đơn bằng
`rows::lock_one`, kiểm bảng, ghi `orders_status_history`, event, job sổ cái — một transaction, bọc
`write_with_retry`). Không có `UPDATE status` nào ngoài hàm này (một test đọc mã nguồn của crate kiểm điều đó — xem
"Test bắt buộc").

| Từ | Sang | Ai / cái gì gây ra | Điều kiện, việc kèm theo |
|---|---|---|---|
| (mới) | `pending_payment` | checkout | thanh toán online / VietQR / chuyển khoản tay |
| (mới) | `confirmed` | checkout; POS; `OrderIntake::import` | COD đủ điều kiện; POS đi tiếp ngay `confirmed → delivered` trong cùng transaction; đơn kênh |
| `pending_payment` | `confirmed` | thanh toán xong (event bước 36); nhân viên xác nhận chuyển khoản tay | đủ tiền; `payment_status = paid` — đơn trả trước một phần: đủ khoản phải trả lúc đặt, `payment_status = partially_paid` (D-232) |
| `pending_payment` | `cancelled` | người mua; nhân viên; job hết hạn | không có khoản đã thu |
| `confirmed` | `preparing` | nhân viên | — |
| `confirmed` | `delivered` | job (hàng số), nhân viên (nhận tại cửa hàng, dịch vụ) | đơn không cần vận chuyển |
| `confirmed`, `preparing` | `cancelled` | người mua (tới trạng thái mà `buyer_cancel_until` cho: `confirmed` hay `preparing`, D-209), nhân viên; hết hạn chờ nhận đơn (`acceptance_hours`) | đã thu thì tạo hoàn tiền toàn phần |
| `preparing` | `shipping` | event `shipping.shipment_status_changed` (trạng thái đã lấy hàng); nhân viên (tự giao) | ít nhất một kiện đã đi; kiện có hàng đặt trước của đơn trả trước một phần chỉ đi khi phần còn lại đã trả hay chuyển COD (D-232) |
| `shipping` | `delivered` | event `shipping.shipment_delivered`; nhân viên (tự giao) | mọi kiện đã giao |
| `shipping` | `delivery_failed` | event `shipping.shipment_delivery_failed` / `shipping.shipment_returned` | ghi thống kê COD |
| `delivery_failed` | `shipping` | nhân viên giao lại | yêu cầu vận đơn mới |
| `delivery_failed` | `cancelled` | nhân viên; event hàng hoàn đã về kho | hoàn tiền nếu đã thu; nhập lại kho qua event |
| `delivered` | `completed` | job `complete_delivered` khi hết hạn đổi trả, không còn yêu cầu trả / tranh chấp mở (kể cả đơn POS) | mốc cho hoa hồng, điểm, chi trả người bán (bước 46–49) |

Chuyển không có trong bảng → 409 `orders.invalid_transition` (kèm `from`, `to`). Đổi trả không đổi
`status` của đơn (đơn `delivered` / `completed` vẫn giữ), chỉ đổi `payment_status` và
`fulfillment_status`. Hoàn tất bị hoãn khi có yêu cầu trả hay tranh chấp (bước 46) đang mở: bên mở
gọi `OrderHolds::hold_completion(ref)` của `api` (một dòng trong `orders_completion_holds`).

**Chờ cửa hàng nhận đơn (D-209, B-b6).** `orders_policies.acceptance_hours` (mặc định 0 = tự xác nhận như
bảng trên). Khác 0 thì đơn đã thanh toán / COD chờ cửa hàng hay người bán bấm nhận; quá hạn thì thành việc
trong hộp việc (P2 bước 51), rồi huỷ theo chính sách, hoàn tiền tự động, event `orders.order_cancelled` với
`cause = seller_not_accepted` (tính vào `cancelled_by_seller` của bước 46). Sàn đặt mức cho người bán (kế
thừa theo cây).

### Kiểu và trait

Phác thảo — tra API thật bằng skill `check-library-api` và theo code đã merge của bước 33–44.

**`orders::api` (công bố cho bước 46–49):**

```rust
pub struct OrderId(pub Uuid);           // newtype; Display = uuid
pub enum OrderStatus { PendingPayment, Confirmed, Preparing, Shipping, Delivered, DeliveryFailed, Completed, Cancelled }
pub struct OrderSummary {               // #[non_exhaustive]
    pub id: OrderId, pub tenant_id: TenantId, pub org_id: OrgId, pub number: String,
    pub buyer_user_id: Option<Uuid>, pub channel: String, pub status: OrderStatus,
    pub funds_flow: FundsFlow, pub total: Money, pub placed_at: DateTime<Utc>,
    pub lines: Vec<OrderLineSummary>,   // variant_id, product_id, category_id, seller_org_id, quantity,
                                        // net (sau giảm, gồm thuế), tax, returned_quantity
    pub attribution: Option<Attribution>,
}
#[async_trait] pub trait Orders: Send + Sync {
    async fn find(&self, tenant: TenantId, id: OrderId) -> Result<Option<OrderSummary>, Error>;
    /// Người mua `buyer` đã nhận dòng hàng của `variant_id` chưa (đánh giá, bảo hành, giới thiệu bạn,
    /// nhãn `verified_buyer` của hỏi đáp — D-202).
    async fn delivered_lines(&self, tenant: TenantId, buyer: Uuid, variant_id: Uuid) -> Result<Vec<Uuid>, Error>;
}
/// Đơn từ ngoài (bước 48 kênh bán, nhập dữ liệu): giá do nguồn quyết, không tính lại.
#[async_trait] pub trait OrderIntake: Send + Sync {
    /// Không trùng theo (`channel`, `external_ref`) — gửi lại trả `Replayed`. Giữ và xác nhận tồn ngay;
    /// thiếu hàng KHÔNG từ chối (kênh đã bán): đơn tạo với `stock_shortage = true` và event
    /// `orders.stock_shortage`. Đơn vào thẳng `confirmed` (đã trả trên kênh hay COD của kênh).
    async fn import(&self, tenant: TenantId, order: ImportedOrder) -> Result<Submitted<OrderSummary>, Error>;
    /// Kênh báo trạng thái (đang giao, đã giao, huỷ, hoàn tất) — đi qua cùng `Lifecycle::apply`, cause `channel`.
    async fn external_transition(&self, tenant: TenantId, id: OrderId, to: OrderStatus, cause_ref: &str) -> Result<(), Error>;
}
/// Hoàn tiền do module khác quyết (tranh chấp của bước 46): cùng service với `POST /{id}/refunds`.
#[async_trait] pub trait OrderRefunds: Send + Sync {
    async fn refund(&self, tenant: TenantId, id: OrderId, amount: Money, reason: &str, reference: &Reference) -> Result<Submitted<Uuid>, Error>;
}
/// Hoãn hoàn tất đơn (tranh chấp của bước 46).
#[async_trait] pub trait OrderHolds: Send + Sync {
    async fn hold_completion(&self, tenant: TenantId, id: OrderId, reference: &str) -> Result<(), Error>;
    async fn release_completion(&self, tenant: TenantId, id: OrderId, reference: &str) -> Result<(), Error>;
}
/// Điểm cắm cho module đăng ký SAU `orders` (bước 46–49): gọi trong `Module::start` của họ.
pub trait OrderExtensions: Send + Sync {
    fn set_seller_split(&self, split: Arc<dyn SellerSplit>) -> Result<(), Error>;   // marketplace
    fn set_buyer_groups(&self, groups: Arc<dyn BuyerGroups>) -> Result<(), Error>;  // customers
    fn add_tender(&self, tender: Arc<dyn Tender>) -> Result<(), Error>;             // customers: points, gift_card, store_credit
}
#[async_trait] pub trait SellerSplit: Send + Sync {
    /// Gom dòng giỏ theo nút bán (người bán). Lỗi nếu người bán không được bán (KYC, tạm khoá).
    async fn split(&self, tenant: TenantId, lines: &[SplitLine]) -> Result<Vec<SellerGroup>, Error>;
}
#[async_trait] pub trait BuyerGroups: Send + Sync {
    async fn groups_of(&self, tenant: TenantId, buyer: Uuid) -> Result<Vec<Uuid>, Error>;
}
#[async_trait] pub trait Tender: Send + Sync {
    fn kind(&self) -> &'static str;     // "points", "gift_card", "store_credit"
    async fn hold(&self, tenant: TenantId, reference: &str, buyer: Option<Uuid>, input: &serde_json::Value, max: Money, expires_at: DateTime<Utc>) -> Result<Money, Error>;
    async fn release(&self, tenant: TenantId, reference: &str) -> Result<(), Error>;
    // xác nhận / hoàn lại: bên cài nghe orders.order_confirmed / orders.refund_completed
}
pub enum FundsFlow { SelfCollect, PartnerCollect, GatewaySplit, ChannelCollect }
pub struct Attribution { pub click_id: Option<Uuid>, pub code: Option<String>, pub live_session_id: Option<Uuid> }
```

`api` còn công bố trait `OrderLookups` với `verified_phone(token)` (D-202): `auth` (bước 34) gọi để lấy số
điện thoại đã qua OTP của một token xem đơn (mục "Mua không cần tài khoản").

Trả lời tự động (D-230): `orders` gắn một `FactSource` vào `SupportFacts` của `support` (bước 53) khi có
`support`: ý định `orders.order_status` (tham số số đơn) trả mẫu câu i18n "Đơn {number} đang {status}, dự kiến
giao {eta}" — chỉ khi đơn thuộc tài khoản hay số điện thoại đã xác thực của người hỏi, không thì `None`.

Khối cho trang (D-229, bước 67): `orders` khai `orders.add_to_cart`, `orders.buy_now`, `orders.reviews` qua
`Module::blocks()` và gắn `BlockData` (giá, còn hàng, điểm đánh giá — chỉ dữ liệu công khai).

`OrderExtensions` vì `start` chạy theo thứ tự đăng ký: `orders` không `require` được trait của module
đăng ký sau nó, nên module sau tự cắm vào (mỗi điểm cắm một lần; cắm hai lần → lỗi khởi động).
Comment trong code giải thích lý do này cho người đọc quen TypeScript (`Arc<dyn Trait>`, `OnceLock`).

**`api` của bước khác mà `orders` giả định** (đối chiếu với đặc tả 36, 41–44, 51, 53):

| Module | Gọi | Dùng ở |
|---|---|---|
| `catalog` 41 | `Catalog::snapshot(tenant, PriceContext { org_id, buyer_groups, channel }, &[variant_id], at) -> Vec<VariantSnapshot>` (SKU, tên theo ngôn ngữ, loại `physical/digital/service/bundle`, thành phần combo, `tax_group`, giá `Money` gồm thuế, giá trước khuyến mại, cân nặng / kích thước, `owner_org_id`, `warranty_months`, `serial_tracked`, `sellable`, `sale_mode`, ngày dự kiến có hàng, `preorder_ends_at` — D-204); `DigitalDelivery::grant(tenant, ref, buyer, variant_id)`; event `catalog.preorder_date_changed` (D-204) | giỏ, checkout, hàng số, đặt trước |
| `inventory` 42 | `Inventory::reserve(tenant, ReserveRequest { reference, lines, sales_org_id, ship_to: Option<Address>, expires_at }) -> Reservation { lines: [variant, qty, warehouse_org_id] }`, `release(tenant, reference)`; nghe `orders.order_confirmed` / `orders.order_cancelled` / `orders.return_received`; phát `inventory.preorder_allocated`; hết suất đặt trước → `orders.preorder_sold_out` (D-204) | checkout, POS, đặt trước |
| `promotions` 43 | `Promotions::evaluate(tenant, CartInput) -> Evaluation { line_discounts[{line_key, promotion_id, amount, reason}], order_discounts, shipping_discount, rejected_codes[{code, reason}] }`, `reserve(tenant, reference, &Evaluation, buyer, expires_at)`, `release`; nghe `orders.order_confirmed` / `…_cancelled`; dữ liệu giỏ, hoàn khi trả, quà theo mục "Coupon, voucher, chia tiền giảm" (D-222) | giỏ, checkout, đổi trả |
| `shipping` 44 | `ShippingQuotes::quote(tenant, QuoteRequest { origin_org_id, destination: Address, parcels, order_value }) -> Vec<ShippingOption { method_code, fee, eta, pickup_org_id }>`; `AddressBook::get(tenant, buyer, id) -> Address`; `hold_days` của phương thức nhận tại cửa hàng; nghe `orders.fulfillment_requested`, `orders.return_pickup_requested`, `orders.order_cancelled`; phát `shipping.shipment_status_changed`, `shipping.shipment_delivered`, `shipping.shipment_delivery_failed`, `shipping.shipment_returned`, `shipping.cod_collected` (tên chốt ở đặc tả 44) | giỏ, vận chuyển, COD, nhận tại cửa hàng, phí gửi trả |
| `tax` 36 | `TaxEngine::compute(tenant, TaxRequest { seller_org_id, date, prices_include_tax, lines[{key, tax_group, amount, quantity}] }) -> TaxResult { lines[{key, components[{code, rate, base, amount}]}] }` | báo giá, chứng từ |
| `ledger` 36 | `Ledger::post(tenant, JournalEntry { reference, org_id, occurred_at, memo, lines[{account, debit/credit: Money}] }) -> Result<Submitted<EntryId>>`, `Ledger::funds_flow(tenant, org_id) -> FundsFlow`; event thanh toán xong (giả định `ledger.payment_settled { purpose: "orders", reference, amount, gateway, external_id }`); `expires_at` của thanh toán `pending` (D-218); link thanh toán còn hạn (gửi lại link) | sổ cái, xác nhận, hạn thanh toán |
| `invoices` 36 | `Invoices::issue(tenant, InvoiceRequest { reference, kind: Sales/CashRegister, seller_org_id, buyer, lines, original: Option }) -> InvoiceId`, `Invoices::adjust(tenant, AdjustRequest { reference, original, lines, reason })`; event `invoices.invoice_issued` | hoá đơn |
| kernel 36 | `numbering::next(&transaction, &SequenceKey { tenant, series: "orders.order", scope: org_id, period: year }) -> Result<u64>`; `PaymentGateway::{create_payment, refund, status}` (có phương thức `installment`); VietQR: `vietqr::payload(...)` | số đơn, thanh toán |
| kernel 51 | `WorkSource` (P2), yêu cầu duyệt (P12), `/api/v1/me/flags` (P14, D-207), `Module::exports` (P16, D-213), `Module::reason_lists` + `ReasonCodes::ensure` (P18, D-223); `context.badge_source` (D-202) | hộp việc, đóng ca, xuất, lý do, nhãn |
| `compliance` 37 | `LegalDocuments::require_accepted(tenant, subject, "terms_of_sale")`, `AccountVerification::is_verified(tenant, user)`, `ProcessingBasis::register(...)`; chứng cứ chấp nhận điều kiện đặt trước (D-204), gồm điều kiện trả trước một phần (D-232) | checkout, đánh giá, COD |
| `notifications` 37 | `Notifier::send(tenant, Notification { reference, recipient, template, category: Transactional, locale, data })`; OTP qua SMS (bước 37) | thư xác nhận, trạng thái, OTP tra đơn |
| `media` | mở rộng `Media` bằng `uploaded_by(ids) -> HashMap<Uuid, Option<Uuid>>` có cài đặt mặc định, hay trả `uploaded_by` trong `MediaRef` theo cách không phá vỡ; `POST /api/v1/media/me/uploads` (D-202) | ảnh trả hàng |
| `support` 53 (tuỳ chọn) | `SupportIntake::open` (D-204) | nút "mở hỗ trợ" của đơn |
| `ai` | `TextModel` (D-109), gọi trong job, ngoài transaction | tóm tắt đánh giá |

### Bảng dữ liệu

Mọi bảng có `tenant_id` (Uuid) đứng đầu mọi khoá unique và index; dữ liệu theo cửa hàng mang `org_id`.
ID là UUIDv7 sinh trong ứng dụng; thời gian `timestamp_with_time_zone` UTC; tiền là cặp
`<tên>_minor` BIGINT + `<tên>_currency` CHAR(3); JSON lưu ở cột `text` (như `kernel_*_outbox`); trạng
thái là `string_len(32)` (không dùng enum của database — SQLite không có). Migration chia theo nhóm
tính năng (`m0001_carts`, `m0002_checkout_orders`, …), mỗi cái có `down`, cộng
`outbox_migration` và `jobs_migration`.

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `orders_carts` | `id`, `tenant_id`, `org_id` (cửa hàng bán), `buyer_user_id` NULL, `token_hash` NULL (vãng lai: SHA-256 của token), `currency`, `codes` text (JSON mã giảm — coupon), `voucher_ids` text (voucher trong ví, chỉ người đăng nhập — D-222), `ship_to` text (JSON `Address` hay `address_id`), `shipping_method` NULL, `attribution` text NULL, `status` (`active/merged/converted/abandoned`), `merged_into` NULL, `abandoned_notified_at` NULL, `created_at`, `updated_at`, `expires_at` | cột `active_buyer_key` NULL (= `org_id:buyer_user_id` khi giỏ `active` của người mua) unique theo (`tenant_id`, `active_buyer_key`) — một giỏ đang dùng mỗi người mua mỗi nút bán; index (`tenant_id`, `token_hash`), (`tenant_id`, `status`, `updated_at`) |
| `orders_cart_lines` | `tenant_id`, `cart_id`, `variant_id`, `quantity` INT, `added_at`, `updated_at` | pk (`tenant_id`, `cart_id`, `variant_id`) |
| `orders_checkouts` | `id` (= `order_id` sẽ tạo; cũng là id nhóm khi tách theo người bán), `tenant_id`, `owner_key` (`user:<id>` hay `cart:<id>`), `reference`, `reference_fingerprint`, `cart_id`, `state` (`reserving/placed/failed`), `failure_code` NULL, `holds` text (JSON các ref đã giữ), `order_ids` text, `created_at`, `updated_at` | unique (`tenant_id`, `owner_key`, `reference`); index (`tenant_id`, `state`, `created_at`) |
| `orders_orders` | `id`, `tenant_id`, `org_id` (nút bán: cửa hàng / người bán), `checkout_id` NULL, `number` VARCHAR(40), `channel` (`web/pos/<kênh>`), `external_ref` NULL (kênh), `buyer_user_id` NULL, `buyer_key` (HMAC theo khách của SĐT chuẩn hoá — chống bom hàng), `contact` text (tên, SĐT, email; người nhận hộ khi nhận tại cửa hàng: tên, SĐT — D-201), `ship_to` text NULL, `pickup_org_id` NULL, trường nhận tại cửa hàng (mục "Nhận tại cửa hàng"), `status`, `payment_status`, `fulfillment_status`, `payment_method`, `funds_flow`, `subtotal_*`, `discount_*`, `shipping_fee_*`, `tax_*`, `total_*`, `paid_*`, `refunded_*`, `currency`, `payment_due_at` NULL (= `expires_at` của `ledger`, D-218), `attribution` text NULL, `stock_shortage` BOOL default false, `invoice_request` text NULL (MST, tên công ty), `terms_version` NULL, `note` NULL, `placed_at`, `confirmed_at`, `delivered_at`, `completed_at`, `cancelled_at`, `cancel_reason` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `number`); unique (`tenant_id`, `channel`, `external_ref`); index (`tenant_id`, `org_id`, `placed_at`, `id`), (`tenant_id`, `buyer_user_id`, `id`), (`tenant_id`, `status`, `payment_due_at`), (`tenant_id`, `status`, `delivered_at`), (`tenant_id`, `buyer_key`) |
| `orders_lines` | `id`, `tenant_id`, `order_id`, `parent_line_id` NULL (thành phần combo), `variant_id`, `product_id`, `category_id`, `seller_org_id`, `kind`, `snapshot` text (SKU, tên, thuộc tính, ảnh, `warranty_months`), `sale_mode` và `preorder_available_on` NULL (chụp lúc đặt, D-204), `advance_*` NULL (phần trả trước của dòng) và `preorder_commit_at` NULL (mốc chốt sản xuất / nhập hàng chụp lúc đặt) (D-232), `quantity`, `unit_price_*` (gồm thuế, trước giảm), `list_price_before_promo_*`, `discount_*`, `net_*` (gồm thuế, sau giảm), `tax_*`, `discounts` text (JSON dòng giảm kèm lý do, `funding` — ai trả, bao nhiêu — và nguồn `automatic` / `code` / `voucher` + id, D-222), `warehouse_org_id` NULL, `returned_quantity` INT default 0 | index (`tenant_id`, `order_id`), (`tenant_id`, `variant_id`, `order_id`), (`tenant_id`, `product_id`) |
| `orders_line_taxes` | `tenant_id`, `line_id`, `tax_code`, `rate` VARCHAR(20) (chuỗi thập phân như `TaxEngine` trả, vd `"0.08"`), `base_*`, `amount_*` | pk (`tenant_id`, `line_id`, `tax_code`) |
| `orders_status_history` | `id`, `tenant_id`, `order_id`, `from_status` NULL, `to_status`, `cause` (`buyer/staff/system/payment/shipping/channel`), `actor_id` NULL, `reason` NULL, `at` | index (`tenant_id`, `order_id`, `id`) — chỉ thêm |
| `orders_payments` | `id`, `tenant_id`, `order_id`, `purpose` (`order` mặc định / `advance` / `balance` — trả trước, phần còn lại, D-232), `method` (`gateway/installment/vietqr/cod/cash/card_terminal/bank_transfer/channel/tender:<kind>`), `gateway` NULL, `external_id` NULL, `reference` (ref gửi đối tác), `amount_*`, `status` (`pending/succeeded/failed/expired/refunded_partial/refunded`), `action` text NULL (URL / payload QR), `installment` text NULL (đối tác, kỳ hạn), `succeeded_at` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `reference`); unique (`tenant_id`, `gateway`, `external_id`); index (`tenant_id`, `order_id`) |
| `orders_refunds` | `id`, `tenant_id`, `order_id`, `return_id` NULL, `payment_id`, `reference`, `reference_fingerprint`, `amount_*`, `status` (`pending/succeeded/failed/manual`), `external_id` NULL, `reason`, `created_by`, `created_at`, `updated_at` | unique (`tenant_id`, `reference`); index (`tenant_id`, `order_id`) |
| `orders_advance_plans` | `tenant_id`, `order_id`, `advance_*` (tổng phải trả trước của các dòng đặt trước), `upfront_*` (phải trả lúc đặt = hàng sẵn + trả trước), `balance_*` (= tổng đơn − `upfront`), `balance_method` (`payment_link/cod`), `terms` text (JSON chụp điều kiện đã công bố lúc đặt: tỉ lệ / số tiền, trần, sàn, mốc chốt từng dòng, tỉ lệ và trần giữ lại khi người mua huỷ, hạn trả phần còn lại, việc làm khi quá hạn), `terms_sha256`, `status` (`awaiting_advance/advance_paid/balance_requested/balance_paid/balance_cod/balance_overdue/cancelled`), `balance_attempt` INT default 0, `balance_requested_at` NULL, `balance_due_at` NULL, `created_at`, `updated_at` (D-232) | pk (`tenant_id`, `order_id`); index (`tenant_id`, `status`, `balance_due_at`) |
| `orders_completion_holds` | `tenant_id`, `order_id`, `reference`, `created_at` | pk (`tenant_id`, `order_id`, `reference`) |
| `orders_fulfillments` | `id`, `tenant_id`, `order_id`, `shipment_ref` NULL (id vận đơn của `shipping`), `lines` text, `cod_*` NULL, `status` (`requested/picked_up/delivered/failed/returned`), `created_at`, `updated_at` — đơn có hàng sẵn lẫn hàng đặt trước có hai lần giao (D-204) | index (`tenant_id`, `order_id`); unique (`tenant_id`, `shipment_ref`) |
| `orders_returns` | `id`, `tenant_id`, `order_id`, `org_id`, `number`, `status` (`requested/approved/rejected/received/refunded/closed`), `reason_code` (mã của danh sách lý do `orders.return_reason`, D-223), `buyer_note`, `resolution` (`refund/exchange`), `return_method` (`pickup/drop_off/in_store`, D-201), `media_ids` text (danh sách UUID ảnh, tối đa 6 — D-202), `lines` text (line_id, qty, tình trạng khi nhận: `restock/damaged`), `refund_*` NULL, `requested_at`, `deadline_at`, `decided_by` NULL, `decided_at` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `number`); index (`tenant_id`, `org_id`, `status`, `id`), (`tenant_id`, `order_id`) |
| `orders_invoice_links` | `tenant_id`, `order_id`, `return_id` NULL, `kind` (`sales/cash_register/adjustment/replacement`), `invoice_id`, `status`, `created_at` | index (`tenant_id`, `order_id`) |
| `orders_lookup_tokens` | `id`, `tenant_id`, `order_id`, `phone_hash`, `token_hash`, `expires_at`, `otp_confirmed_at`, `created_at` (token xem đơn vãng lai, chỉ lưu băm — D-202) | — |
| `orders_pos_shifts` | `id`, `tenant_id`, `org_id` (cửa hàng), `register_code`, `opened_by`, `opened_at`, `opening_cash_*`, `closed_by` NULL, `closed_at` NULL, `expected_cash_*` NULL, `counted_cash_*` NULL, `variance_minor` NULL, `variance_reason` NULL, `variance_note` NULL, `ledger_entry_id` NULL (D-227), `status` (`open/closed`) | unique (`tenant_id`, `org_id`, `register_code`) qua cột `open_key` NULL (như `active_buyer_key`); index (`tenant_id`, `org_id`, `opened_at`) |
| `orders_pos_sales` | `tenant_id`, `order_id`, `shift_id`, `reference`, `reference_fingerprint` | unique (`tenant_id`, `shift_id`, `reference`) |
| `orders_reviews` | `id`, `tenant_id`, `org_id`, `order_line_id` , `product_id`, `variant_id`, `author_user_id`, `rating` SMALLINT 1–5, `body`, `media_ids` text, `status` (`published/hidden/removed_by_author`), `hidden_reason` NULL (mã của danh sách lý do `orders.review_hidden`, D-223), `hidden_by` NULL, `hidden_at` NULL, `reply` NULL, `replied_at` NULL, `ai_assisted` BOOL, `helpful_count` INT default 0 (D-203), `created_at`, `updated_at` | unique (`tenant_id`, `order_line_id`); index (`tenant_id`, `product_id`, `status`, `id`), (`tenant_id`, `product_id`, `status`, `helpful_count`, `id`) cho `sort=helpful` (D-203), (`tenant_id`, `org_id`, `created_at`) cho đánh giá mới (đợt 2) |
| `orders_review_votes` | `tenant_id`, `review_id`, `user_id`, `created_at` (phiếu "hữu ích", D-203) | pk (`tenant_id`, `review_id`, `user_id`); index (`tenant_id`, `user_id`) để xoá theo tài khoản |
| `orders_review_summaries` | `id`, `tenant_id`, `product_id`, `locale`, `body`, `status` (`draft/approved/rejected/superseded`), `source_review_count`, `source_until`, `model`, `generated_at`, `reviewed_by` NULL, `reviewed_at` NULL (bản nháp tóm tắt AI, D-109) | unique bản `approved` mỗi (`tenant_id`, `product_id`, `locale`) qua cột khoá NULL (như `active_buyer_key`) |
| `orders_review_stats` | `tenant_id`, `product_id`, `count`, `rating_sum`, `count_1`…`count_5`, `updated_at` | pk (`tenant_id`, `product_id`) — cập nhật bằng `rows::adjust` |
| `orders_delivery_stats` | `tenant_id`, `buyer_key`, `delivered`, `failed`, `refused`, `updated_at` | pk (`tenant_id`, `buyer_key`) |
| `orders_cod_rules` | `id`, `tenant_id`, `org_id`, `min_orders`, `max_fail_ratio_bp`, `max_order_value_*` NULL, `action` (`require_prepayment/block_cod`), `effective_from`, `effective_to` NULL | index (`tenant_id`, `org_id`, `effective_from`) |
| `orders_cod_blocks` | `id`, `tenant_id`, `buyer_key`, `reason_code`, `note`, `expires_at`, `created_by`, `created_at`, `lifted_at` NULL | index (`tenant_id`, `buyer_key`, `expires_at`) |
| `orders_warranties` | `id`, `tenant_id`, `org_id`, `order_line_id`, `serial` NULL, `buyer_user_id` NULL, `starts_at`, `ends_at`, `status` (`active/expired/void`) | unique (`tenant_id`, `order_line_id`, `serial`) (serial NULL → `-`); index (`tenant_id`, `serial`) |
| `orders_warranty_claims` | `id`, `tenant_id`, `warranty_id`, `org_id` (nơi nhận), `number`, `status` (`received/inspecting/repairing/replacing/ready/returned/rejected`), `issue`, `resolution` NULL, `due_at`, `created_at`, `updated_at` | unique (`tenant_id`, `number`); index (`tenant_id`, `org_id`, `status`, `id`) |
| `orders_policies` | `tenant_id`, `org_id`, `return_window_days`, `buyer_cancel_until` (`pending_payment/confirmed/preparing`, D-209), `allowed_return_reasons` (danh sách mã của `orders.return_reason`, D-209, D-223), `return_photo_reasons` text (mã lý do bắt buộc ảnh, mặc định `damaged,wrong_item` — D-202), `acceptance_hours` (0 = tự xác nhận, D-209), `cod_enabled`, `cod_max_*` NULL, `invoice_timing` (`on_confirm/on_delivered/on_complete/on_request`, mặc định `on_delivered` — D-215), `advance_invoice_timing` (`with_order` mặc định / `on_advance`, D-232), `preorder_payment` (`full/on_arrival/partial`, D-204, D-232), trường trả trước một phần (`preorder_advance_bp` NULL, `preorder_advance_fixed_*` NULL, `preorder_advance_min_*` NULL, `preorder_advance_max_*` NULL, `preorder_commit_days` NULL, `preorder_cancel_retention_bp` default 0, `preorder_cancel_retention_max_*` NULL, `preorder_balance_due_hours` default 72, `preorder_balance_cod` BOOL default false, `preorder_balance_overdue_action` (`cancel_per_terms` mặc định / `cancel_full_refund`), `preorder_max_delay_days` default 30 — mục "Trả trước một phần cho hàng đặt trước", D-232), `pickup_expired_action` (`cancel_refund` mặc định / `keep_waiting`), `number_prefix`, `updated_at` — chính sách đơn theo D-209, D-215 (B-b5, B-b6, B-c8) | pk (`tenant_id`, `org_id`); giá trị hiệu lực theo cây bằng khuôn cài đặt kế thừa của bước 34 |

`active_buyer_key` / `open_key` / cột khoá của `orders_review_summaries`: cột mang giá trị khi dòng đang
"hoạt động", NULL khi không — unique trên cột NULL được cả hai database cho nhiều NULL, thay cho index một
phần (`WHERE`) mà SeaORM migration không viết chung được.

### Danh sách lý do (D-223)

`orders` khai qua `Module::reason_lists()` (P18 bước 51); service kiểm mã bằng `ReasonCodes::ensure`, dòng
dữ liệu lưu mã, không lưu nhãn:

- `orders.review_hidden` — lý do ẩn đánh giá; mặc định từ gói `vn` của bước 37, đánh dấu `legal`.
- `orders.return_rejected` — lý do từ chối trả hàng.
- `orders.return_reason` — lý do trả hàng; giá trị của `allowed_return_reasons` lấy từ đây. Bộ mã của đặc tả
  này: `changed_mind`, `damaged`, `wrong_item`, `not_as_described`, `missing_parts`, `other` (D-202).
- `orders.cash_variance` — lý do chênh lệch tiền két khi đóng ca (D-227).

### Giỏ: coupon, voucher, chia tiền giảm (D-222)

- **Dữ liệu giỏ cho `promotions`.** Giỏ giữ `codes` (coupon) và `voucher_ids` (voucher trong ví, chỉ người
  đăng nhập) tách nhau; `PUT /carts/{id}/vouchers`. Điền `Cart.buyer_keys` (`user:<id>`, băm số điện thoại,
  băm địa chỉ giao chuẩn hoá bằng HMAC với secret của khách — không lưu bản thô ở `promotions`),
  `payment_method`, `carrier`, `ship_to_province_code`, `client`, `membership_tier` (từ `BuyerGroups` do
  `customers` cắm vào `OrderExtensions`; chưa cài thì rỗng), `gift_selections`, `first_order` (người mua chưa có
  đơn hoàn tất trong khách — `promotions` không hỏi ngược `orders`, D-228). Đổi phương thức thanh toán hay
  hãng giao thì báo giá lại (khoản giảm theo phương thức thanh toán có thể đổi).
- **Mã và voucher dùng được cho giỏ** (D-201, D-222, D-228): `GET /carts/{id}/promotions` gọi
  `Promotions::eligible` với giỏ đã dựng; trả mã / voucher dùng được, mã chưa đủ điều kiện kèm lý do,
  `best_selection`.
- **Chụp vào đơn.** Mỗi dòng đơn lưu khoản giảm theo chương trình kèm `funding` (ai trả, bao nhiêu) và nguồn
  (`automatic` / `code` / `voucher` + id); đơn con trên sàn nhận phần chia của voucher sàn.
- **Trả hàng.** Theo mục 8 của đặc tả 43: hoàn theo tỉ lệ bằng `Money::allocate`, tổng hoàn không vượt tiền đã
  trả; `promotions.clawback_on_return` (mặc định `none`); quà theo `gift_return_policy`; `returns/preview` hiện
  rõ phần giảm bị trừ. Huỷ trọn đơn gọi `Promotions::release` (voucher về ví nếu còn hạn).
- **Hoá đơn.** Khoản giảm ghi trên hoá đơn là chiết khấu / giảm giá của dòng; hàng tặng là dòng riêng ghi rõ
  "hàng khuyến mại"; phần do sàn hay đối tác thanh toán trả ghi theo dữ liệu quy tắc hoá đơn — **cần đối
  chiếu** NĐ 123/2020 (sửa bởi NĐ 70/2025) và cần người có chuyên môn xác nhận trước khi chạy thật.
- Báo giá giỏ có `free_shipping_remaining` (theo từng nơi bán, D-201).

### Hàng đặt trước (D-204)

- Dòng đơn chụp `sale_mode` và `preorder_available_on` lúc đặt; một đơn có hàng sẵn lẫn hàng đặt trước thì
  tách thành hai lần giao (`orders_fulfillments`) — hàng sẵn đi trước, không chờ.
- **Thanh toán:** theo chính sách của nút `preorder_payment` (`full` trả hết lúc đặt / `on_arrival` trả khi
  hàng về, chỉ với COD hay thanh toán khi có hàng / `partial` trả trước một phần — mục "Trả trước một phần cho
  hàng đặt trước", D-232).
- **Thông tin bắt buộc trước khi đặt:** báo giá giỏ trả `preorder: {available_on, message}` cho từng dòng;
  checkout yêu cầu `accept_preorder_terms = true` khi giỏ có hàng đặt trước (lưu vào chứng cứ chấp nhận của
  bước 37). Thời gian giao dự kiến phải hiện rõ trước khi đặt.
- **Dời ngày / huỷ hàng:** nghe `catalog.preorder_date_changed` → báo người mua qua `notifications` (tin giao
  dịch), cho **huỷ không mất phí** và hoàn tiền đủ khi ngày mới trễ hơn ngày đã hứa; người bán ngừng bán đặt
  trước → huỷ các dòng chưa chia hàng, hoàn tiền tự động. Nghe `inventory.preorder_allocated` → chuyển dòng
  sang `processing`.

### Trả trước một phần cho hàng đặt trước (D-232)

Gọi là **"trả trước một phần"** (thanh toán trước một phần giá), **không** gọi và không mặc định xử lý như
"đặt cọc" theo Bộ luật Dân sự 2015 Điều 328 (đặt cọc có chế tài: bên nhận cọc từ chối giao kết thì trả cọc và
một khoản tương đương). Khách muốn điều khoản đặt cọc thật thì đó là chính sách riêng của họ, ghi rõ trong
điều kiện giao dịch — **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**. Mã, tên trường, nhãn giao
diện và mẫu thông báo đều dùng `advance` / "trả trước", không dùng `deposit` / "cọc".

- **Chính sách (`orders_policies`, kế thừa theo cây).** `preorder_payment = partial` cần **đúng một** trong
  `preorder_advance_bp` (tỉ lệ, 1–9999; vd 3000 = 30%) hay `preorder_advance_fixed_*` (số tiền cố định cho
  phần đặt trước của đơn); `preorder_advance_min_*` / `preorder_advance_max_*` là sàn / trần (NULL = không
  giới hạn). Sai → `orders.preorder_advance_invalid` (422), gom lỗi một lần (D-083).
- **Tính tiền** (một hàm thuần trong `checkout/`, có test thuộc tính): phần đặt trước = Σ `net` các dòng đặt
  trước + phần phí giao và giảm cấp đơn chia cho chúng bằng `Money::allocate`; `advance` = tỉ lệ × phần đặt
  trước làm tròn **xuống** đơn vị nhỏ nhất (hay số cố định), kẹp vào [sàn, trần], không vượt phần đặt trước;
  chia xuống dòng bằng `Money::allocate` (`orders_lines.advance_*`). `upfront` = phần hàng sẵn + `advance`;
  `balance` = tổng đơn − `upfront` — **tính bằng phép trừ**, không nhân tỉ lệ lần nữa, nên tổng thu luôn bằng
  giá đơn từng đồng. `advance` bằng phần đặt trước thì đơn coi như `full`.
- **Công bố trước khi đặt.** Báo giá giỏ trả thêm cho mỗi dòng đặt trước `preorder.advance: {amount,
  commit_at, retention_bp, retention_max}` và cho cả giỏ `advance: {upfront, balance, balance_due_hours,
  balance_methods}`. `accept_preorder_terms = true` (đã có, D-204) là đồng ý cả điều kiện này: `terms` và
  `terms_sha256` chụp vào `orders_advance_plans` trong transaction tạo đơn và ghi vào chứng cứ chấp nhận của
  bước 37. Đổi chính sách sau khi đặt không áp ngược cho đơn đã đặt.
- **Mốc chốt** của dòng (`preorder_commit_at`, chụp lúc đặt): `preorder_ends_at` của biến thể (hết nhận đặt =
  lúc người bán chốt số lượng sản xuất / nhập hàng); biến thể không có thì `preorder_available_on −
  preorder_commit_days`; cả hai không có thì không có mốc (huỷ luôn hoàn đủ).
- **Dòng tiền (D-187).** Phần trả trước đi qua đối tác có giấy phép như mọi thanh toán: `payment_actions[]`
  của checkout có số tiền `upfront`; `orders_payments.purpose = advance`, `reference = orders:<order_id>:advance`.
  Ghi sổ là nghĩa vụ với người mua: Nợ `clearing.gateway` / Có `orders.customer_advance` (mục "Chỗ dễ sai").
  Đủ `upfront` → `confirmed`, `payment_status = partially_paid`, plan `advance_paid`.
- **Thu phần còn lại khi hàng sẵn sàng.** Nghe `inventory.preorder_allocated`: khi mọi dòng đặt trước còn hiệu
  lực của đơn đã chia hàng → plan `balance_requested`, `balance_due_at = now + preorder_balance_due_hours`,
  event `orders.preorder_balance_requested` (cùng transaction) và job `orders.request_preorder_balance`:
  - `balance_method = payment_link`: tạo thanh toán ở `ledger` (`purpose = balance`, `reference =
    orders:<order_id>:balance:<balance_attempt>`), báo người mua qua `notifications` (tin giao dịch) kèm link
    còn hạn. Mỗi link sống bằng hạn thanh toán của `ledger` (D-218); hết hạn mà chưa quá `balance_due_at` thì
    người mua mở lại bằng `POST /me/orders/{id}/payments` (hay nhân viên gửi lại bằng `POST /{id}/payment-link`):
    lần thử mới tăng `balance_attempt`, chỉ khi lần trước đã `expired` / `failed`.
  - `balance_method = cod` (chỉ khi `preorder_balance_cod = true` và đơn đủ điều kiện COD của mục chống bom
    hàng; người mua chọn lúc checkout bằng `preorder_balance_method`): plan `balance_cod`, kiện hàng đặt trước
    mang `cod = balance`, ghi sổ khi `shipping.cod_collected` như COD thường.
  - Chưa `balance_paid` / `balance_cod` thì không yêu cầu vận đơn cho hàng đặt trước: 409
    `orders.balance_unpaid`.
- **Quá hạn phần còn lại.** Job `orders.expire_preorder_balances` (mỗi giờ): plan `balance_requested` quá
  `balance_due_at` → `balance_overdue`, event `orders.preorder_balance_overdue`, rồi theo
  `preorder_balance_overdue_action` đã chụp: `cancel_per_terms` (mặc định — huỷ các dòng đặt trước, hoàn
  phần trả trước theo điều kiện người mua huỷ sau mốc, như dưới) hay `cancel_full_refund`. Thanh toán phần còn
  lại tới sau khi đã huỷ → hoàn tự động (`late_payment`), như thanh toán muộn của đơn thường.
- **Huỷ.**
  - Người bán huỷ, ngừng bán đặt trước, hay dời ngày quá `preorder_max_delay_days` so với ngày đã hứa → huỷ
    các dòng đặt trước chưa giao, hoàn **toàn bộ** phần trả trước của chúng tự động (không phụ thuộc mốc).
    Dời ngày trong giới hạn: người mua huỷ được miễn phí, hoàn đủ (mục "Hàng đặt trước").
  - Người mua huỷ (`POST /me/orders/{id}/cancel` hay đường vãng lai, cùng `buyer_cancel_until`): theo điều kiện
    đã chụp lúc đặt. Mặc định: trước `preorder_commit_at` của dòng → hoàn đủ; sau mốc → giữ lại
    `min(advance của dòng × retention_bp, retention_max, advance của dòng)`, hoàn phần còn lại. Mặc định
    `preorder_cancel_retention_bp = 0` (luôn hoàn đủ) — khách tự đặt tỉ lệ. Phần hàng sẵn đã giao thì chỉ
    huỷ các dòng đặt trước chưa chia hàng.
  - Hoàn đi qua `orders_refunds` và `orders.refund` như mọi hoàn tiền (bút toán đảo, `orders.refund_completed`);
    phần giữ lại ghi Nợ `orders.customer_advance` / Có `orders.cancellation_income`.
- **Hoá đơn.** Khoản trả trước không đổi thời điểm lập hoá đơn của đơn (`invoice_timing`); khách cần lập hoá
  đơn ngay khi thu tiền trước thì đặt `advance_invoice_timing = on_advance` (hoá đơn cuối trừ phần đã lập). Thời
  điểm lập hoá đơn với khoản thu trước — **cần đối chiếu** NĐ 123/2020 sửa bởi NĐ 70/2025 (Điều 9) và người có
  chuyên môn trước khi chạy thật; vì vậy để là dữ liệu, không viết cứng.
- **Không thu hai lần.** `POST /checkout` và `POST /me/orders/{id}/payments` nhận `Idempotency-Key`; mỗi lần
  thử có `reference` riêng (unique ở `orders_payments` và `ledger`); một lần thử mới chỉ mở khi lần trước đã hết
  hạn / hỏng (khoá dòng plan bằng `rows::lock_one`). Hai lần thử cùng thành công (đối tác báo muộn) → lần sau hoàn
  tự động, không cộng vào đơn.

### Nhận tại cửa hàng (D-201, B-c7, đợt 2)

- Liên hệ của đơn có người nhận hộ (tên, số điện thoại).
- `POST /{id}/pickup/ready` gửi mã nhận 6 số cho người mua; `POST /{id}/pickup/handover {code | proxy_name,
  proxy_phone}` giao hàng; danh sách đơn lọc `pickup_org_id`, `pickup_status`.
- **Giữ hàng chờ nhận.** Khi `pickup/ready`: `pickup_hold_until = ready_at + hold_days` (`hold_days` theo lịch
  của bước 44). Job `orders.expire_pickups` mỗi giờ: quá hạn → `pickup_expired`, nhắc người mua một lần trước
  24 giờ; hết hạn thì làm theo chính sách của nút `pickup_expired_action`: `cancel_refund` (mặc định — huỷ, trả
  kho, hoàn tiền) hay `keep_waiting`.

### Đổi trả: cách trả, ảnh, xem trước tiền hoàn

- **Cách trả (D-201).** `return_method` (`pickup` / `drop_off` / `in_store`), thời gian hoàn tiền dự kiến theo
  phương thức thanh toán.
- **Lý do.** Mã thuộc `allowed_return_reasons` của nút (D-209), lấy từ danh sách `orders.return_reason`;
  từ chối kèm lý do thuộc `orders.return_rejected` (D-223).
- **Ảnh kèm (D-202).** `media_ids` tối đa 6; lý do thuộc `return_photo_reasons` (kế thừa theo cây như các cột
  khác của `orders_policies`) thì bắt buộc ảnh. Mỗi `media_id` kiểm qua trait của `media` (`uploaded_by`) —
  phải do chính người mua tải lên, không trong thùng rác; không đọc bảng của `media`. Người mua tải ảnh qua
  endpoint tải lên dành cho người mua của `media` (ảnh `private`, hạn mức theo người dùng) — nếu `media` chưa
  có thì bước này thêm `POST /api/v1/media/me/uploads` (permission `media.upload_own`, chỉ ảnh, ≤ 10 MiB)
  vào `media`. Lỗi ảnh gom chung một lần với lỗi khác của đầu vào (D-083).
- **Xem trước (D-202).** `POST /me/orders/{id}/returns/preview {lines: [{line_id, qty}], reason_code,
  return_method?}` (`orders.buy`, chỉ đơn của mình; không ghi, không event, không giữ chỗ) trả:
  `refund: {amount, currency}` của các dòng đã chọn (giá đã trả sau giảm giá phân bổ, cộng phí vận chuyển
  khi lỗi do người bán theo chính sách; phần giảm bị trừ hiện rõ — D-222), `methods: [{method, fee: {amount,
  currency}, available}]` (phí theo `shipping` qua `api`, `in_store` chỉ khi có cửa hàng nhận), `refund_eta:
  {min_days, max_days, channel}` theo phương thức thanh toán của đơn (bảng dữ liệu theo phương thức, có ngày
  hiệu lực), `deadline_at` (hạn đổi trả), `photos_required: bool`. Tiền là số nguyên đơn vị nhỏ nhất + mã
  tiền tệ. Lỗi giống `POST …/returns` (`orders.return_window_closed`, `orders.return_quantity_invalid`…)
  nhưng là 422 không ghi.
- Hoàn theo tỉ lệ khoản giảm, quà, voucher: mục "Giỏ: coupon, voucher, chia tiền giảm".

### Hoá đơn và chứng từ của người mua

- `invoice_timing` mặc định `on_delivered` (lập khi giao xong); `on_complete` chỉ dùng khi khách có căn cứ;
  giữ `on_confirm`, `on_request` (D-215, B-b5).
- Khoản giảm và hàng tặng trên hoá đơn: mục "Giỏ: coupon, voucher, chia tiền giảm" (D-222).
- `GET /me/orders/{id}/documents` (hoá đơn, phiếu bảo hành, biên bản trả hàng) và bản dùng token xem đơn vãng
  lai (B-c5).

### Mua không cần tài khoản (D-201, D-202, D-204)

- **Tra đơn.** `POST /public/orders/lookup {number, phone}` → OTP qua SMS (bước 37) → `POST
  /public/orders/lookup/{id}/confirm {code}` trả `access_token` (băm lưu ở `orders_lookup_tokens`). Token gửi
  bằng header `Authorization: Order <token>`, chỉ mở đúng một đơn. Trả lời "không thấy đơn" và "sai số điện
  thoại" giống hệt nhau (không dò được số đơn).
- **Thao tác** (mỗi cái gọi CÙNG service với đường có tài khoản, nhận `Idempotency-Key`):
  - `POST /public/orders/{id}/cancel {reason}` — cùng chính sách `buyer_cancel_until`;
  - `POST /public/orders/{id}/address` — đổi địa chỉ giao khi đơn chưa rời kho (trạng thái trước `shipping`;
    đã rời kho → 409 `orders.address_locked`), cần OTP lần hai: `POST /public/orders/{id}/address/otp` rồi
    gửi `{address, code}`; phí giao tính lại, chênh lệch thì trả 409 `orders.address_change_needs_payment`;
  - `POST /public/orders/{id}/support {message}` — mở hỗ trợ (mục "Phiếu hỗ trợ").
- **Giới hạn (cấu hình, có mặc định).** `AUTOXIDE_ORDERS_LOOKUP_PER_HOUR` (mặc định 5, theo cặp mã đơn + số
  điện thoại băm, và 20 theo IP băm) qua `autoxide_kernel::throttle`; `AUTOXIDE_ORDERS_LOOKUP_RESEND_SECONDS`
  (mặc định 60, gửi lại OTP sớm hơn → 429 `orders.lookup_resend_too_soon` có `retry_after`);
  `AUTOXIDE_ORDERS_LOOKUP_TOKEN_MINUTES` (mặc định 30).
- **Bảo vệ dữ liệu cá nhân.** Response cho token xem đơn che số điện thoại (`090•••567`: giữ 3 số đầu, 3 số
  cuối), tên người nhận chỉ chữ cái đầu của họ và đệm, địa chỉ chỉ tới cấp phường / xã (không số nhà,
  đường); người mua có tài khoản thì thấy đầy đủ.
- **Gắn đơn vào tài khoản.** Sau khi người mua xác thực số điện thoại, `POST /me/orders/claim` gắn mọi đơn
  vãng lai cùng số đó trong khách.
- **Tạo tài khoản từ số đã xác thực.** Thuộc **`auth` (bước 34)**: trang xem đơn gọi
  `POST /api/v1/auth/phone-signups {lookup_token}` — `auth` hỏi `orders` (`OrderLookups::verified_phone(token)`)
  để lấy số đã qua OTP, tạo tài khoản đăng nhập bằng passkey hay OTP (không bắt buộc mật khẩu), đánh dấu số
  đã xác thực; rồi giao diện gọi `POST /me/orders/claim`. `orders` không tạo tài khoản.
- **Phiếu hỗ trợ (bước 53, D-204).** Nút "mở hỗ trợ" trên trang đơn (có tài khoản và vãng lai) gọi
  `SupportIntake::open` của `support` với `links = ["orders.order:<id>"]`, `reference =
  "orders.order:<id>:<Idempotency-Key>"`. `support` chưa cài thì endpoint trả 404 `orders.support_unavailable`
  và đơn không hiện nút (đọc qua `Subscription::optional` / điểm nối tuỳ chọn). Không có event
  `orders.support_requested`.

### Đánh giá: lọc, hữu ích, tóm tắt AI, nhãn người đã mua

Đặt trong thư mục `reviews/` của `orders`, bật bằng tính năng `orders.reviews`.

- **Danh sách công khai (D-201).** Lọc `has_media`, `rating`, `sort=newest|helpful`; `GET …/reviews/media` gom
  ảnh của người mua. Mặc định `newest`.
- **Phiếu "hữu ích" (D-203).**
  - `PUT /public/reviews/{id}/helpful` (đăng nhập, `orders.buy`): bỏ phiếu. Gửi lại khi đã bỏ phiếu thì trả
    200 như lần đầu, không đếm thêm (`INSERT … ON CONFLICT DO NOTHING`, chỉ tăng đếm khi chèn được dòng).
    `DELETE` cùng đường: rút phiếu; chưa bỏ phiếu thì không đổi gì. Cả hai trả `{helpful_count, voted_by_me}`.
  - `helpful_count` đổi trong CÙNG transaction với phiếu bằng `UPDATE … SET helpful_count = helpful_count ± 1`
    (không đọc rồi ghi). Đánh giá công khai trả thêm `helpful_count`; có đăng nhập thì thêm `voted_by_me`, lấy
    cho cả trang bằng một câu truy vấn.
  - `sort=helpful`: `helpful_count` giảm dần rồi `id` giảm dần, phân trang bằng `fetch_page_by` (con trỏ mang
    cả hai). Thứ tự "hữu ích" là người đọc tự chọn, không dùng để đẩy đánh giá xấu xuống.
  - Chặn bỏ phiếu sai: không bỏ phiếu cho đánh giá của chính mình (`orders.review_vote_own`, 409); người có
    `orders.reviews.moderate` hay quyền quản lý sản phẩm ở nút bán sản phẩm đó không được bỏ phiếu (người bán
    không tự đẩy đánh giá tốt lên; `orders.review_vote_not_allowed`, 403); chỉ đánh giá `published`, còn lại
    404 `orders.review_not_found`; không có nút "không hữu ích" (tránh dồn phiếu hạ đánh giá xấu); throttle
    theo người dùng `AUTOXIDE_ORDERS_REVIEW_VOTES_PER_HOUR` (mặc định 60), quá thì 429.
  - Đánh giá bị ẩn giữ nguyên phiếu (mở lại thì số cũ quay về), không nhận phiếu mới. Tài khoản bị xoá
    (`privacy::USER_DELETED`): xoá phiếu của người đó và trừ đếm trong cùng transaction; nhận lại event thì
    không trừ hai lần.
  - Không phát event cho từng phiếu (quá nhiều, không cần audit).
- **Ẩn đánh giá.** Chỉ với lý do thuộc danh sách `orders.review_hidden` (D-223).
- **Đánh giá mới, chưa xem (D-207).** Mỗi nhân viên lưu mốc đã xem bằng `/api/v1/me/flags` (key
  `orders.reviews.seen_at`); `GET /reviews?created_after=` và `GET /reviews/count?status&created_after` (index
  `tenant_id`, `org_id`, `created_at`). Không thêm bảng.
- **Bản nháp tóm tắt bằng AI (D-109, D-201).** `orders_review_summaries`; `POST /products/{id}/review-summaries`
  enqueue job gọi `TextModel` ngoài transaction, kết quả theo JSON Schema; `POST
  /review-summaries/{id}/approve|reject` (`orders.reviews.moderate`). Công khai chỉ trả bản `approved`, kèm
  nhãn AI; không lược ý kiến tiêu cực (kiểm bằng hướng dẫn trong `instructions`, người duyệt chịu trách nhiệm
  cuối).
- **Nhãn `verified_buyer` cho hỏi đáp (D-202).** `context.badge_source("catalog.product", &["verified_buyer"],
  resolver)`: resolver lấy các biến thể của sản phẩm qua `catalog::api` rồi hỏi
  `Orders::delivered_lines(tenant, author.user_id(), variant_id)`; có ít nhất một dòng đã giao thì trả
  `verified_buyer`. Chỉ đọc bảng của `orders`.

### POS: đóng ca có chênh lệch (D-227)

`close` tính `variance = counted − expected`; khác 0 thì bắt buộc `variance_reason` (danh sách
`orders.cash_variance`) và ghi chú. Chênh lệch ghi sổ cái ngay khi đóng ca vào tài khoản chờ xử lý của cửa
hàng: thiếu → `suspense.cash_shortage`, thừa → `suspense.cash_overage` (`ledger_entry_id` trỏ tới bút toán);
người quản lý xử lý sau bằng bút toán từ sổ cái. Vượt ngưỡng `cash_variance_approval_minor` (cài đặt kế thừa)
thì đóng ca cần người duyệt (P12 bước 51).

### Giỏ bỏ dở, link thanh toán, xuất đơn (B-b9, B-c6, D-213)

`GET /carts?status=abandoned` (danh sách giỏ bỏ dở), `POST /{id}/payment-link` (gửi lại link thanh toán còn
hạn của `ledger`). Xuất đơn đi qua xuất chung của bước 51 (P16, D-213): `orders` khai danh sách xuất được qua
`Module::exports`, không có endpoint xuất riêng.

### API REST (`/api/v1/orders`)

Người mua: permission `orders.buy` (role người mua mặc định có, phạm vi `Own`). Vãng lai: không cần
đăng nhập, giỏ nhận diện bằng header `Cart-Token` (token 32 byte ngẫu nhiên, trả một lần khi tạo, chỉ
lưu băm); đơn mở bằng token xem đơn (`Authorization: Order <token>`). Nhân viên: permission ghi ở từng
dòng, lọc theo phạm vi (`Unit` / `Subtree` / `Tenant`) qua công cụ lọc theo nhánh của bước 34. Mọi lệnh
ghi theo luật 5 (`ValidJson`, service gom lỗi).

| Method + path | Permission | Ghi chú |
|---|---|---|
| `POST /carts` | công khai / `orders.buy` | tạo giỏ cho nút bán của tên miền (`TenantContext`); vãng lai nhận `cart_token` |
| `GET /carts/{id}` | chủ giỏ | giỏ + **báo giá tính lại** (dòng, giảm kèm lý do, thuế, phí, tổng, `quote_token`, mã bị từ chối kèm lý do, `free_shipping_remaining` theo từng nơi bán — D-201, `preorder: {available_on, message, advance?}` mỗi dòng đặt trước — D-204; `advance` của giỏ khi trả trước một phần — D-232) |
| `PUT /carts/{id}/lines/{variant_id}` | chủ giỏ | `{quantity}`; 0 = xoá; quá trần → 422 `orders.quantity_invalid`; hết hàng (sản phẩm không cho bán khi hết) → 409 `orders.out_of_stock` (chỉ báo, chưa giữ chỗ) |
| `GET /carts/{id}/promotions` | chủ giỏ | mã / voucher dùng được và chưa đủ điều kiện cho giỏ (D-201, D-222, D-228) |
| `PUT /carts/{id}/codes` · `PUT /carts/{id}/vouchers` · `PUT /carts/{id}/delivery` · `PUT /carts/{id}/attribution` | chủ giỏ (voucher: chỉ người đăng nhập) | mã giảm (coupon); voucher trong ví (D-222); địa chỉ (`address_id` hay `Address`) + phương thức giao / nhận tại cửa hàng; mã tiếp thị liên kết / phiên livestream |
| `POST /carts/{id}/merge` | `orders.buy` + `Cart-Token` | gộp giỏ vãng lai vào giỏ của người mua (cộng số lượng, chặn ở trần) |
| `GET /carts?status=abandoned` | xem "Agent tự quyết" | giỏ bỏ dở (B-b9) |
| `POST /checkout` | công khai / `orders.buy` | header `Idempotency-Key` **bắt buộc**; thân `{cart_id, quote_token, payment_method, contact, invoice_request?, accept_terms_version, accept_preorder_terms?, preorder_balance_method?, note?}` → 201 `{orders[], payment_actions[]}` (D-216); `accept_preorder_terms = true` bắt buộc khi giỏ có hàng đặt trước (D-204); trả trước một phần: `payment_actions[]` mang `upfront`, `preorder_balance_method` = `payment_link` (mặc định) / `cod` (D-232); gửi lại → 200 kết quả cũ |
| `GET /public/payments/{token}` | công khai + token | trạng thái thanh toán cho màn QR; token sống bằng hạn thanh toán (D-201) |
| `GET /me/orders` · `GET /me/orders/{id}` | `orders.buy` | danh sách / chi tiết của chính mình (`Page`) |
| `GET /me/orders/{id}/documents` | `orders.buy` (bản vãng lai: token xem đơn) | hoá đơn, phiếu bảo hành, biên bản trả hàng (B-c5) |
| `POST /me/orders/claim` | `orders.buy` | gắn mọi đơn vãng lai cùng số điện thoại đã xác thực trong khách (D-201) |
| `GET /lookup/{number}` | công khai + `access` (link ký trong thư) | tra đơn vãng lai |
| `POST /public/orders/lookup` · `POST /public/orders/lookup/{id}/confirm` | công khai | `{number, phone}` → OTP SMS; `{code}` → `access_token` (D-202) |
| `POST /public/orders/{id}/cancel` · `POST /public/orders/{id}/address/otp` · `POST /public/orders/{id}/address` · `POST /public/orders/{id}/support` | token xem đơn | huỷ `{reason}`; đổi địa chỉ có OTP lần hai `{address, code}`; mở hỗ trợ `{message}` — cùng service với đường có tài khoản, nhận `Idempotency-Key` (D-202, D-204) |
| `POST /me/orders/{id}/cancel` | `orders.buy` | theo `buyer_cancel_until`; đơn trả trước một phần hoàn theo điều kiện đã chụp lúc đặt (D-232) |
| `POST /me/orders/{id}/payments` | `orders.buy` | thanh toán lại / đổi phương thức trước `payment_due_at`; trả phần còn lại của đơn trả trước một phần trước `balance_due_at` (`Idempotency-Key`, D-232) |
| `POST /me/orders/{id}/returns` | `orders.buy` | yêu cầu trả trong hạn, kèm `return_method`, `media_ids`; 201 |
| `POST /me/orders/{id}/returns/preview` | `orders.buy` | xem trước tiền hoàn, cách gửi trả và phí, thời gian hoàn, hạn — không ghi (D-202) |
| `GET /` · `GET /{id}` | `orders.read` | lọc `status`, `payment_status`, `org_id`, `channel`, `from`/`to`, `number`, `pickup_org_id`, `pickup_status`; `Tagged` có ETag |
| `POST /{id}/transitions` | `orders.manage` | `{to, reason?}`, `If-Match` bắt buộc (`if_match.required()?`) |
| `POST /{id}/shipments` | `orders.manage` | yêu cầu vận đơn (kiện, dòng, COD) → event `orders.fulfillment_requested` (payload `after.shipment` là `ShipmentRequest` của bước 44); hàng đặt trước chưa trả phần còn lại → 409 `orders.balance_unpaid` (D-232) |
| `POST /{id}/pickup/ready` · `POST /{id}/pickup/handover` | xem "Agent tự quyết" | gửi mã nhận 6 số; giao `{code \| proxy_name, proxy_phone}` (B-c7) |
| `POST /{id}/payments/manual` | `orders.manage` | ghi nhận chuyển khoản tay / thẻ tại quầy, có `reference` (D-184) |
| `POST /{id}/payment-link` | xem "Agent tự quyết" | gửi lại link thanh toán còn hạn của `ledger` (B-b9), kể cả link phần còn lại của đơn trả trước một phần (D-232) |
| `POST /{id}/refunds` | `orders.refund` | `{amount, reason, reference}`; qua cổng nếu khoản gốc qua cổng, không thì `manual` |
| `POST /{id}/lines/{line_id}/serials` | `orders.manage` | ghi số serial khi chuẩn bị hàng (mở bảo hành theo serial) |
| `POST /{id}/invoice` | `orders.manage` / `orders.buy` (của mình) | xin xuất hoá đơn (MST) khi `invoice_timing = on_request` |
| `GET /returns` · `GET /returns/{id}` · `POST /returns/{id}/transitions` | `orders.returns.manage` | duyệt / từ chối (lý do thuộc `orders.return_rejected`) / nhận hàng (tình trạng từng dòng) / hoàn tiền / đóng |
| `POST /pos/shifts` · `POST /pos/shifts/{id}/close` | `orders.pos.shift` | mở ca (tiền đầu ca), đóng ca (tiền đếm được, chênh lệch; lý do và ghi chú khi chênh lệch ≠ 0 — D-227) |
| `POST /pos/sales` | `orders.pos.sell` (phạm vi `Unit` của cửa hàng) | `{shift_id, reference, lines, buyer?, payments[{method: cash/card_terminal/vietqr, amount, approval_code?}]}` → đơn `delivered` (hoàn tất bằng job như mọi đơn); trùng `reference` → 200 kết quả cũ |
| `POST /me/orders/{id}/lines/{line_id}/review` · `PUT …/review` | `orders.buy` | chỉ dòng đã giao; sửa trong `review_edit_days` |
| `GET /public/products/{product_id}/reviews` · `GET …/reviews/media` | công khai | `Page` + thống kê sao; lọc `has_media`, `rating`, `sort=newest\|helpful`; `helpful_count`, `voted_by_me` khi đăng nhập; tóm tắt `approved` kèm nhãn AI; bộ sưu tập ảnh của người mua; ETag / cache như `content` |
| `PUT /public/reviews/{id}/helpful` · `DELETE /public/reviews/{id}/helpful` | `orders.buy` | bỏ / rút phiếu "hữu ích"; trả `{helpful_count, voted_by_me}` (D-203) |
| `GET /reviews` · `GET /reviews/count` · `POST /reviews/{id}/reply` · `POST /reviews/{id}/moderation` | `orders.reviews.moderate` | `created_after`; đếm theo `status`, `created_after`; ẩn chỉ với `reason` thuộc danh sách `orders.review_hidden`; mở lại được |
| `POST /products/{id}/review-summaries` | xem "Agent tự quyết" | enqueue job sinh bản nháp tóm tắt (D-109) |
| `POST /review-summaries/{id}/approve` · `POST /review-summaries/{id}/reject` | `orders.reviews.moderate` | duyệt / từ chối bản nháp |
| `GET /risk/buyers/{buyer_key}` · `GET/POST /risk/cod-rules` · `GET/POST/DELETE /risk/cod-blocks` | `orders.risk.manage` | thống kê, quy tắc có ngày hiệu lực, danh sách chặn có lý do và hạn |
| `GET /warranties?serial=&phone=` · `POST /warranty-claims` · `POST /warranty-claims/{id}/transitions` | `orders.warranty.manage` | tra, tiếp nhận, xử lý |
| `GET /me/warranties` | `orders.buy` | bảo hành của mình |
| `GET /reports/sales` | `orders.reports.read` | `group_by=org/channel/product/day/month`, `from`, `to`, `currency`; trong phạm vi quyền |
| `GET/PUT /policies/{org_id}` | `orders.settings.manage` | chính sách theo nút (kế thừa) |

Mọi handler có `///` một dòng, nêu permission, `description` cho từng response kèm `code` lỗi.

### Tool MCP

Cùng service, cùng permission với REST (luật 8): `list_orders`, `get_order` (`read_only`),
`transition_order`, `request_shipment`, `record_manual_payment`, `refund_order` (`destructive`),
`list_returns`, `decide_return` (trả kèm `media_ids` để nhân viên xem ảnh — D-202), `sales_report`
(`read_only`), `list_reviews` (trả kèm `helpful_count` — D-203), `reply_review`, `moderate_review`,
`lookup_warranty` (`read_only`), `update_warranty_claim`. `get_order` trả kèm khối `advance` (trả trước, phần
còn lại, hạn, trạng thái — D-232). Không có tool cho checkout của người mua hay POS
(giao diện người dùng cuối, không phải việc của agent quản trị), cho xem trước tiền hoàn, bỏ phiếu "hữu
ích" hay token xem đơn (việc của người mua; nhân viên dùng tool sẵn có).

### Event (`Module::events`, payload `{"before":…, "after":…}`, luôn có `tenant_id`, `org_id`, `channel`, tiền)

`orders.order_placed`, `orders.order_confirmed` (kèm `hold_reference`, dòng, kho), `orders.order_status_changed`
(mọi chuyển), `orders.order_cancelled` (kèm `hold_reference`; `cause = seller_not_accepted` khi quá hạn chờ
nhận đơn — D-209), `orders.fulfillment_requested`, `orders.return_pickup_requested` (trả hàng cần hãng tới lấy),
`orders.order_delivered`, `orders.order_completed`, `orders.stock_shortage` (đơn nhập từ kênh thiếu hàng),
`orders.payment_recorded`, `orders.refund_completed` (dòng và số tiền hoàn), `orders.return_requested` (kèm
`photo_count`, không có ID ảnh — D-202), `orders.return_received` (dòng nhập lại kho / hỏng), `orders.invoice_linked`,
`orders.cart_abandoned` (người mua, tóm tắt giỏ — cho bước 47), `orders.review_published`,
`orders.review_moderated`, `orders.pos_shift_closed`, `orders.warranty_claim_changed`,
`orders.cod_block_changed`, `orders.preorder_balance_requested` (số tiền, hạn — cho thông báo),
`orders.preorder_balance_overdue` (D-232). Không mang secret, token, `Cart-Token`. Không có event cho từng phiếu "hữu
ích" (D-203) và không có `orders.support_requested` (D-204).

Nghe: event thanh toán xong của bước 36 (lọc `purpose = "orders"`), `shipping.shipment_*`,
`shipping.cod_collected`, `invoices.invoice_issued`, `catalog.preorder_date_changed`,
`inventory.preorder_allocated` (D-204), `privacy::USER_DELETED`; tự nghe `orders.order_*` để gửi thông báo
(gọi `Notifier` với `reference = <event id>`, chống gửi trùng).

### Job và lịch

| Job | Lịch | Việc |
|---|---|---|
| `orders.expire_unpaid` | mỗi phút | `pending_payment` quá `payment_due_at` (hạn của `ledger`) → `cancelled` (lô 100) |
| `orders.recover_checkouts` | mỗi phút | `reserving` quá `checkout_stuck_minutes` → `release` theo ref, `failed` |
| `orders.complete_delivered` | mỗi giờ | `delivered` quá hạn đổi trả, không có hold → `completed` |
| `orders.expire_pickups` | mỗi giờ | đơn chờ nhận quá `pickup_hold_until` → `pickup_expired`, nhắc một lần trước 24 giờ, rồi theo `pickup_expired_action` |
| `orders.post_ledger` | enqueue cùng transaction, `unique_key = <ref bút toán>` | gọi `Ledger::post` (không trùng nhờ ref) |
| `orders.issue_invoice` · `orders.adjust_invoice` | enqueue | gọi `Invoices` (mạng, ngoài transaction) |
| `orders.refund` | enqueue | gọi `PaymentGateway::refund`, cập nhật `orders_refunds` |
| `orders.grant_digital` | enqueue khi `confirmed` | `DigitalDelivery::grant`, rồi `confirmed → delivered` |
| job tóm tắt đánh giá | enqueue từ `POST /products/{id}/review-summaries` | gọi `TextModel` ngoài transaction, ghi bản `draft` (D-109) |
| `orders.mark_abandoned_carts` | mỗi 10 phút | giỏ có người mua / email, không đổi quá `abandoned_after_minutes` → `orders.cart_abandoned` một lần |
| `orders.purge_carts` | mỗi ngày | xoá giỏ quá `cart_ttl_days` |
| `orders.expire_warranties` | mỗi ngày | `active` quá `ends_at` → `expired` |
| `orders.request_preorder_balance` | enqueue khi plan sang `balance_requested`, `unique_key = <order_id>:balance:<attempt>` | tạo thanh toán phần còn lại ở `ledger` (ngoài transaction), báo người mua kèm link (D-232) |
| `orders.expire_preorder_balances` | mỗi giờ | plan `balance_requested` quá `balance_due_at` → `balance_overdue`, rồi theo `preorder_balance_overdue_action` (lô 100, D-232) |

### Cấu hình (`Setting`, tiền tố `AUTOXIDE_ORDERS_`)

`HOLD_GRACE_MINUTES` (10 — chỗ giữ sống lâu hơn hạn thanh toán, để đơn được huỷ trước khi chỗ tự hết),
`CHECKOUT_STUCK_MINUTES` (10), `CART_TTL_DAYS` (30), `CART_MAX_LINES` (100), `LINE_MAX_QUANTITY` (999),
`ABANDONED_AFTER_MINUTES` (60), `REVIEW_EDIT_DAYS` (30), `QUOTE_TTL_MINUTES` (15), `LOOKUP_PER_HOUR` (5 theo
cặp mã đơn + số điện thoại băm; 20 theo IP băm), `LOOKUP_RESEND_SECONDS` (60), `LOOKUP_TOKEN_MINUTES` (30),
`REVIEW_VOTES_PER_HOUR` (60). Hạn thanh toán không có biến riêng: đọc từ `ledger`
(`AUTOXIDE_LEDGER_PAYMENT_EXPIRY_MINUTES`, D-218). Chính sách kinh doanh (hạn đổi trả, huỷ, COD, hoá đơn, tiền
tố số đơn, chờ nhận đơn, lý do trả, ảnh bắt buộc, đặt trước, trả trước một phần — D-232, giữ hàng chờ nhận)
là dữ liệu theo nút trong `orders_policies`, không phải biến môi trường. Sinh lại `.env.example`.

Tính năng (`Module::features`): `orders.pos`, `orders.reviews`, `orders.installment`,
`orders.warranty`, `orders.cod_risk`. Hạn mức (`Module::limits`): `orders.orders_per_month` (đo dùng,
giữ chỗ khi tạo đơn qua `Entitlements::reserve`, trả lại khi checkout hỏng).

### Permission

`orders.buy`, `orders.read`, `orders.manage`, `orders.refund`, `orders.returns.manage`,
`orders.pos.sell`, `orders.pos.shift`, `orders.reviews.moderate`, `orders.risk.manage`,
`orders.warranty.manage`, `orders.reports.read`, `orders.settings.manage`. Ở `media` (nếu bước này thêm):
`media.upload_own` (D-202).

### Mã lỗi (code → `locales/vi.json`)

| Code | HTTP | Tiếng Việt |
|---|---|---|
| `orders.cart_not_found` | 404 | Không tìm thấy giỏ hàng. |
| `orders.cart_token_invalid` | 401 | Mã giỏ hàng không đúng hoặc đã hết hạn. |
| `orders.cart_empty` | 422 | Giỏ hàng đang trống. |
| `orders.cart_too_many_lines` | 422 | Giỏ hàng có tối đa {max} sản phẩm. |
| `orders.quantity_invalid` | 422 | Số lượng phải từ 1 đến {max}. |
| `orders.variant_unavailable` | 422 | Sản phẩm {sku} hiện không bán. |
| `orders.out_of_stock` | 409 | Sản phẩm {sku} không đủ hàng (còn {available}). |
| `orders.promotion_unavailable` | 409 | Khuyến mại {code} đã hết lượt hoặc hết hạn. |
| `orders.quote_changed` | 409 | Giá hoặc phí đã thay đổi; hãy xem lại giỏ hàng trước khi đặt. |
| `orders.quote_expired` | 409 | Báo giá đã hết hạn; hãy tải lại giỏ hàng. |
| `orders.idempotency_key_required` | 428 | Đặt hàng cần header `Idempotency-Key`. |
| `orders.delivery_required` | 422 | Cần địa chỉ và cách giao hàng cho đơn có hàng phải giao. |
| `orders.payment_method_unavailable` | 422 | Phương thức thanh toán {method} không dùng được cho đơn này. |
| `orders.cod_not_allowed` | 422 | Đơn này cần thanh toán trước, không nhận thanh toán khi giao. |
| `orders.terms_not_accepted` | 422 | Cần đồng ý điều kiện giao dịch chung phiên bản {version}. |
| `orders.preorder_terms_required` | 422 | Cần đồng ý điều kiện mua hàng đặt trước. |
| `orders.preorder_sold_out` | 409 | Hàng đặt trước đã hết suất. |
| `orders.preorder_closed` | 409 | Đã quá thời hạn đặt trước. |
| `orders.order_not_found` | 404 | Không tìm thấy đơn hàng. |
| `orders.invalid_transition` | 409 | Đơn đang ở trạng thái {from}, không chuyển sang {to} được. |
| `orders.cancel_not_allowed` | 409 | Đơn đã qua bước được tự huỷ; hãy liên hệ cửa hàng. |
| `orders.payment_expired` | 409 | Đơn đã quá hạn thanh toán. |
| `orders.refund_exceeds_paid` | 422 | Số tiền hoàn ({amount}) lớn hơn số đã thu còn lại ({remaining}). |
| `orders.return_window_closed` | 409 | Đã quá hạn đổi trả của đơn ({days} ngày). |
| `orders.return_quantity_invalid` | 422 | Số lượng trả của dòng {line} vượt số đã mua. |
| `orders.return_not_found` | 404 | Không tìm thấy yêu cầu đổi trả. |
| `orders.return_photos_required` | 422 | Lý do trả này cần kèm ảnh. |
| `orders.return_photo_not_owned` | 422 | Ảnh không phải do bạn tải lên. |
| `orders.return_too_many_photos` | 422 | Yêu cầu trả hàng có tối đa {max} ảnh. |
| `orders.lookup_failed` | 404 | Không tìm thấy đơn với số đơn và số điện thoại này. |
| `orders.lookup_resend_too_soon` | 429 | Hãy chờ {retry_after} giây trước khi gửi lại mã. |
| `orders.lookup_token_invalid` | 401 | Mã xem đơn không đúng hoặc đã hết hạn. |
| `orders.address_locked` | 409 | Hàng đã rời kho, không đổi địa chỉ được nữa. |
| `orders.address_change_needs_payment` | 409 | Địa chỉ mới làm đổi phí giao; cần thanh toán phần chênh lệch. |
| `orders.support_unavailable` | 404 | Cửa hàng chưa bật hỗ trợ khách hàng. |
| `orders.shift_not_open` | 409 | Ca bán hàng chưa mở hoặc đã đóng. |
| `orders.shift_already_open` | 409 | Quầy {register} đang có ca mở. |
| `orders.payments_mismatch` | 422 | Tổng các khoản thanh toán ({paid}) khác tổng đơn ({total}). |
| `orders.review_not_allowed` | 403 | Chỉ người đã nhận hàng mới đánh giá được sản phẩm này. |
| `orders.review_exists` | 409 | Bạn đã đánh giá sản phẩm này trong đơn này. |
| `orders.review_edit_closed` | 409 | Đã quá thời hạn sửa đánh giá. |
| `orders.review_not_found` | 404 | Không tìm thấy đánh giá. |
| `orders.review_vote_own` | 409 | Không bỏ phiếu cho đánh giá của chính mình. |
| `orders.review_vote_not_allowed` | 403 | Người bán, người kiểm duyệt không bỏ phiếu cho sản phẩm của mình. |
| `orders.moderation_reason_invalid` | 422 | Lý do ẩn đánh giá không thuộc các lý do được phép. |
| `orders.account_not_verified` | 403 | Cần xác thực số điện thoại trước khi đánh giá. |
| `orders.warranty_not_found` | 404 | Không tìm thấy bảo hành cho số serial hoặc đơn này. |
| `orders.warranty_expired` | 409 | Bảo hành đã hết hạn ngày {date}. |
| `orders.serial_invalid` | 422 | Số serial {serial} không hợp lệ cho dòng hàng này. |
| `orders.preorder_advance_invalid` | 422 | Trả trước một phần cần đúng một trong tỉ lệ hoặc số tiền cố định, trong khoảng sàn và trần. |
| `orders.balance_unpaid` | 409 | Đơn còn {amount} chưa thanh toán; chưa giao được hàng đặt trước. |
| `orders.balance_overdue` | 409 | Đã quá hạn thanh toán phần còn lại của đơn. |

Nguồn mã mới: đặt trước (D-204; `preorder_sold_out` đến từ `inventory`, `preorder_closed` khi quá
`preorder_ends_at`); ảnh trả hàng — trường `media_ids`, gom chung một lần (D-202, D-083); tra đơn vãng lai
(D-202; `lookup_failed` chung cho không thấy đơn / sai số, `lookup_resend_too_soon` có `retry_after`);
`support_unavailable` (D-204); phiếu "hữu ích" (D-203); trả trước một phần (D-232; `balance_overdue` khi trả
phần còn lại sau `balance_due_at`, phương thức không dùng được dùng lại `orders.payment_method_unavailable`).
Câu tiếng Việt là bản gợi ý — đặc tả gốc chỉ nêu ý.

## Thứ tự làm

Mỗi dòng là một commit (chạy `./gate.sh --quick` trước mỗi commit); "mục N" là mục của bước 45 trong ROADMAP:

1. Khung crate, permission, cấu hình, `api.rs` (kiểu + trait + `OrderExtensions` + `OrderLookups`), đăng ký
   module, danh sách lý do. → chuẩn bị cho mọi mục
2. Giỏ hàng: bảng, token vãng lai, dòng, báo giá tính lại ở máy chủ (`Catalog`, `Promotions`,
   `ShippingQuotes`, `TaxEngine`), `quote_token`, gộp giỏ; coupon / voucher và dữ liệu giỏ cho `promotions`,
   `free_shipping_remaining`; giỏ bỏ dở. → **mục 1 (giỏ hàng)**
3. Checkout saga: `orders_checkouts`, giữ chỗ, `numbering`, tạo đơn, `Idempotency-Key` bắt buộc +
   `reference`, `recover_checkouts`, `payment_actions[]`. → **mục 2 (đặt hàng)**
4. Thanh toán theo chế độ dòng tiền: `orders_payments`, gateway / VietQR / COD / chuyển khoản tay,
   nghe event thanh toán, hạn thanh toán từ `ledger`, `expire_unpaid`, thanh toán muộn sau khi huỷ → hoàn tự
   động; trạng thái thanh toán công khai; gửi lại link thanh toán. → **mục 2, 3**
5. Máy trạng thái + lịch sử + event + job sổ cái (bảng bút toán ở "Chỗ dễ sai"), vận đơn qua event
   của `shipping`, hàng số, `complete_delivered`, chờ nhận đơn (`acceptance_hours`), tool MCP quản trị đơn.
   → **mục 3 (vòng đời)**
6. Nhận tại cửa hàng (mã nhận, người nhận hộ, `expire_pickups`) và hàng đặt trước. → **mục 4**
7. Đổi trả, hoàn tiền, bút toán đảo, hoá đơn điều chỉnh; cách trả, ảnh kèm, xem trước tiền hoàn, hoàn theo
   tỉ lệ khoản giảm. → **mục 5, 6**
8. Hoá đơn điện tử (`issue_invoice`, hoá đơn từ máy tính tiền, `invoice_timing`), thông báo qua `Notifier`
   theo i18n, chứng từ của người mua. → **mục 7**
9. Mua không cần tài khoản: tra đơn bằng OTP, token xem đơn, thao tác vãng lai, `claim`, `OrderLookups`, phiếu
   hỗ trợ. → **mục 8**
10. POS: ca, bán tại quầy, trừ kho cửa hàng, `reference` cho client ngoại tuyến, đóng ca có chênh lệch. →
    **mục 9**
11. Đánh giá (`reviews/`, ghi quyết định reviews ở `orders` thay vì `comments`), lọc công khai, ảnh, lý do ẩn,
    đánh giá mới, nhãn `verified_buyer`. → **mục 10**
12. Phiếu "hữu ích" và bản nháp tóm tắt bằng AI. → **mục 11**
13. Báo cáo bán hàng + kiểm payload event đủ cho bước 38; khai xuất đơn qua `Module::exports`. → **mục 12**
14. Chống bom hàng COD (kèm `contact_key` của kernel). → **mục 13**
15. Trả góp + bảo hành. → **mục 14**
16. Trả trước một phần cho hàng đặt trước: cột chính sách, `orders_advance_plans`, tính `advance` / `upfront` /
    `balance`, công bố trong báo giá và chụp điều kiện, thu phần còn lại (link / COD), quá hạn, huỷ và hoàn theo
    mốc, bút toán `orders.customer_advance` (D-232). → **mục 4**
17. Hướng dẫn (nếu có tài liệu người ngoài), `docs/openapi.json`, `.env.example`, CHANGELOG, ROADMAP.

## Test bắt buộc

Mọi test qua `TestDatabase::run` / `TestApp::builder(test)`, chạy trên SQLite và Postgres. Dùng module
thật `catalog`, `inventory`, `promotions`, `shipping`, `ledger`, `tax`, `invoices` trong `TestApp`;
cổng thanh toán, hoá đơn điện tử, `Notifier` là bộ giả của testkit; lỗi giữa chừng tiêm bằng
`.provide::<dyn Inventory>(…)` bọc bản thật.

- `cart_prices_are_recomputed_ignoring_client_numbers` — client gửi giá / tổng sai: bị bỏ qua.
- `guest_cart_merges_into_buyer_cart_on_login`; `cart_token_from_other_tenant_is_rejected`.
- `checkout_requires_idempotency_key` (428); `checkout_replays_same_key_after_24_hours` (key HTTP đã
  hết hạn — dịch thời gian / xoá bảng idempotency — vẫn trả đơn cũ); `same_key_different_cart_is_422`.
- `concurrent_checkouts_same_key_create_one_order` — 16 request cùng key → 1 đơn, 1 chỗ giữ.
- `last_unit_concurrent_checkouts` — 50 người mua 1 cái cuối: đúng 1 đơn, 49 nhận `orders.out_of_stock`.
- `promotion_hold_failure_releases_stock_hold` — tiêm lỗi ở bước 5: chỗ giữ tồn được trả ngay.
- `crash_after_reserve_is_recovered_by_job` — dừng sau bước 4: `recover_checkouts` trả chỗ.
- `order_numbers_have_no_gaps_under_concurrency` — 40 checkout song song (có cái hỏng ở giữ chỗ):
  dãy số liền, không trùng.
- `quote_changed_between_quote_and_checkout` — đổi giá giữa hai lần gọi → 409, không đơn.
- `unpaid_order_expires_and_releases_holds`; `late_payment_after_cancel_is_refunded`.
- `duplicate_payment_webhook_confirms_once` — event thanh toán giao hai lần, song song: một lần
  `confirmed`, một bút toán, một `orders.order_confirmed`.
- `cancel_races_payment` — huỷ và thanh toán cùng lúc: hoặc `confirmed`, hoặc `cancelled` + hoàn tiền;
  không bao giờ cả hai.
- `every_transition_in_table_is_allowed_others_rejected` — duyệt mọi cặp trạng thái.
- `status_is_only_changed_by_lifecycle` — đọc mã nguồn crate: không có chỗ nào khác gán `status`.
- `ledger_entries_balance_per_order` — mỗi đơn qua đủ vòng đời (tự thu, COD, VietQR): tổng nợ = có;
  hoàn một phần → bút toán đảo đúng số.
- `partial_return_creates_adjustment_invoice_and_restock_event`.
- `refund_cannot_exceed_paid_concurrently` — 10 lệnh hoàn song song: tổng ≤ đã thu.
- Coupon, voucher (D-222): `voucher_cua_nguoi_khac_bi_tu_choi`, `doi_phuong_thuc_thanh_toan_tinh_lai_giam`,
  `tra_mot_phan_hoan_theo_ti_le_tong_khop`, `tra_mon_dieu_kien_tru_gia_tri_qua`, `huy_don_tra_voucher_vao_vi`.
- Hàng đặt trước (D-204): `don_tron_hang_san_va_dat_truoc_tach_hai_lan_giao`,
  `doi_ngay_dat_truoc_bao_nguoi_mua_va_cho_huy_mien_phi`, `dat_truoc_phai_dong_y_dieu_kien`,
  `ngung_dat_truoc_huy_va_hoan_tien_tu_dong`.
- Trả trước một phần (D-232): `tra_truoc_dung_ti_le_va_kep_tran_san`, `phan_con_lai_thu_khi_hang_san_sang`,
  `phan_con_lai_cod_khi_chinh_sach_cho_phep`, `chua_tra_phan_con_lai_khong_giao_hang_dat_truoc`,
  `nguoi_ban_huy_hoan_du_tra_truoc`, `ngung_dat_truoc_hoac_doi_ngay_qua_gioi_han_hoan_du`,
  `nguoi_mua_huy_truoc_moc_hoan_du`, `nguoi_mua_huy_sau_moc_giu_lai_theo_ti_le_co_tran`,
  `link_phan_con_lai_qua_han_theo_chinh_sach`, `thanh_toan_phan_con_lai_goi_lai_khong_thu_hai_lan` (cùng
  `Idempotency-Key` gửi song song; hai lần thử cùng thành công → lần sau tự hoàn),
  `tong_tien_thu_bang_gia_don_tung_dong` (thuộc tính: mọi tỉ lệ, trần, sàn, đơn trộn hàng sẵn — `upfront +
  balance = total`, bút toán cân), `doi_chinh_sach_sau_khi_dat_khong_ap_nguoc`.
- Trả hàng kèm ảnh, xem trước (D-202): `ly_do_hong_vo_bat_buoc_anh`, `anh_cua_nguoi_khac_bi_tu_choi`,
  `xem_truoc_tien_hoan_khong_ghi_gi`, `chinh_sach_anh_ke_thua_theo_cay`.
- Đơn vãng lai (D-202): `tra_don_sai_so_va_khong_co_don_tra_loi_giong_nhau`, `gui_lai_otp_phai_cho`,
  `token_xem_don_het_han_sau_30_phut`, `huy_don_vang_lai_cung_chinh_sach`, `doi_dia_chi_can_otp_lan_hai`,
  `response_che_so_dien_thoai_va_dia_chi`, `thao_tac_vang_lai_goi_lai_cung_idempotency_key`.
- `pos_sale_with_same_reference_twice_is_one_order`; `pos_shift_close_reports_cash_difference`.
- `review_only_by_buyer_with_delivered_line`; `negative_review_cannot_be_hidden_without_allowed_reason`;
  `review_stats_consistent_under_concurrent_reviews`.
- Nhãn hỏi đáp (D-202): `nguoi_da_nhan_hang_co_nhan_verified_buyer`, `don_bi_huy_khong_cho_nhan`.
- Phiếu "hữu ích" (D-203): `phieu_huu_ich_moi_nguoi_mot_lan`, `khong_bo_phieu_cho_danh_gia_cua_minh`,
  `nguoi_ban_khong_bo_phieu_cho_san_pham_cua_minh`, `dem_huu_ich_dung_khi_bo_phieu_dong_thoi` (nhiều người
  cùng lúc, cùng người gửi lặp), `sap_theo_huu_ich_lat_trang_khong_trung_khong_sot`,
  `xoa_tai_khoan_xoa_phieu_va_tru_dem`.
- `cod_rule_requires_prepayment_for_high_risk_buyer`; `cod_stats_not_visible_across_tenants`
  (cùng SĐT ở hai khách → hai `buyer_key` khác nhau).
- `warranty_lookup_by_serial_and_expiry`.
- `sales_report_respects_scope` — nhân viên cửa hàng A không thấy số của B; `Subtree` ở vùng thấy cả hai.
- Rò rỉ: `tenant_isolation` (bài chung của testkit cho mọi bảng), `sibling_store_isolation` (đơn, đổi
  trả, đánh giá chờ, báo cáo).
- `user_deleted_keeps_accounting_fields` — xoá tài khoản: giỏ, đánh giá xoá; đơn giữ phần kế toán,
  bỏ SĐT / email liên hệ.
- `openapi_every_endpoint_described` (bài sẵn có) và `schema_ownership_violations`.

## Tiêu chí xong

- [ ] Mọi mục của bước 45 trong ROADMAP có commit tương ứng (bảng "Thứ tự làm") và test ở trên xanh.
- [ ] Không có transaction xuyên module; không đọc bảng module khác; mọi thao tác giữ chỗ có test đồng
      thời trên cả hai database.
- [ ] `docs/openapi.json`, `.env.example` sinh lại; `cargo xtask check` xanh (dịch đủ `vi.json`).
- [ ] Quyết định ghi vào `docs/DECISIONS.md`: đánh giá đặt ở `orders`; quy ước "giữ chỗ gọi trực tiếp,
      xác nhận / trả qua event"; `OrderExtensions`; bảng bút toán theo chế độ dòng tiền (gồm
      `orders.customer_advance`, `orders.cancellation_income` — D-232); căn cứ pháp lý của đánh giá, đổi trả,
      hoá đơn điều chỉnh, chống bom hàng, trả trước một phần.
- [ ] `CHANGELOG.md` (Added); đánh dấu ROADMAP; `./gate.sh` đầy đủ xanh; phát hành theo
      `docs/RELEASING.md` khi cả bước xong.

## Chỗ dễ sai

- **Bút toán theo chế độ dòng tiền** (tài khoản theo khoá của `ledger` bước 36, có `org_id`). Tên tài khoản dưới
  đây là tên tạm; khi làm bước này viết lại theo bộ tên duy nhất của đặc tả 36 (`clearing.gateway`,
  `clearing.carrier_cod`, `revenue.*`…), như D-218 yêu cầu cho 46:
  - Thu online / VietQR / chuyển khoản (mọi chế độ): Nợ `payment_clearing:<gateway>` / Có
    `buyer_prepayments`. COD khi giao xong: Nợ `cod_receivable:<carrier>` / Có `buyer_prepayments`
    (đối soát với hãng là việc của `shipping`). Tiền mặt POS: Nợ `cash_drawer:<org>`.
  - Đơn kênh (`funds_flow = ChannelCollect`): Nợ `channel_receivable:<account>` / Có `buyer_prepayments`
    lúc nhập; `channels` (bước 48) đối soát khoản kênh trả.
  - Ghi doanh thu khi `delivered` **chỉ khi `funds_flow` là `SelfCollect` hay `ChannelCollect`**: Nợ `buyer_prepayments` /
    Có `sales_revenue` (net trước thuế), Có `vat_output` (từ `orders_line_taxes`), Có
    `shipping_revenue`. Chế độ `PartnerCollect` / `GatewaySplit`: `marketplace` (bước 46) ghi phần
    chia khi nghe `orders.order_completed` — `orders` không ghi doanh thu của người bán.
  - Hoàn tiền: bút toán đảo đúng phần đã ghi, mã tham chiếu trỏ về bút toán gốc; không sửa bút toán cũ.
  - Chênh lệch tiền két khi đóng ca: `suspense.cash_shortage` / `suspense.cash_overage` (D-227).
  - Khoản người mua trả trước (D-232): tên theo bộ của đặc tả 36 cho `buyer_prepayments` ở trên là
    `orders.customer_advance` (mục đích riêng của module, tiền tố `orders.`; ánh xạ số hiệu tài khoản là dữ liệu
    của `ledger`, vd dư Có TK 131 "người mua trả tiền trước" — **cần đối chiếu** TT 99/2025, TT 133/2016).
    Trả trước một phần và phần còn lại online: Nợ `clearing.gateway` / Có `orders.customer_advance`; phần còn
    lại COD: Nợ `clearing.carrier_cod` / Có `orders.customer_advance`; doanh thu khi `delivered` như trên.
    Hoàn: Nợ `orders.customer_advance` / Có `clearing.gateway`. Phần giữ lại khi người mua huỷ sau mốc: Nợ
    `orders.customer_advance` / Có `orders.cancellation_income` (thu nhập khác — thuế GTGT của khoản này
    **cần đối chiếu**). Chế độ `PartnerCollect` / `GatewaySplit`: phần trả trước theo bảng của đặc tả 36 (Có
    `payable.seller_held[S]`), huỷ thì hoàn từ tài khoản thu hộ của đối tác.
- **Hết hạn chỗ giữ trước khi huỷ đơn**: chỗ giữ phải sống tới `payment_due_at + HOLD_GRACE_MINUTES`;
  `expire_unpaid` chạy mỗi phút nên đơn luôn bị huỷ trước. Thanh toán tới sau khi đã huỷ → không mở
  lại đơn, tạo hoàn tiền tự động (`late_payment`).
- **Hai hạn thanh toán**: không thêm hạn riêng ở `orders` — `payment_due_at` luôn là `expires_at` của
  `ledger` (D-218).
- **Huỷ đua với thanh toán**: cả hai đi qua `Lifecycle::apply` có `rows::lock_one` trên dòng đơn; bên
  sau thấy trạng thái mới và làm nhánh đúng (thanh toán sau huỷ → hoàn).
- **Số đơn không nhảy**: chỉ lấy số trong transaction tạo đơn (bước 7); lấy trước khi giữ chỗ sẽ đốt số
  khi giữ chỗ hỏng. Transaction này giữ khoá dãy số — phải ngắn: không gọi mạng, không gọi `api` module
  khác bên trong.
- **Không gọi mạng trong transaction ghi**: cổng thanh toán, hoá đơn, thông báo, `Ledger::post` (có
  thể ở database khác), `TextModel` đều qua job enqueue cùng transaction.
- **Idempotency hai lớp**: lớp HTTP của kernel bỏ qua request chưa xác thực (`http/idempotency.rs`) —
  checkout vãng lai chỉ được chống trùng bởi `orders_checkouts.reference`; không dựa vào lớp HTTP.
  Fingerprint của `reference` tính trên nội dung nghiệp vụ (giỏ, `quote_token`, phương thức), không
  trên thân request thô.
- **Nhận lại event**: event thanh toán, `shipping.*` có thể tới hai lần hoặc sai thứ tự (`delivered`
  tới trước `picked_up`): `Lifecycle` coi chuyển đã ở trạng thái đích là thành công không làm gì, và
  cho phép nhảy `preparing → delivered` khi event giao xong tới trước (ghi lịch sử đủ hai bước).
- **Tràn và làm tròn**: cộng tiền bằng `Money::checked_add`; giảm giá chia xuống dòng bằng
  `Money::allocate` (tổng không lệch một đồng); thuế lấy nguyên từ `TaxEngine`, không tự nhân lại.
  Tổng đơn = Σ net dòng + phí giao − giảm phí giao; kiểm bằng test thuộc tính. Hoàn khi trả hàng cũng chia
  bằng `Money::allocate`, tổng hoàn không vượt tiền đã trả (D-222).
- **Trả trước một phần (D-232)**: không gọi là "cọc", không áp chế tài đặt cọc; `balance` tính bằng phép trừ
  (`total − upfront`), không nhân tỉ lệ hai lần — lệch một đồng là lỗi; điều kiện huỷ đọc từ `terms` đã chụp,
  không đọc chính sách hiện tại; phần giữ lại không vượt phần trả trước của dòng; `balance_due_at` là hạn của
  nghĩa vụ trả phần còn lại, **không** phải hạn thanh toán thứ hai — mỗi link vẫn sống bằng hạn của `ledger`
  (D-218); core không giữ tiền, kể cả phần giữ lại (D-187).
- **Unique có NULL**: dùng cột khoá "đang hoạt động" thay vì index một phần.
- **SQLite một người ghi**: saga gồm nhiều transaction ngắn — không mở transaction dài bao cả saga.
- **Đánh giá**: không có đường ẩn đánh giá nào ngoài `moderation` với lý do trong danh sách
  `orders.review_hidden`; điểm trung bình tính trên mọi đánh giá `published`; ẩn phải ghi người ẩn, lý do,
  và báo người viết.
- **Đếm "hữu ích"**: đổi bằng `UPDATE … ± 1` trong transaction của phiếu, không đọc rồi ghi; `voted_by_me`
  của cả trang lấy bằng một câu truy vấn; xoá tài khoản nhận lại event không trừ hai lần (D-203).
- **Tóm tắt AI**: chỉ bản `approved` ra công khai, kèm nhãn AI; không lược ý kiến tiêu cực (D-109).
- **Dò số đơn**: tra đơn sai số và không có đơn trả lời giống hệt nhau; token xem đơn chỉ mở một đơn và
  response che dữ liệu cá nhân (D-202).
- **Chống bom hàng**: `buyer_key` là HMAC-SHA256 của SĐT chuẩn hoá (E.164) bằng khoá theo khách dẫn
  xuất từ kho secret của bước 33 — không so được giữa hai khách. **API mới cần thêm** ở kernel để
  `customers` (47) dùng cùng cách: `autoxide_kernel::tenancy::contact_key(secrets: &dyn TenantSecrets,
  tenant: TenantId, kind: ContactKind /* Phone | Email */, raw: &str) -> Result<String, Error>`
  (phác thảo; chuẩn hoá SĐT Việt Nam `0…` → `+84…` nằm trong hàm); không trả thống kê cho người mua khác; không chặn vĩnh viễn (luôn có
  `expires_at`).

## Căn cứ pháp lý

- Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15 và NĐ 55/2024 (từ 01/7/2024): không chặn / ẩn
  đánh giá trái quy định; minh bạch điều kiện giao dịch chung (lưu `terms_version` người mua đã đồng ý,
  qua `compliance`); hiển thị rõ điều kiện khuyến mại — cần đối chiếu điều cụ thể. Thời gian giao dự kiến
  của hàng đặt trước phải hiện rõ trước khi đặt (D-204) — cần đối chiếu. Sắp xếp theo "hữu ích" không làm
  mặc định, không có phiếu "không hữu ích" (D-203) — cần đối chiếu văn bản gốc xem sắp xếp có thuộc phạm vi
  không.
- Luật Giao dịch điện tử 20/2023: bằng chứng chấp nhận điều kiện giao dịch.
- NĐ 123/2020 sửa bởi NĐ 70/2025 (từ 01/6/2025): hoá đơn điện tử, hoá đơn từ máy tính tiền, xử lý hoá
  đơn khi trả hàng (điều chỉnh / thay thế) — cần đối chiếu điều về trả hàng trước khi chạy thật. Điều 9
  (thời điểm lập hoá đơn khi bán hàng hoá là thời điểm chuyển giao quyền sở hữu hay quyền sử dụng): căn cứ
  cho `invoice_timing = on_delivered` mặc định (D-215) — **cần đối chiếu văn bản gốc**. Ghi khoản giảm, hàng
  khuyến mại, phần do sàn / đối tác trả trên hoá đơn (D-222) — **cần đối chiếu** và cần người có chuyên môn
  xác nhận trước khi chạy thật.
- Luật Giá 16/2023: giá niêm yết bằng Đồng Việt Nam, gồm thuế khi bán cho người tiêu dùng — cần đối chiếu.
- Luật Kế toán 88/2015 Điều 41, NĐ 174/2016: đơn, chứng từ, bút toán lưu ít nhất 10 năm (D-190). Chênh lệch
  tiền két vào tài khoản chờ xử lý: chế độ kế toán doanh nghiệp TT 200/2014, TT 133/2016 (tài khoản 1381 /
  3381, D-227) — cần đối chiếu.
- Luật BVDLCN 91/2025 + NĐ 356/2025: thống kê giao hàng để chống gian lận cần căn cứ xử lý ghi trong sổ
  của `compliance` (vd thực hiện hợp đồng / lợi ích hợp pháp — cần người có chuyên môn xác nhận), chỉ
  dùng trong khách, có thời hạn lưu. Che số điện thoại, tên, địa chỉ khi xem đơn bằng token: Luật 91/2025/QH15,
  NĐ 13/2023/NĐ-CP (nguyên tắc tối thiểu hoá) — **cần đối chiếu văn bản gốc** (D-202).
- NĐ 147/2024: khách bật yêu cầu xác thực tài khoản cho đánh giá.
- NĐ 52/2024: tiền đi qua đối tác có giấy phép; `orders` không giữ tiền (D-187); QR không bao giờ trỏ tới tài
  khoản của nền tảng (D-201, D-216).
- Trả trước một phần cho hàng đặt trước (D-232): Bộ luật Dân sự 2015 Điều 328 (đặt cọc) — core **không** coi
  khoản trả trước là đặt cọc; điều khoản đặt cọc là chính sách riêng của khách — **cần đối chiếu văn bản gốc /
  người có chuyên môn pháp lý**. Công bố điều kiện huỷ, phần giữ lại trước khi đặt và lưu bằng chứng đồng ý:
  Luật 19/2023/QH15 (điều kiện giao dịch chung), Luật Giao dịch điện tử 20/2023 — cần đối chiếu điều cụ thể.
  Thời điểm lập hoá đơn với khoản thu trước: NĐ 123/2020 sửa bởi NĐ 70/2025, Điều 9 — **cần đối chiếu**, để
  dạng dữ liệu (`advance_invoice_timing`). Thuế GTGT của phần giữ lại khi người mua huỷ: Luật Thuế GTGT
  48/2024/QH15 và văn bản hướng dẫn — **cần đối chiếu**.
- Bảo hành: Luật 19/2023 (trách nhiệm bảo hành) — thời hạn do người bán công bố, lưu theo dòng.
- Tóm tắt đánh giá bằng AI: gắn nhãn AI, không lược ý kiến tiêu cực — Luật 19/2023, Luật Trí tuệ nhân tạo
  134/2025 — cần đối chiếu văn bản gốc (D-201).

## Agent làm bước này tự quyết

- Tên thật của `api` các bước 36, 41–44 khi lệch bảng giả định — theo code đã merge; ghi lệch.
- Đánh giá: khuyến nghị đặt trong `orders` (thư mục `reviews/`, bật bằng tính năng `orders.reviews`):
  cần dữ liệu "đã nhận hàng" của chính `orders`, có số sao và thống kê, và không được qua bộ tự kiểm
  duyệt / chặn spam tự ẩn của `comments`. Thay bằng module `reviews` riêng nếu thấy `orders` quá lớn.
  Ghi quyết định (ROADMAP yêu cầu).
- Hạn đổi trả mặc định (khuyến nghị 7 ngày, là dữ liệu theo nút) và lý do trả mặc định.
- Có tách bảng tổng hợp báo cáo (`orders_sales_daily`) hay không: khuyến nghị truy vấn thẳng
  `orders_lines` có index trước, đo bằng bộ dữ liệu lớn của bước 39, chỉ thêm bảng tổng hợp khi đo thấy chậm.
- Một `PUT /carts/{id}` gộp hay nhiều endpoint con: khuyến nghị nhiều endpoint con như trên (mỗi thay
  đổi một lỗi rõ), giữ nhất quán.
- Ngưỡng mặc định của quy tắc COD (khuyến nghị: không có quy tắc nào bật sẵn; khách tự đặt).
- Mặc định của trả trước một phần: `preorder_balance_due_hours` (khuyến nghị 72), `preorder_max_delay_days`
  (khuyến nghị 30), `preorder_cancel_retention_bp` (0 — luôn hoàn đủ tới khi khách tự đặt) (D-232).
- Cách mở rộng `Media` để kiểm người tải lên: hàm `uploaded_by(ids)` có cài đặt mặc định, hay trường
  `uploaded_by` trong `MediaRef` theo cách không phá vỡ (D-202).
- Permission chưa ghi cho `GET /carts?status=abandoned`, `POST /{id}/payment-link`, `POST /{id}/pickup/*`,
  `POST /products/{id}/review-summaries`; tên cột và cách lưu trường nhận tại cửa hàng (`pickup_status`,
  `ready_at`, `pickup_hold_until`, mã nhận 6 số); khoá / index của `orders_lookup_tokens`. Ghi quyết định.
