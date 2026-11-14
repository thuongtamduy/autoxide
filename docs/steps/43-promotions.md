# Bước 43 — Khuyến mại (`promotions`)

> Lộ trình: `docs/ROADMAP.md` bước 43 và phần "Giai đoạn module thương mại" · Thiết kế:
> `docs/research/business-foundation.md` mục 8 · Quyết định: D-180, D-181, D-184, D-186, D-188
> (`rust_decimal`), D-191
> Đã gộp các quyết định: D-200, D-201, D-208…D-218, D-222, D-225 ; thêm thẳng vào thân: D-229 (D-228).
> Nguồn đối chiếu thiết kế: `docs/prompts/ui-trace-roadmap.md` (mã `A-…` / `B-…` trỏ tới
> `docs/research/ui-trace/`) và `docs/prompts/ui-phase2-api.md` (đợt 2, màn ghi trong
> `docs/research/ui-traceability.md`).

## Mục tiêu

Xong bước này, khách chạy được chương trình khuyến mại phổ biến ở Việt Nam và trên các nền tảng lớn — giảm
tự động theo sản phẩm / danh mục / giá trị đơn / nhóm khách, mua X tặng Y, miễn phí vận chuyển, flash sale có
số lượng, bậc thang, combo đồng giá, mua kèm deal sốc, chọn quà — với giới hạn lượt chống vượt khi nhiều người
dùng cùng lúc. Chương trình là **luật**; nó đến tay người mua bằng một trong ba cách: tự động, **coupon** (mã ai
có cũng dùng được) hay **voucher** (quyền dùng một lần trong ví của một tài khoản) (D-222). Khuyến mại cộng dồn
theo ba lớp `product` / `order` / `shipping`, ghi rõ ai trả tiền giảm. `orders` (45) gọi
`autoxide_promotions::api` để tính giảm giá cho một giỏ (từng dòng kèm lý do người mua đọc được) và giữ →
xác nhận → trả lượt dùng theo `order_id`. Chương trình vượt hạn mức pháp luật đang hiệu lực bị chặn; hạn mức
(mức giảm, số ngày giảm giá) là dữ liệu có ngày hiệu lực; xuất được dữ liệu thông báo khuyến mại gửi Sở Công
Thương theo từng nơi nhận (D-225).

## Bắt đầu khi

- Bước 41 xong: `dyn Catalog` (`variants` có `category_ids` kèm tổ tiên, `reference_prices` = giá ngay
  trước khuyến mại, `price_floors` = sàn giá), `autoxide_kernel::csv`.
- Từ nền: `TenantContext`, `OrgId`, `Scope`, `dyn OrgDirectory`, `rust_decimal`, `Money::allocate`; rate
  limiting của kernel; `address` (tỉnh làm nơi nhận thông báo); `notifications` (nhắc flash sale).
- Song song được với 42, 44.

## Phạm vi

- Crate mới `modules/promotions` (`autoxide-promotions`), đăng ký sau `catalog`.
- Ngoài phạm vi: điểm thưởng, thẻ quà tặng (47 — thẻ quà tặng là cách thanh toán, không phải khuyến mại,
  D-222); nhóm khách thật (47 — ở đây `customer_groups` là ID nhóm / phân khúc của bước 47 do bên gọi đưa, mờ
  với `promotions`); giảm giá riêng theo kênh (48); hoa hồng tiếp thị (49); nộp thông báo lên cổng dịch vụ công
  (chỉ xuất dữ liệu, người nộp là con người); "miễn phí giao không thời hạn" (là quy tắc phí của bước 44, không
  phải chương trình khuyến mại — D-225); "doanh thu tăng thêm" của chương trình (bước 57).

## Thiết kế

### Chương trình, coupon, voucher (D-222)

- **Chương trình** (`promotions_promotions`) là luật: điều kiện, lợi ích, thời gian, ngân sách, cộng dồn, ai
  trả tiền. `trigger` ∈ `automatic` / `code` / `voucher`.
- **Coupon** = dòng `promotions_codes` với `purpose = redeem` của chương trình `code`: ai có mã cũng dùng được,
  kể cả khách vãng lai. Mã chung hay lô mã riêng. Không có coupon "chỉ một người": việc đó là voucher.
- **Voucher** = dòng `promotions_vouchers` của chương trình `voucher`: một quyền **dùng một lần** trong ví của
  **một tài khoản**, không cần gõ mã. Chỉ người đăng nhập. Không chuyển cho người khác, không đổi ra tiền.
- Mã nhận voucher = `promotions_codes.purpose = claim`: nhập mã → cấp một voucher vào ví (không giảm thẳng).
- Coupon và voucher đều là khoản **giảm giá**: cùng chịu trần pháp luật, cùng ghi ai trả tiền.
- Người mua có voucher bằng cách bấm "Lưu" (chương trình `voucher` có `claimable`; đây là nút "Lưu mã" của
  website — không có `/me/saved-promotions`, D-225), nhập mã nhận, hay được cấp qua `grant_voucher`. Người gọi
  `grant_voucher`: `customers` (sinh nhật, đổi điểm, sau khi mua, giới thiệu bạn — bước 47), `support` (bù
  lỗi — 53), `automations` (58); cấp tay qua REST (lý do bắt buộc, có audit). Gọi lại cùng `source` +
  `source_ref` → trả voucher cũ, không cấp thêm.
- Voucher trong giỏ (`Cart.voucher_ids`) phải thuộc `buyer_id`, còn `available` và còn hạn; sai thì vào
  `rejected_codes` với lý do, không lỗi cả giỏ. Giữ: `available → held` bằng
  `UPDATE … WHERE status = 'available'`; xác nhận → `used`; trả → `available` nếu còn hạn, không thì `expired`.
- Dữ liệu cá nhân: ví nằm trong `PersonalData` export; xoá tài khoản → voucher chưa dùng chuyển `revoked`,
  voucher đã dùng giữ cho chứng từ, bỏ liên kết người.

### Kiểu và trait

Phác thảo, tra API thật bằng skill `check-library-api`.

```rust
// modules/promotions/src/api.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Cart {
    pub org_id: OrgId,                    // cửa hàng bán
    pub currency: Currency,
    pub buyer_id: Option<Uuid>,           // người mua đăng nhập
    pub buyer_keys: Vec<String>,          // khoá cho hạn mức mỗi người: `user:<id>`, băm SĐT, băm địa chỉ giao
                                          // chuẩn hoá (do `orders` tính; ở đây không lưu bản thô) (D-222)
    pub customer_groups: Vec<Uuid>,       // ID nhóm / phân khúc của bước 47
    pub membership_tier: Option<String>,  // hạng thành viên (bước 47, D-222)
    pub channel: Option<String>,
    pub client: Option<String>,           // `web` / `app` / `pos` (D-222)
    pub payment_method: Option<String>,   // đổi phương thức thanh toán / hãng giao là tính lại (D-222)
    pub carrier: Option<String>,
    pub ship_to_province_code: Option<String>,
    pub lines: Vec<CartLine>,
    pub shipping_fee: Option<Money>,      // phí vận chuyển đã báo (bước 44), để tính miễn phí vận chuyển
    pub codes: Vec<String>,               // coupon / mã người mua nhập (tối đa `MAX_CODES_PER_CART`)
    pub voucher_ids: Vec<Uuid>,           // voucher trong ví của `buyer_id` (D-222)
    pub gift_selections: Vec<GiftSelection>, // quà người mua chọn cho chương trình `gift_choice` (D-222)
    pub at: DateTime<Utc>,
    pub locale: String,                   // ngôn ngữ của `reason`
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CartLine {
    pub line_key: String,
    pub variant_id: Uuid,
    pub quantity: i64,
    pub unit_price: Money,                // giá `orders` lấy từ `Catalog::prices`
    pub has_compare_at: bool,             // dòng đang có giá gạch `compare_at` (D-222)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GiftSelection { pub promotion_id: Uuid, pub variant_id: Uuid, pub quantity: i64 }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Evaluation {
    pub lines: Vec<LineDiscount>,         // mọi khoản giảm đã phân về từng dòng
    pub shipping_discount: Money,
    pub gifts: Vec<GiftLine>,             // hàng tặng / quà — `orders` thêm dòng riêng giá 0
    pub total_discount: Money,
    pub applied: Vec<AppliedPromotion>,
    pub rejected_codes: Vec<RejectedCode>,
    pub fingerprint: String,              // dấu vân tay kết quả, `orders` gửi lại khi giữ
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineDiscount {
    pub line_key: String, pub promotion_id: Uuid, pub amount: Money,
    pub reason: Reason,
    pub capped_by_law: bool,              // đã bị cắt về trần pháp luật
    pub funding: Vec<FundingShare>,       // ai trả; chia bằng `Money::allocate`, tổng khớp `amount` (D-222)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PayerKind { Platform, Seller, Brand, PaymentPartner }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FundingShare { pub payer_kind: PayerKind, pub payer_org_id: Option<OrgId>, pub amount: Money }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reason { pub code: String, pub params: BTreeMap<String, String>, pub text: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedPromotion {
    pub promotion_id: Uuid, pub code: Option<String>, pub title: String, pub terms: String,
    pub amount: Money,
}
// Giữ tên `RejectedCode`; mỗi mục mang `code` hoặc `voucher_id` (D-222).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedCode { pub code: Option<String>, pub voucher_id: Option<Uuid>, pub reason: Reason }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GiftLine { pub promotion_id: Uuid, pub variant_id: Uuid, pub quantity: i64 }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReserveRequest {
    pub order_id: Uuid,
    pub cart: Cart,
    pub expected_fingerprint: Option<String>, // khác kết quả tính lại → `promotions.changed`
    pub expires_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RedemptionStatus { Held, Confirmed, Released, Expired }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Redemption {
    pub id: Uuid, pub order_id: Uuid, pub status: RedemptionStatus,
    pub evaluation: Evaluation,           // ảnh chụp để chụp vào chứng từ
    pub expires_at: Option<DateTime<Utc>>,
}

// Voucher (D-222)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoucherStatus { Available, Held, Used, Expired, Revoked }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoucherSource {
    Claim, ClaimCode, Grant, Loyalty, PurchaseReward, Birthday, Referral, Compensation, Automation,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GrantVoucher {
    pub promotion_id: Uuid, pub user_id: Uuid,
    pub source: VoucherSource, pub source_ref: String, // vd `birthday:<năm>`, `points:<id>`, `purchase:<order_id>`
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Voucher {
    pub id: Uuid, pub promotion_id: Uuid, pub holder_user_id: Uuid, pub status: VoucherStatus,
    pub source: VoucherSource, pub source_ref: String,
    pub valid_from: DateTime<Utc>, pub expires_at: DateTime<Utc>, pub issued_at: DateTime<Utc>,
    pub used_order_id: Option<Uuid>,
}

#[async_trait::async_trait]
pub trait Promotions: Send + Sync {
    /// Tính giảm giá cho giỏ — không giữ gì, không ghi gì.
    async fn evaluate(&self, tenant: &TenantContext, cart: &Cart) -> Result<Evaluation, Error>;
    /// Mã, voucher dùng được và chưa đủ điều kiện (kèm lý do) cho giỏ, cùng `best_selection` — không ghi gì.
    async fn eligible(&self, tenant: &TenantContext, cart: &Cart) -> Result<Eligibility, Error>;
    /// Tính lại và giữ mọi lượt (chương trình, mã, voucher, ngân sách, ngày, mỗi khoá người mua, flash sale,
    /// quà) trong MỘT transaction: đủ hết hoặc không giữ gì. Gọi lại cùng `order_id` + cùng giỏ → `Replayed`.
    async fn reserve(&self, tenant: &TenantContext, request: &ReserveRequest)
        -> Result<Submitted<Redemption>, Error>;
    /// Giữ → đã dùng. Gọi lại: trả như cũ. Đã hết hạn / trả: `promotions.redemption_expired`.
    async fn confirm(&self, tenant: &TenantContext, order_id: Uuid) -> Result<Redemption, Error>;
    /// Trả lượt (huỷ đơn, thanh toán hỏng). Đã xác nhận thì trả khi chương trình cho phép
    /// (`return_usage_on_cancel`); voucher về ví nếu còn hạn. Không có: `Ok(None)`.
    async fn release(&self, tenant: &TenantContext, order_id: Uuid) -> Result<Option<Redemption>, Error>;
    /// Cấp voucher vào ví. Gọi lại cùng `source` + `source_ref` → trả voucher cũ, không cấp thêm (D-222).
    async fn grant_voucher(&self, tenant: &TenantContext, request: &GrantVoucher)
        -> Result<Submitted<Voucher>, Error>;
    /// Ví voucher của một tài khoản (D-222).
    async fn vouchers_of(&self, tenant: &TenantContext, user_id: Uuid, status: Option<VoucherStatus>)
        -> Result<Vec<Voucher>, Error>;
}
```

### Lớp và cộng dồn (D-222)

- `class` ∈ `product` / `order` / `shipping`, suy từ `scope` (`items` → `product`).
- `exclusivity` ∈ `none` / `class` / `global`: `class` = không cộng với chương trình khác cùng lớp; `global` =
  không cộng với bất cứ gì.
- `combines_with` (tập lớp được cộng cùng, vd chương trình lớp `order` cộng được với `shipping` nhưng không với
  `product`). Hai chương trình cộng được khi **cả hai** cho phép (như Shopify).
- `stop_after` (bool): áp chương trình này thì không xét chương trình ưu tiên thấp hơn cùng lớp.
- Chế độ của khách (cài đặt kế thừa `promotions.stacking_mode`): `priority` (mặc định — dễ đoán: theo
  `priority` giảm dần rồi `id`) hay `best_for_buyer`: mỗi lớp so **tập cộng dồn được** với **từng chương trình
  độc quyền đứng riêng**, lấy khoản giảm lớn nhất; bằng nhau → `priority` → `id`. Số phương án ≤ số chương trình
  độc quyền ứng viên, nên tuyến tính, vẫn tất định. Không tìm tổ hợp tối ưu toàn cục.
- Giới hạn: `MAX_CODES_PER_CART` và `MAX_VOUCHERS_PER_CLASS` (1) — tính **theo từng bên trả tiền**: trên sàn
  một giỏ dùng được 1 voucher của mỗi người bán + 1 voucher sàn cho mỗi lớp (kiểu Shopee).

### Loại khuyến mại

`kind` ∈ `percent_off` / `amount_off` / `fixed_price` / `buy_x_get_y` / `free_shipping` / `flash_sale`, cùng
các loại sau (D-222):

| `kind` | Nghĩa | Cột / bảng |
|---|---|---|
| `tiered` | Mua nhiều giảm nhiều: 2 món −10%, 3 món −15%; đơn 500k −50k, 1 triệu −120k | `promotions_tiers` (`promotion_id`, `threshold_kind` `quantity`/`amount`, `threshold`, `value_bp?`, `value_minor?`); lấy bậc cao nhất đạt |
| `bundle_price` | Combo đồng giá: chọn N món trong tập mục tiêu giá X ("3 món 199k") | `bundle_quantity`, `bundle_price_minor`; lặp theo `max_applications`; xếp món đắt nhất vào combo trước cho tất định |
| `add_on` | Mua kèm deal sốc: có món điều kiện thì mua món thưởng giá ưu đãi | `role = condition / reward`; `max_add_on_quantity`; giá / mức giảm theo `promotions_flash_items` |
| `gift_choice` | Chọn quà: đạt điều kiện thì chọn `gift_quantity` món trong danh sách | `promotions_gift_options` (`variant_id`, `stock_limit?`, `quantity_used`); `Cart.gift_selections`; chưa chọn → gợi ý, không chặn đặt hàng |
| `buy_x_get_y` (mở rộng) | Mua 2 tính 1, món rẻ nhất miễn phí, mua 2 giảm 50% món thứ hai | `reward_selection` (`cheapest`/`same_item`/`specified`), `reward_value_bp` (10.000 = tặng), `max_applications` |

Hàng tặng và quà là **dòng riêng giá 0** (`gifts`), giữ tồn qua `orders` → `inventory` như dòng thường (D-222
chốt).

### Điều kiện

- Nhóm / phân khúc khách qua `promotions_customer_groups` (ID nhóm của bước 47); kênh qua `promotions_channels`.
- Điều kiện `first_order_only`: đơn đầu của người mua trong khách. `orders` đăng ký sau `promotions` nên không
  hỏi ngược: `orders` điền `Cart.first_order: bool` (D-208…D-218, B-b7, B-c10; D-228).
- Điều kiện mở rộng (D-222): `promotions_conditions` (`promotion_id`, `kind`, `operator` `in`/`not_in`/`gte`/
  `lte`, `values` JSON trong `text`). Nạp vào bộ nhớ thì biên dịch thành struct, không đọc JSON khi tính. `kind`:
  `payment_method`, `carrier`, `ship_to_province`, `membership_tier`, `client` (`web`/`app`/`pos`), `brand`
  (thuộc tính), `seller_org`, `attribute`, `new_customer` (chưa có đơn hoàn tất), `returning_customer`,
  `min_items`, `exclude_discounted` (bỏ dòng đã có khoản giảm lớp `product` hay đang có giá gạch `compare_at`),
  `exclude_targets`. `orders` điền các trường tương ứng của `Cart`.
- Khung giờ ("giờ vàng"): `promotions_schedules` (`promotion_id`, `weekdays` (bitmask), `start_minute`,
  `end_minute`) theo múi giờ cửa hàng; không có dòng = cả ngày (D-222).

### Khối cho trang (D-229, bước 67)

`promotions` khai `promotions.flash_sale` (đếm ngược, suất còn là số thật) và `promotions.voucher_claim` (nút
"Lưu" voucher `claimable`) qua `Module::blocks()`, gắn `BlockData` trả dữ liệu công khai của chương trình;
chương trình đã kết thúc → `None`. Bộ nháp chiến dịch của bước 67 tạo trang, banner, bản tin từ một chương trình.

### Cách tính (thứ tự cố định để cùng giỏ luôn ra cùng kết quả)

1. Lấy biến thể từ `Catalog::variants` (sản phẩm, danh mục kèm tổ tiên), giá tham chiếu từ
   `Catalog::reference_prices` tại `starts_at` của từng chương trình và sàn giá từ `Catalog::price_floors` —
   trước khi tính, ngoài mọi transaction. Chương trình lấy từ `CompiledPromotions` trong bộ nhớ (mục "Nhẹ và
   nhanh"): tra chỉ mục cho các dòng của giỏ, không query database ngoài lần nạp.
2. Ứng viên: trạng thái `active`, `starts_at ≤ at < ends_at`, trong khung giờ (nếu có), `org_id` của chương
   trình là cửa hàng hay tổ tiên của nó, nhóm khách / kênh / điều kiện khớp; chương trình `code` chỉ khi mã
   (`purpose = redeem`) có trong `codes` và còn hiệu lực; chương trình `voucher` chỉ khi voucher có trong
   `voucher_ids` và hợp lệ (mã / voucher sai → `rejected_codes` với lý do, không lỗi cả giỏ). Chương trình của
   người bán chỉ áp dòng của người bán đó. Không đủ ngân sách thì **không áp** chương trình (lý do
   `promotions.reason.budget_exhausted`), không áp một phần.
3. Lớp `product` (flash sale, giảm theo sản phẩm / danh mục, mua X tặng Y, bậc thang, combo, mua kèm, chọn
   quà): chọn theo luật cộng dồn ở trên (`stacking_mode`, `exclusivity`, `combines_with`, `stop_after`); các
   khoản cộng dồn tính trên phần giá còn lại; không xuống dưới 0.
4. Lớp `order` (giá trị đơn tối thiểu, nhóm khách, bậc theo số tiền…): trên tổng sau bước 3, phân về các dòng
   đủ điều kiện bằng `Money::allocate` theo tỉ lệ tiền dòng (cần cho thuế và đổi trả).
5. Lớp `shipping`: miễn / giảm phí vận chuyển trên `shipping_fee`.
6. **Trần pháp luật**: mỗi dòng, cộng **mọi** khoản giảm trên một đơn vị (mọi lớp, mọi bên trả tiền — kể cả
   voucher sàn và voucher của người bán cùng áp) rồi so với `max_bp` × giá tham chiếu của luật đang hiệu lực tại
   `at` (theo loại khuyến mại và loại chương trình); vượt thì cắt và `capped_by_law = true` (D-222, cần đối
   chiếu). Tính bằng `rust_decimal`, làm tròn về đơn vị nhỏ nhất theo quy tắc ghi trong DECISIONS.
   **Sàn giá** của `catalog`: quy tắc `block` cắt khoản giảm như trần pháp luật, lý do
   `promotions.reason.capped_by_floor` (D-222).
7. Chia mỗi khoản giảm cho người trả tiền theo `promotions_funding` (`LineDiscount.funding`, mục "Ai trả tiền").
8. `reason.text` dịch theo `locale` (key = `reason.code`, vd `promotions.reason.percent_off` với
   `{percent}`, `{min_order}`).

### Ai trả tiền (D-222)

- `promotions_funding` (`promotion_id`, `payer_kind` (`platform`/`seller`/`brand`/`payment_partner`),
  `payer_org_id?`, `share_bp`; tổng `share_bp` = 10.000, sai → `promotions.invalid_funding`). Mặc định: một dòng
  `seller` = nút sở hữu chương trình, 100%.
- `LineDiscount.funding` chia bằng `Money::allocate`, tổng khớp từng đồng. Chương trình của người bán chỉ áp dòng
  của người bán đó; chương trình của sàn chia về các đơn con theo tiền dòng. `orders` chụp `funding` vào đơn;
  `marketplace` quyết toán theo đó (đặc tả 46); `payment_partner` thành khoản phải thu từ đối tác trong `ledger`.
- **Tham gia chương trình của sàn** (flash sale, ngày hội): `promotions_participations` (`promotion_id`,
  `seller_org_id`, `status` `pending`/`approved`/`rejected`, `items` JSON: biến thể, giá / mức giảm đề nghị, số
  suất, `reviewed_by?`). Người bán đăng ký (`promotions.participate`), sàn duyệt; duyệt xong mới thành
  `flash_items`. Phần người bán tự giảm thì `funding` của người bán.

### Ngân sách, lượt theo ngày (D-222)

- `budget_minor?` + `budget_used_minor`: giữ = cộng khoản giảm của chương trình trong đơn bằng `rows::adjust`
  có trần. Không đủ ngân sách thì không áp chương trình (không áp một phần). Hết ngân sách → event
  `promotions.budget_exhausted`, chương trình vẫn `active` cho tới khi trả lượt làm còn chỗ.
- `daily_usage_limit?`: `promotions_daily_usage` (`promotion_id`, `day` theo múi giờ cửa hàng, `used`) — chèn
  `ON CONFLICT DO NOTHING` rồi `adjust`.

### Chống lạm dụng (D-222)

- `per_customer_limit` kèm `per_customer_period` (`lifetime` / `day` / `week` / `month`).
- Hạn mức mỗi người tính trên `Cart.buyer_keys` (`user:<id>`, băm số điện thoại, băm địa chỉ giao chuẩn hoá —
  `orders` tính; ở đây không lưu bản thô). Giữ lượt cộng **mọi** khoá theo thứ tự cố định; một khoá hết lượt là
  từ chối — một người không lấy thêm lượt bằng tài khoản mới cùng số điện thoại.
- Flash sale: `per_customer_quantity_limit` mỗi biến thể.
- Thử mã: rate limiting của kernel theo người mua / IP cho nhập coupon và mã nhận voucher; lô mã riêng dài ≥ 8 ký
  tự.

### Trả hàng và huỷ (quy tắc cho `orders`, đặc tả 45; D-222)

Dùng ảnh chụp `Evaluation`:
- Trả một phần: tiền hoàn của dòng = tiền đã trả của dòng × số trả / số mua, chia bằng `Money::allocate` (đơn vị
  cuối nhận phần dư) — tổng hoàn qua nhiều lần trả không vượt tiền đã trả.
- Trả làm đơn rớt dưới ngưỡng (đơn 500k giảm 50k, trả còn 300k): mặc định **không thu lại** khoản giảm (cài đặt
  kế thừa `promotions.clawback_on_return` = `none` / `proportional`).
- Quà / hàng tặng khi trả món điều kiện: `gift_return_policy` của chương trình (`return_gift` /
  `deduct_gift_value` / `keep`), mặc định `return_gift`.
- Huỷ trọn đơn: coupon trả lượt theo `return_usage_on_cancel`; voucher về ví nếu còn hạn. Trả hàng một phần không
  trả voucher.

### Bảng dữ liệu

Bảng cấp khách có `tenant_id`; unique / index bắt đầu bằng nó; phần trăm lưu `integer` basis point
(1% = 100); tiền `*_minor` + `*_currency`; JSON trong `text`. Migration có `down`; thêm
`outbox_migration`, `jobs_migration`.

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `promotions_promotions` | `id`, `tenant_id`, `org_id`, `kind` (`percent_off`/`amount_off`/`fixed_price`/`buy_x_get_y`/`free_shipping`/`flash_sale`/`tiered`/`bundle_price`/`add_on`/`gift_choice`), `scope` (`items`/`order`/`shipping`; lớp suy từ đây), `trigger` (`automatic`/`code`/`voucher`), `status` (`draft`/`scheduled`/`active`/`paused`/`ended`/`cancelled`), `value_bp?`, `value_minor?`, `value_currency?`, `min_order_minor?`, `min_quantity?`, `max_discount_minor?`, `buy_quantity?`, `get_quantity?`, `reward_selection?`, `reward_value_bp?`, `max_applications?`, `bundle_quantity?`, `bundle_price_minor?`, `max_add_on_quantity?`, `gift_quantity?`, `starts_at`, `ends_at` (bắt buộc, D-225), `priority`, `exclusivity` (`none`/`class`/`global`), `combines_with` (tập lớp), `stop_after`, `usage_limit?`, `usage_count`, `per_customer_limit?`, `per_customer_period`, `daily_usage_limit?`, `budget_minor?`, `budget_used_minor`, `return_usage_on_cancel`, `gift_return_policy`, `program_kind` (`regular`/`national_focus`), chương trình `voucher`: `voucher_quota?` + `issued_count` (tổng số phát), `per_user_claim_limit?`, `voucher_valid_days?` (hạn tính từ lúc nhận; không có thì dùng `ends_at`), `claimable` (hiện nút "Lưu" công khai), `claim_requires_verified_phone`; `public`, `created_by`, `created_at`, `updated_at`, `activated_at?` | (`tenant_id`,`status`,`starts_at`); (`tenant_id`,`org_id`,`status`) |
| `promotions_translations` | `promotion_id`, `locale`, `title`, `terms` (điều kiện người mua đọc) | unique (`tenant_id`,`promotion_id`,`locale`) |
| `promotions_targets` | `promotion_id`, `target_type` (`product`/`variant`/`category`), `target_id`, `role` (`condition`/`reward`) | (`tenant_id`,`promotion_id`); (`tenant_id`,`target_type`,`target_id`) |
| `promotions_customer_groups` · `promotions_channels` | `promotion_id`, `group_id` (ID nhóm của bước 47) / `channel` | unique (`tenant_id`,`promotion_id`,…) |
| `promotions_conditions` | `promotion_id`, `kind`, `operator` (`in`/`not_in`/`gte`/`lte`), `values` (JSON) | (`tenant_id`,`promotion_id`) |
| `promotions_schedules` | `promotion_id`, `weekdays` (bitmask), `start_minute`, `end_minute` | (`tenant_id`,`promotion_id`) |
| `promotions_tiers` | `promotion_id`, `threshold_kind` (`quantity`/`amount`), `threshold`, `value_bp?`, `value_minor?` | (`tenant_id`,`promotion_id`) |
| `promotions_gift_options` | `promotion_id`, `variant_id`, `stock_limit?`, `quantity_used` | (`tenant_id`,`promotion_id`) |
| `promotions_codes` | `id`, `tenant_id`, `promotion_id`, `code`, `code_key` (chữ hoa, bỏ khoảng trắng), `purpose` (`redeem`/`claim`), `usage_limit?`, `usage_count`, `created_at` | unique (`tenant_id`,`code_key`); (`tenant_id`,`promotion_id`) |
| `promotions_vouchers` | `id`, `tenant_id`, `promotion_id`, `holder_user_id`, `status` (`available`/`held`/`used`/`expired`/`revoked`), `source` (`claim`/`claim_code`/`grant`/`loyalty`/`purchase_reward`/`birthday`/`referral`/`compensation`/`automation`), `source_ref`, `valid_from`, `expires_at`, `issued_at`, `used_order_id?`, `updated_at` | unique (`tenant_id`,`promotion_id`,`holder_user_id`,`source`,`source_ref`); (`tenant_id`,`holder_user_id`,`status`,`expires_at`); (`tenant_id`,`status`,`expires_at`) |
| `promotions_flash_items` | `promotion_id`, `variant_id`, `sale_price_minor?`, `value_bp?`, `quantity_limit`, `quantity_used`, `per_customer_quantity_limit?` | unique (`tenant_id`,`promotion_id`,`variant_id`) |
| `promotions_funding` | `promotion_id`, `payer_kind` (`platform`/`seller`/`brand`/`payment_partner`), `payer_org_id?`, `share_bp` | (`tenant_id`,`promotion_id`) |
| `promotions_participations` | `promotion_id`, `seller_org_id`, `status` (`pending`/`approved`/`rejected`), `items` (JSON), `reviewed_by?` | (`tenant_id`,`promotion_id`) |
| `promotions_customer_usage` | `promotion_id`, `buyer_key` (một dòng mỗi khoá trong `Cart.buyer_keys`), `used` | unique (`tenant_id`,`promotion_id`,`buyer_key`) |
| `promotions_daily_usage` | `promotion_id`, `day` (theo múi giờ cửa hàng), `used` | unique (`tenant_id`,`promotion_id`,`day`) |
| `promotions_redemptions` | `id`, `tenant_id`, `order_id`, `org_id`, `status`, `fingerprint`, `evaluation` (JSON), `total_discount_minor`, `currency`, `expires_at?`, `created_at`, `updated_at` | unique (`tenant_id`,`order_id`); (`tenant_id`,`status`,`expires_at`) |
| `promotions_redemption_holds` | `redemption_id`, `counter` (`promotion`/`code`/`voucher`/`budget`/`daily`/`customer`/`flash`/`gift`), `counter_id`, `quantity` | (`tenant_id`,`redemption_id`) |
| `promotions_stats_daily` | `promotion_id`, `day`, `redemptions`, `discount_minor`, `order_value_minor`, `platform_funded_minor`, `seller_funded_minor` | unique (`tenant_id`,`promotion_id`,`day`) |
| `promotions_revisions` | `tenant_id`, `revision` (tăng trong cùng transaction với mọi lần đổi chương trình) | pk (`tenant_id`) |
| `promotions_versions` | **chỉ thêm**: `promotion_id`, `version`, `snapshot` (JSON đủ để dựng lại chương trình), `changed_by`, `changed_at` | unique (`tenant_id`,`promotion_id`,`version`) |
| `promotions_notices` | `id`, `tenant_id`, `promotion_id`, `authority_code` (tỉnh theo `address`), `status` (`pending`/`submitted`/`not_required`), `reference?`, `submitted_at?`, `submitted_by?` | unique (`tenant_id`,`promotion_id`,`authority_code`) |
| `promotions_reminders` | `tenant_id`, `promotion_id`, `user_id`, `channel`, `consent_id`, `remind_at`, `sent_at?` | unique (`tenant_id`,`promotion_id`,`user_id`) |
| `promotions_legal_limits` | **dữ liệu dùng chung mọi khách (mặt điều khiển)**: `id`, `jurisdiction`, `promotion_kind` (`price_reduction`/`gift_value`/…), `program_kind`, `max_bp`, `max_days_per_program?`, `max_days_per_year?`, `effective_from`, `effective_to?`, `legal_basis`, `source_url`, `note`, `pack_version` | unique (`jurisdiction`,`promotion_kind`,`program_kind`,`effective_from`) |
| `promotions_notice_rules` | **dùng chung**: `jurisdiction`, `promotion_kind` (gồm `purchase_voucher` — voucher cấp sau khi mua), `notice_required`, `days_before`, `authority`, `effective_from`, `effective_to?`, `legal_basis` | như trên |

Giữ lượt: mỗi bộ đếm đổi bằng `rows::adjust` với `Bounds { floor: 0, ceiling: <giới hạn> }` (giới hạn
đọc trong cùng transaction) — `usage_count` của chương trình, của mã, `issued_count` khi phát voucher,
`budget_used_minor`, `used` của ngày, `quantity_used` của flash sale và của quà, `used` của từng khoá người mua
(dòng mỗi người / mỗi ngày chèn `used = 0` bằng `ON CONFLICT DO NOTHING` trước); voucher đổi trạng thái bằng
`UPDATE` có điều kiện. Mọi bộ đếm trong một transaction, theo thứ tự (`counter`, `counter_id`) cố định. Mỗi lần
giữ ghi một dòng `promotions_redemption_holds`, trả lại đúng các dòng đó. `promotions_stats_daily` cập nhật trong
cùng transaction với `confirm` / `release`.

### Thông báo khuyến mại theo nơi nhận (D-225)

- Mỗi cơ quan nhận một dòng `promotions_notices`. Nơi nhận suy từ nơi bán của chương trình (cửa hàng áp dụng,
  hay "toàn quốc" khi bán online — quy tắc là dữ liệu `promotions_notice_rules`). Trạng thái thông báo của
  chương trình suy ra từ các dòng này (mọi nơi đã nộp).
- `PATCH /promotions/{id}/notices/{authority_code}` ghi nhận đã nộp; xuất dữ liệu thông báo theo từng nơi.

### Thời hạn chương trình (D-225)

`ends_at` bắt buộc. `activate` cộng số ngày các chương trình giảm giá đã chạy cho cùng biến thể trong năm (theo
`promotions_versions` và mục tiêu) và so với `max_days_per_program` / `max_days_per_year` của luật đang hiệu
lực — vượt → `promotions.exceeds_duration_limit`.

### Kiểm trước khi bật (D-225)

`POST /promotions/{id}/activation-check` (`promotions.publish`, không ghi) trả `checks[]` (`code`, `ok`,
`params`): trần giảm, thời hạn, thông báo từng nơi, mục tiêu, ngân sách / funding, xung đột (cảnh báo). Sàn
giá của `catalog` (`Catalog::price_floors`): cảnh báo hay chặn theo `action` của quy tắc (D-222). `activate`
lỗi trả cùng danh sách trong `errors[]` của `Problem` (không lưu "lỗi lần bật cuối"); cảnh báo (vd xung đột)
trả trong `warnings`, không chặn.

### Nhắc khi flash sale bắt đầu (D-225)

Người mua bấm "Nhắc tôi": `POST/DELETE /public/promotions/{id}/reminders` (`promotions.use`) tạo / xoá một dòng
`promotions_reminders`. Lần bấm là đồng ý riêng cho đúng một lần nhắc đó (`consent_id`, ghi bằng chứng đồng ý
theo bước 37), không thành đồng ý nhận quảng cáo. Job `promotions.send_reminders` mỗi phút gửi qua
`notifications` khi còn `REMINDER_LEAD_MINUTES` (10) tới giờ mở, đánh dấu `sent_at` có điều kiện (không gửi hai
lần). Tối đa `MAX_REMINDERS_PER_USER` lời nhắc chưa gửi.

### Chạy thử, xung đột, lịch, số liệu (D-222)

- `POST /promotions/{id}/simulate` `{carts: [Cart]}` (≤ `MAX_SIMULATE_CARTS` giỏ): mỗi giỏ kết quả có và không
  có chương trình; chạy được cả khi `draft`. Không ghi gì.
- `GET /promotions/{id}/conflicts`: chương trình trùng mục tiêu và thời gian có `exclusivity` khác `none` →
  cảnh báo; `activate` trả `warnings`, không chặn.
- `GET /promotions/calendar?from&to&org_id`: chương trình theo khoảng ngày (cho lịch của người bán).
- `promotions_stats_daily`; `GET /promotions/{id}/stats?from&to`. Số liệu chỉ là số đếm.

### Nhẹ và nhanh (D-222)

- `CompiledPromotions` trong bộ nhớ theo (khách, cửa hàng): chương trình `active` / `scheduled` trong 24 giờ
  tới, điều kiện đã biên dịch, chỉ mục `variant_id` / `product_id` / `category_id` / thuộc tính → chương trình.
  Khoá theo `promotions_revisions` (`tenant_id`, `revision`) tăng trong cùng transaction với mọi lần đổi chương
  trình; máy chủ khác nhận event để nạp lại, kèm hạn `COMPILED_TTL_SECONDS` làm lưới an toàn.
- `evaluate` = tra chỉ mục cho dòng trong giỏ → ứng viên → tính; không query database ngoài lần nạp. Mục tiêu:
  giỏ 50 dòng, 500 chương trình đang chạy, p99 < 5 ms phần tính; bench `promotions/benches/evaluate.rs`, số ghi
  `docs/BENCHMARKS.md`.
- `reserve` một transaction: mọi bộ đếm theo thứ tự (`counter`, `counter_id`) cố định.
- Giá sau giảm tự động lớp `product` không có điều kiện giỏ thì ghi sẵn sang `catalog`: phát event khi giá sau
  giảm tự động của biến thể đổi, cho `catalog_storefront_entries` (D-208…D-218, B-b7).

### API REST

Dưới `/api/v1/promotions`. Chương trình kiểm quyền theo độ sâu ở `org_id`.

| Method + path | Permission | Ghi chú |
|---|---|---|
| `GET /promotions?status&kind&org_id&q` · `POST /promotions` | `promotions.read` · `promotions.write` | tạo ở `draft` |
| `GET/PATCH/DELETE /promotions/{id}` | read · write | xoá chỉ khi `draft`; sửa sau khi chạy bị giới hạn (dưới) |
| `PUT /promotions/{id}/translations/{locale}` · `PUT /promotions/{id}/targets` · `PUT /promotions/{id}/flash-items` | write | |
| `POST /promotions/{id}/activation-check` | `promotions.publish` | không ghi; `checks[]` (`code`, `ok`, `params`) (D-225) |
| `POST /promotions/{id}/activate` | `promotions.publish` | kiểm như `activation-check`; lỗi → `errors[]`, cảnh báo → `warnings`; → `scheduled` / `active` |
| `POST /promotions/{id}/pause` · `…/resume` · `…/end` · `…/cancel` | `promotions.publish` | |
| `POST /promotions/{id}/codes` (sinh lô mã ngẫu nhiên hay một mã đặt tay; `purpose` `redeem` / `claim`) · `GET /promotions/{id}/codes` · `GET /promotions/{id}/codes/export` (CSV) | `promotions.codes.manage` | tối đa 10.000 mã / lần, bằng job khi > 500 |
| `POST /promotions/{id}/vouchers` | `promotions.vouchers.grant` | cấp tay; lý do bắt buộc, có audit (D-222) |
| `POST /promotions/{id}/simulate` | `promotions.read` | `{carts: [Cart]}`, ≤ 20 giỏ; không ghi (D-222) |
| `GET /promotions/{id}/conflicts` · `GET /promotions/calendar?from&to&org_id` · `GET /promotions/{id}/stats?from&to` | `promotions.read` | (D-222) |
| `GET /promotions/{id}/versions` | `promotions.read` | lưu vết |
| `POST /promotions/{id}/notice-export?format=csv|json` | `promotions.compliance` | dữ liệu thông báo gửi Sở Công Thương, theo từng nơi nhận |
| `PATCH /promotions/{id}/notices/{authority_code}` | `promotions.compliance` | `{status: submitted, reference, submitted_at}` (D-225) |
| `GET /legal-limits?at` · `GET /notice-rules?at` | `promotions.read` | dữ liệu dùng chung, chỉ đọc |
| `POST /evaluate` | `promotions.read` | thân là `Cart` — cho agent / nhân viên thử; cùng hàm với `api` |
| `GET /redemptions?order_id&status` · `POST /redemptions/{id}/release` | `promotions.read` · `promotions.publish` | gỡ kẹt bằng tay |
| `GET /me/vouchers?status&store_id` | `promotions.use` | ví voucher, sắp theo hạn gần nhất; `Page` (D-222) |
| `POST /me/vouchers/claim-code` `{code}` | `promotions.use` | nhập mã nhận voucher; giới hạn số lần thử (D-222) |
| `POST /public/promotions/{id}/claim` | `promotions.use` | bấm "Lưu" (chương trình `voucher` có `claimable`); 201; hết suất → `promotions.voucher_quota_exhausted` (D-222, D-225) |
| `POST/DELETE /public/promotions/{id}/reminders` | `promotions.use` | nhắc khi flash sale bắt đầu (D-225) |
| `GET /public/promotions?locale&store_id` | công khai | chương trình `public` đang chạy, tiêu đề + điều kiện (Luật 19/2023); ETag như `content` |
| `GET /public/promotions/for-product/{product_id}?variant_id&store_id` | công khai | mã `public` đang chạy áp được cho sản phẩm (D-200, dưới) |
| (không có endpoint riêng) | — | mã / voucher dùng được cho giỏ: hàm `Promotions::eligible(tenant, &Cart)` của `api`, `orders` phục vụ qua `GET /api/v1/orders/carts/{id}/promotions` (D-201, D-222, D-228, dưới) |
| `GET /public/promotions/flash-sales` | công khai | giờ bắt đầu / kết thúc và **suất còn** (D-208…D-218, B-c10) |

Sửa chương trình đã `active`: chỉ được kéo dài `ends_at`, tăng giới hạn lượt, tạm dừng, kết thúc sớm,
sửa bản dịch không đổi điều kiện; thay đổi khác → `promotions.locked_after_start` (tạo chương trình
mới). Mọi lần đổi sau `activate` thêm một dòng `promotions_versions`.

**Giá khi áp mã cho một sản phẩm (D-200).** `GET /public/promotions/for-product/{product_id}` trả các mã
`public` đang chạy áp được cho sản phẩm: mã, giá sau mã cho số lượng 1, điều kiện (dùng chung `terms` với
`GET /public/promotions`), số lượt còn lại. Giảm tự động (không cần mã) thì `catalog` hiện giá đã giảm; mã phải
nhập thì giá niêm yết giữ nguyên và giao diện hiện thêm dòng "còn X với mã …" — không trình bày như giá đã giảm
sẵn (Luật 19/2023, minh bạch điều kiện).

**Mã dùng được cho một giỏ (D-201, D-222).** Giỏ thuộc `orders` (đăng ký sau), nên `promotions` không đọc giỏ:
`api` có `Promotions::eligible(tenant, &Cart) -> Eligibility`, `orders` gọi và phục vụ ở
`GET /api/v1/orders/carts/{id}/promotions` (D-228). Kết quả gồm: mã dùng được (kèm số
giảm cho giỏ này), mã `public` chưa đủ điều kiện kèm `reason` có tham số ("thêm 120.000 ₫", "giỏ chưa có sản phẩm
áp dụng", "chỉ cho đơn đầu tiên"), voucher trong ví dùng được cho giỏ, và `best_selection` (tổ hợp mã + voucher
lợi nhất theo luật cộng dồn). Website thay ô nhập mã mở sẵn bằng bộ chọn, ô nhập tự do nằm sau một liên kết.
`remaining_uses` chỉ trả khi chương trình có giới hạn lượt, lấy từ số giữ chỗ thật (không làm tròn, không số
giả).

**Flash sale công khai (D-208…D-218, B-c10).** Suất còn là số thật, không làm giả độ khan hiếm (Luật 19/2023).

### Tool MCP

`promotions.list_promotions`, `promotions.get_promotion`, `promotions.create_promotion`,
`promotions.activate_promotion`, `promotions.end_promotion`, `promotions.generate_codes`,
`promotions.evaluate_cart` (`read_only`), `promotions.export_notice` (`read_only`),
`promotions.grant_voucher` (cùng service với REST, `promotions.vouchers.grant`; bước 58 gọi),
`promotions.simulate` (`read_only`), `promotions.list_vouchers_of_user` (`read_only`, cho nhân viên hỗ trợ)
(D-222).

### Event

`promotions.promotion_created` · `promotions.promotion_updated` · `promotions.promotion_activated` ·
`promotions.promotion_started` · `promotions.promotion_paused` · `promotions.promotion_ended` ·
`promotions.promotion_cancelled` · `promotions.codes_generated` · `promotions.redemption_confirmed` ·
`promotions.redemption_released` · `promotions.notice_submitted` · `promotions.voucher_issued` ·
`promotions.voucher_used` · `promotions.budget_exhausted` · `promotions.participation_reviewed` (D-222); event khi
giá sau giảm tự động của biến thể đổi (cho `catalog_storefront_entries`, D-208…D-218). Không phát event cho mỗi
lần giữ (quá nhiều) và cho mỗi voucher hết hạn (job đổi theo lô); xác nhận và trả có event. Payload không chứa
`buyer_keys`.

Không nhận event của `orders` ở bước này (`orders` gọi `api`); nếu bước 45 muốn đường an toàn bằng
event thì thêm `Subscription::optional` cho `orders.order_confirmed` / `orders.order_cancelled` như
`inventory`.

### Job và lịch

- `promotions.expire_holds` mỗi phút (đổi `held → expired` có điều kiện, trả bộ đếm theo
  `redemption_holds`); `promotions.tick` mỗi phút (`scheduled → active` khi tới giờ, phát `…_started`;
  `active → ended` khi qua `ends_at`, phát `…_ended`); `promotions.generate_codes` (lô lớn);
  `promotions.expire_vouchers` mỗi giờ (D-222); `promotions.send_reminders` mỗi phút (D-225).
- Task: `promotions.load_legal_data pack=vn` — nạp `modules/promotions/data/vn/legal_limits.json` và
  `notice_rules.json` (nguồn, ngày, ghi chú "cần đối chiếu"); chạy lại không nhân đôi.
- `Module::checks`: cảnh báo khi chưa có hạn mức pháp luật nào hiệu lực cho `VN`, hay hạn mức hết
  hiệu lực trong 30 ngày.

### Cấu hình (`AUTOXIDE_PROMOTIONS_`)

`MAX_CODES_PER_CART` (5), `MAX_HOLD_HOURS` (72), `CODE_LENGTH` (10), `MAX_VOUCHERS_PER_CLASS` (1),
`MAX_SIMULATE_CARTS` (20), `COMPILED_TTL_SECONDS` (60) (D-222), `MAX_REMINDERS_PER_USER` (50), `REMINDER_LEAD_MINUTES` (10) (D-225). Sinh lại
`.env.example`.

Cài đặt kế thừa theo nút: `promotions.stacking_mode` (`priority` mặc định / `best_for_buyer`),
`promotions.clawback_on_return` (`none` mặc định / `proportional`) (D-222).

### Tính năng và hạn mức

`Module::features`: `promotions.codes`, `promotions.flash_sale`, `promotions.buy_x_get_y`,
`promotions.customer_groups`, `promotions.vouchers`, `promotions.tiered`, `promotions.bundles`,
`promotions.budget` (D-222). `Module::limits`: `promotions.active_promotions`.

### Permission

`promotions.read`, `promotions.write`, `promotions.publish`, `promotions.codes.manage`,
`promotions.compliance`, `promotions.use` (người mua: ví, lưu, nhập mã nhận, nhắc — gán sẵn cho vai trò người
mua của bước 34), `promotions.vouchers.grant`, `promotions.funding.manage` (đặt phần sàn / nhãn hàng trả),
`promotions.participate` (người bán đăng ký chương trình của sàn) (D-222).

### Mã lỗi (`locales/vi.json`)

| Mã | HTTP | Tiếng Việt |
|---|---|---|
| `promotions.not_found` | 404 | Không tìm thấy chương trình khuyến mại. |
| `promotions.exceeds_legal_limit` | 422 | Mức giảm {value} vượt trần {max} theo {legal_basis} đang hiệu lực. |
| `promotions.exceeds_duration_limit` | 422 | Số ngày giảm giá vượt giới hạn {max} ngày theo {legal_basis} đang hiệu lực. (D-225) |
| `promotions.notice_required` | 409 | Chương trình cần thông báo cho {authority} trước khi bắt đầu; hãy ghi nhận đã nộp. |
| `promotions.no_legal_data` | 409 | Chưa có dữ liệu hạn mức khuyến mại hiệu lực; chạy `autoxide task promotions.load_legal_data`. |
| `promotions.locked_after_start` | 409 | Chương trình đã chạy chỉ được kéo dài, tăng lượt, tạm dừng hay kết thúc. |
| `promotions.invalid_transition` | 409 | Không chuyển được chương trình từ `{from}` sang `{to}`. |
| `promotions.invalid_window` | 422 | Thời gian kết thúc phải sau thời gian bắt đầu. |
| `promotions.code_taken` | 409 | Mã `{code}` đã có. |
| `promotions.code_invalid` | 422 | Mã `{code}` không tồn tại. |
| `promotions.code_expired` | 422 | Mã `{code}` đã hết hạn hoặc chưa bắt đầu. |
| `promotions.code_not_applicable` | 422 | Giỏ hàng chưa đủ điều kiện dùng mã `{code}`: {condition}. |
| `promotions.usage_exhausted` | 409 | Chương trình `{title}` đã hết lượt. |
| `promotions.customer_limit_reached` | 409 | Bạn đã dùng hết lượt của chương trình `{title}`. |
| `promotions.flash_sold_out` | 409 | Suất giá sốc của `{variant}` đã hết. |
| `promotions.changed` | 409 | Khuyến mại của giỏ đã thay đổi; hãy xem lại trước khi đặt. |
| `promotions.redemption_not_found` | 404 | Không có lượt khuyến mại đang giữ cho đơn này. |
| `promotions.redemption_expired` | 409 | Lượt khuyến mại giữ cho đơn đã hết hạn. |
| `promotions.too_many_codes` | 422 | Mỗi giỏ dùng tối đa {max} mã. |
| `promotions.voucher_not_found` | 404 | Không tìm thấy voucher. (D-222) |
| `promotions.voucher_not_owned` | 403 | Voucher này không thuộc tài khoản của bạn. (D-222) |
| `promotions.voucher_expired` | 422 | Voucher đã hết hạn. (D-222) |
| `promotions.voucher_used` | 409 | Voucher đã được dùng. (D-222) |
| `promotions.voucher_quota_exhausted` | 409 | Chương trình `{title}` đã phát hết voucher. (D-222) |
| `promotions.claim_limit_reached` | 409 | Bạn đã nhận đủ số voucher cho phép của chương trình `{title}`. (D-222) |
| `promotions.claim_requires_verified_phone` | 403 | Cần xác thực số điện thoại để nhận voucher này. (D-222) |
| `promotions.budget_exhausted` | 409 | Chương trình `{title}` đã hết ngân sách. (D-222) |
| `promotions.daily_limit_reached` | 409 | Chương trình `{title}` đã hết lượt trong hôm nay. (D-222) |
| `promotions.outside_schedule` | 422 | Chương trình `{title}` chỉ áp dụng trong khung giờ đã công bố. (D-222) |
| `promotions.not_combinable` | 422 | Các khuyến mại đã chọn không dùng chung được. (D-222) |
| `promotions.too_many_vouchers` | 422 | Mỗi lớp khuyến mại dùng tối đa {max} voucher cho mỗi bên. (D-222) |
| `promotions.invalid_funding` | 422 | Tổng tỉ lệ chia tiền giảm phải bằng 100%. (D-222) |
| `promotions.gift_out_of_stock` | 409 | Quà `{variant}` đã hết suất. (D-222) |

Thêm các key lý do `promotions.reason.*` (vd `percent_off`: "Giảm {percent}% cho {target}",
`order_min`: "Giảm {amount} cho đơn từ {min_order}", `free_shipping`, `buy_x_get_y`, `flash_sale`,
`capped_by_law`: "Mức giảm đã giới hạn ở {max}% giá trước khuyến mại theo quy định", `capped_by_floor`,
`budget_exhausted`) và key lý do tương ứng với các mã lỗi mới (D-222).

## Thứ tự làm

Số **mục** theo danh sách bước 43 trong ROADMAP.

1. Khung crate, permission, tính năng / hạn mức, cấu hình; bảng dữ liệu luật dùng chung (kể cả giới hạn số
   ngày) + task nạp gói "vn".
2. Chương trình, bản dịch, mục tiêu, nhóm khách, khung thời gian (`ends_at` bắt buộc), mã (`redeem` / `claim`),
   chuyển trạng thái, phiên bản → **mục 1**.
3. Loại mới, điều kiện mở rộng, khung giờ → **mục 2**, **mục 3**.
4. Bộ tính (`evaluate`) và `CompiledPromotions`: mọi loại, ba lớp và cộng dồn, phân về dòng, chia người trả
   tiền, trần pháp luật và sàn giá, lý do theo ngôn ngữ; bench → **mục 4**, **mục 5**, **mục 6**.
5. `reserve` / `confirm` / `release`, mọi bộ đếm (lượt, mỗi khoá người mua, flash sale, ngân sách, ngày, quà),
   số liệu ngày, job hết hạn, test đồng thời → **mục 7**, **mục 6**.
6. Ví voucher: `grant_voucher`, `vouchers_of`, lưu, nhập mã nhận, cấp tay, hết hạn, dữ liệu cá nhân → **mục 8**.
7. Ai trả tiền, tham gia chương trình của sàn; quy tắc trả hàng / huỷ cho `orders` → **mục 6**, **mục 9**.
8. `api.rs`, REST `/evaluate`, tool MCP, endpoint công khai (danh sách, theo sản phẩm, mã dùng được cho giỏ,
   flash sale), event giá sau giảm tự động, nhắc flash sale → **mục 4**, **mục 12**, **mục 13**.
9. Kiểm tuân thủ khi `activate` và `activation-check`, thời hạn, thông báo theo nơi nhận, xuất dữ liệu thông
   báo, lưu vết; chạy thử, xung đột, lịch, số liệu → **mục 10**, **mục 11**, **mục 14**.
10. `docs/PROMOTIONS.md` + `.vi.md`, OpenAPI, `.env.example`, `docs/BENCHMARKS.md`, CHANGELOG, ROADMAP.

## Test bắt buộc

Cả SQLite và Postgres; `dyn Catalog`, `dyn OrgDirectory` giả.

- **Đồng thời**: `ma_gioi_han_10_luot_100_nguoi` (100 `reserve` cùng mã → đúng 10 thành công, `usage_count
  = 10`); `flash_sale_5_suat_100_nguoi`; `moi_nguoi_mot_luot_gui_dong_thoi` (cùng người, 10 đơn đồng
  thời → 1); `giu_lai_cung_order_id` (16 request → một lần giữ); `xac_nhan_va_het_han_dua_nhau`;
  `tra_lai_dung_bo_dem_da_giu` (hai lần `release` → cộng lại một lần); `voucher_quota_10_nguoi_100` (claim);
  `mot_voucher_hai_don_dong_thoi` (đúng 1); `ngan_sach_khong_vuot_khi_100_don`; `luot_theo_ngay_dong_thoi`;
  `cung_so_dien_thoai_hai_tai_khoan` (khoá chung chặn); `qua_het_suat`;
  `nhac_flash_sale_gui_mot_lan_khi_hai_worker`.
- **Tính**: `cung_gio_cung_ket_qua` (chạy 100 lần, thứ tự dòng xáo trộn → cùng `fingerprint`);
  `exclusive_chan_cong_don` (`exclusivity` `class` / `global`); `giam_theo_don_phan_ve_dong_tong_khop` (tổng
  phần chia = khoản giảm); `khong_giam_xuong_duoi_0`; `mua_x_tang_y`; `mien_phi_van_chuyen`;
  `nhom_khach_va_kenh`; `ma_sai_khong_chan_ca_gio`; `ly_do_theo_ngon_ngu`.
- **Cộng dồn**: ma trận `exclusivity` × `combines_with` × lớp; `stop_after`; `best_for_buyer_tat_dinh` (100 lần,
  thứ tự dòng xáo trộn → cùng kết quả); `voucher_shop_va_voucher_san_cung_gio`.
- **Loại**: `bac_thang_lay_bac_cao_nhat`, `combo_dong_gia_lap_lai`, `mua_kem_deal_soc`, `chon_qua`,
  `mua_2_tinh_1_mon_re_nhat`.
- **Điều kiện**: `theo_phuong_thuc_thanh_toan`, `khung_gio_theo_mui_gio_cua_hang`, `khach_moi`,
  `loai_hang_dang_giam`.
- **Tiền**: `chia_nguoi_tra_tong_khop`, `tran_cong_ca_voucher_san_va_shop`, `san_gia_cat_khoan_giam`.
- **Voucher**: `chi_chu_vi_dung_duoc`, `cap_lai_cung_source_ref_khong_nhan_doi`, `huy_don_tra_voucher_con_han`,
  `xoa_tai_khoan_thu_hoi_voucher`, `voucher_trong_xuat_du_lieu_ca_nhan`.
- **Tuân thủ**: `chan_kich_hoat_vuot_tran_50_phan_tram`; `chuong_trinh_tap_trung_quoc_gia_theo_du_lieu`;
  `tran_doi_theo_ngay_hieu_luc` (hai dòng luật, `at` trước / sau ngày đổi); `cat_ve_tran_khi_tinh`
  (giảm theo số tiền trên danh mục vượt 50% giá tham chiếu của một biến thể → `capped_by_law`);
  `can_thong_bao_truoc_khi_chay`; `xuat_du_lieu_thong_bao_chong_csv_injection`;
  `sua_sau_khi_chay_bi_khoa`; `moi_lan_doi_co_phien_ban`; `khong_bat_duoc_khi_thieu_ends_at`,
  `vuot_so_ngay_giam_gia_trong_nam`, `thong_bao_hai_tinh_mot_tinh_chua_nop`, `kiem_truoc_khi_bat_khong_ghi`
  (D-225).
- **Công khai**: theo sản phẩm (D-200) — mã hết lượt không còn trong danh sách; giá sau mã khớp kết quả
  `evaluate` của giỏ một dòng.
- **Rò rỉ**: bài chung giữa khách (mã của khách A không dùng được ở khách B, cùng chữ mã);
  `ro_ri_giua_hai_cua_hang_anh_em` (chương trình của cửa hàng A không áp cho giỏ ở cửa hàng B, nhân viên
  B không sửa được).
- `api_promotions_qua_testapp` cho từng hàm của trait.
- **Hiệu năng**: bench ở mục "Nhẹ và nhanh" (không là test đỏ / xanh của gate; số đo ghi tài liệu).

## Tiêu chí xong

- [ ] Mọi mục của bước 43 trong ROADMAP được phủ.
- [ ] Không có hạn mức pháp luật viết cứng trong code; xoá dữ liệu luật thì `activate` báo
      `promotions.no_legal_data`, không tự cho qua.
- [ ] `docs/PROMOTIONS.md` + `docs/PROMOTIONS.vi.md` (loại khuyến mại, chương trình / coupon / voucher, thứ tự
      tính và cộng dồn, giữ lượt, tuân thủ, nạp dữ liệu luật); OpenAPI, `.env.example`; số đo bench trong
      `docs/BENCHMARKS.md`; CHANGELOG.
- [ ] DECISIONS: thứ tự tính và cộng dồn; quy tắc làm tròn; trần pháp luật là dữ liệu (căn cứ, ngày
      hiệu lực, "cần đối chiếu"); chặn hay cảnh báo khi chưa thông báo.
- [ ] `./gate.sh` đầy đủ xanh; ROADMAP; commit; phát hành theo `docs/RELEASING.md`.

## Chỗ dễ sai

- **Gọi `Catalog` trong transaction**: lấy biến thể, giá tham chiếu và sàn giá trước, rồi mới mở transaction
  giữ lượt; transaction chạy lại được (`write_with_retry`), không gọi ra ngoài.
- **Thứ tự khoá bộ đếm**: nhiều chương trình / mã / voucher / ngân sách / ngày / khoá người mua / flash item /
  quà trong một lần giữ → `adjust` theo thứ tự (`counter`, `counter_id`) cố định.
- **`ceiling` của `adjust` là hằng**: đọc giới hạn trong cùng transaction; giới hạn bị hạ thấp hơn
  `usage_count` hiện có thì `adjust` trả 0 — đúng (hết lượt), không phải lỗi.
- **Chèn dòng mỗi người / mỗi ngày đua nhau**: `INSERT … ON CONFLICT DO NOTHING` (SeaORM `on_conflict` —
  tra API) rồi `adjust`; không "đọc thấy chưa có thì chèn".
- **Voucher hai đơn cùng lúc**: `available → held` bằng `UPDATE … WHERE status = 'available'`; số dòng đổi = 0
  là đã bị đơn khác giữ.
- **Dấu vân tay**: chuẩn hoá giỏ (sắp dòng theo `line_key`, mã theo chữ hoa) trước khi băm; không đưa
  `at` vào — đưa danh sách chương trình đã áp và số tiền. `voucher_ids`, `payment_method`, `carrier`,
  `ship_to_province_code`, `client`, `membership_tier`, `gift_selections`, `has_compare_at` đi vào
  `fingerprint` (D-222).
- **Điều kiện đọc JSON lúc tính**: biên dịch `promotions_conditions` thành struct khi nạp `CompiledPromotions`.
- **Giá tham chiếu thiếu** (sản phẩm mới, chưa có giá trước khuyến mại): không áp được trần → chặn
  khoản giảm đó (lý do rõ), không coi như không có trần. Ghi quyết định.
- **Làm tròn và phân bổ**: phần trăm trên tổng dòng (không trên đơn giá rồi nhân) để không lệch; phân
  bổ bằng `Money::allocate`; tổng các dòng luôn bằng khoản giảm; phần chia cho người trả tiền khớp từng đồng.
- **Múi giờ**: `starts_at` / `ends_at` lưu UTC; giao diện nhập theo múi giờ của cửa hàng (cài đặt kế
  thừa) — nửa đêm giờ Việt Nam là 17:00 UTC hôm trước. Khung giờ và `day` của lượt theo ngày cũng theo múi giờ
  cửa hàng.
- **Mã**: so bằng `code_key`; sinh mã ngẫu nhiên bằng `secret::random_token` rồi đổi về bảng chữ không
  nhầm (bỏ `0/O`, `1/I`); unique index quyết định, đụng thì sinh lại.
- **Khách vãng lai**: `buyer_keys` là bản băm do `orders` tính — không lưu SĐT / email / địa chỉ thô ở đây.

## Căn cứ pháp lý

Đối chiếu ngày 13/11/2026 qua nguồn tổng hợp — **cần đối chiếu văn bản gốc / người có chuyên môn pháp
lý** trước khi chạy thật (D-186):

- Luật Thương mại 36/2005/QH11, mục khuyến mại (Điều 88 trở đi): các hình thức khuyến mại. Điều 92: "bán hàng,
  cung ứng dịch vụ có kèm theo phiếu mua hàng, phiếu sử dụng dịch vụ" — voucher cấp sau khi mua thuộc hình thức
  này → `promotion_kind = purchase_voucher` trong `promotions_notice_rules` (D-222, cần đối chiếu). Điều 97:
  công bố thời gian bắt đầu, kết thúc khuyến mại → `ends_at` bắt buộc (D-225).
- NĐ 81/2018/NĐ-CP về xúc tiến thương mại (thông báo khuyến mại tới Sở Công Thương trước khi thực
  hiện, nội dung hồ sơ, nơi nhận thông báo, giới hạn tổng số ngày giảm giá cho một loại hàng trong năm), sửa đổi
  bởi NĐ 128/2024/NĐ-CP; NĐ 239/2026 bỏ các điều về hạn mức trong NĐ 81/2018 và giao Bộ Công Thương quy định —
  cần đối chiếu số hiệu và ngày hiệu lực, và miễn phí giao có phải thông báo không (D-225).
- Thông tư 39/2025/TT-BCT: giảm giá tối đa 50% giá hàng hoá, dịch vụ ngay trước thời gian khuyến mại;
  ngoại lệ cho chương trình khuyến mại tập trung quốc gia — dữ liệu `promotions_legal_limits`. Trên sàn, khi
  voucher sàn và voucher của người bán cùng áp, mặc định cộng mọi khoản giảm trên một đơn vị rồi so trần (cách
  an toàn) — chưa rõ trần tính riêng cho từng thương nhân hay gộp (D-222).
- Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15 (từ 01/7/2024): thông tin điều kiện khuyến mại rõ
  ràng → `terms`, `reason`, endpoint công khai; điều kiện coupon / voucher hiện rõ trước khi dùng; số lượt /
  suất còn là số thật (D-222).
- Luật Bảo vệ dữ liệu cá nhân 91/2025: ví voucher là dữ liệu cá nhân — có trong xuất dữ liệu, xoá tài khoản thì
  thu hồi voucher chưa dùng; khoá chống lạm dụng chỉ lưu bản băm (D-222); đồng ý cho mục đích cụ thể — lần bấm
  "Nhắc tôi" là đồng ý cho đúng một lần nhắc (D-225).

## Agent làm bước này tự quyết

- Mặc định khi quy tắc thông báo nói "bắt buộc": chặn `activate` (khuyến nghị) hay chỉ cảnh báo.
- Quy tắc làm tròn — khuyến nghị `MidpointAwayFromZero` về đơn vị nhỏ nhất, trên tổng dòng.
- Trả lượt khi huỷ sau xác nhận: mặc định `return_usage_on_cancel = true`.
- Ghi vào `docs/DECISIONS.md` (skill `record-decision`).
