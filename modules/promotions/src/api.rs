//! Hợp đồng của module `promotions` cho module khác (`orders` bước 45, `customers` 47,
//! `support` 53, `automations` 58…): chỉ những gì ở đây được dùng từ bên ngoài
//! (`autoxide_promotions::api::…`). Module khác lấy trait lúc khởi động:
//!
//! ```text
//! let promotions = context.require::<dyn autoxide_promotions::api::Promotions>()?;
//! ```
//!
//! Module đó phải đăng ký SAU `promotions` trong `crates/app/src/modules.rs`. Không module nào đọc
//! bảng `promotions_…` (luật kiến trúc 1). File này không tham chiếu phần bên trong module.
//!
//! Luồng của `orders` (đặc tả 43 và 45):
//!
//! 1. Giỏ đổi → [`Promotions::evaluate`] (không ghi gì) → hiện từng khoản giảm kèm lý do.
//! 2. Bộ chọn mã của website → [`Promotions::eligible`] (mã / voucher dùng được, chưa đủ điều kiện
//!    kèm lý do, tổ hợp lợi nhất).
//! 3. Đặt hàng → [`Promotions::reserve`] với `order_id` và `fingerprint` đã cho người mua xem:
//!    giữ MỌI lượt trong một transaction (đủ hết hoặc không giữ gì). Kết quả khác lúc xem →
//!    `promotions.changed`.
//! 4. Thanh toán xong → [`Promotions::confirm`]; huỷ / thanh toán hỏng → [`Promotions::release`].
//!    Không gọi thì lượt giữ tự hết hạn ở `expires_at` (job `promotions.expire_holds`).
//! 5. Trả hàng → [`returns`] (hàm thuần, tính trên ảnh chụp [`Evaluation`] đã lưu vào đơn).

use std::collections::BTreeMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Số dòng tối đa của một giỏ gửi tới [`Promotions::evaluate`].
pub const MAX_CART_LINES: usize = 500;

/// Tên event `promotions` phát mà module khác hay nghe (payload `{"before": …, "after": …}`).
/// Payload không bao giờ chứa `buyer_keys`.
pub mod events {
    /// Chương trình vừa tạo (`draft`).
    pub const PROMOTION_CREATED: &str = "promotions.promotion_created";
    /// Chương trình đổi (nội dung, bản dịch, mục tiêu…).
    pub const PROMOTION_UPDATED: &str = "promotions.promotion_updated";
    /// Chương trình được bật (`scheduled` hay `active`).
    pub const PROMOTION_ACTIVATED: &str = "promotions.promotion_activated";
    /// Chương trình tới giờ bắt đầu.
    pub const PROMOTION_STARTED: &str = "promotions.promotion_started";
    /// Chương trình tạm dừng.
    pub const PROMOTION_PAUSED: &str = "promotions.promotion_paused";
    /// Chương trình kết thúc (hết `ends_at` hay kết thúc sớm).
    pub const PROMOTION_ENDED: &str = "promotions.promotion_ended";
    /// Chương trình bị huỷ.
    pub const PROMOTION_CANCELLED: &str = "promotions.promotion_cancelled";
    /// Một lô mã vừa sinh.
    pub const CODES_GENERATED: &str = "promotions.codes_generated";
    /// Lượt giữ của một đơn đã xác nhận (`after`: `order_id`, `total_discount`, chương trình).
    pub const REDEMPTION_CONFIRMED: &str = "promotions.redemption_confirmed";
    /// Lượt giữ của một đơn đã trả lại.
    pub const REDEMPTION_RELEASED: &str = "promotions.redemption_released";
    /// Ghi nhận đã nộp thông báo khuyến mại cho một cơ quan.
    pub const NOTICE_SUBMITTED: &str = "promotions.notice_submitted";
    /// Một voucher vừa vào ví người dùng.
    pub const VOUCHER_ISSUED: &str = "promotions.voucher_issued";
    /// Một voucher đã được dùng cho đơn.
    pub const VOUCHER_USED: &str = "promotions.voucher_used";
    /// Chương trình hết ngân sách.
    pub const BUDGET_EXHAUSTED: &str = "promotions.budget_exhausted";
    /// Sàn duyệt / từ chối đăng ký tham gia của người bán.
    pub const PARTICIPATION_REVIEWED: &str = "promotions.participation_reviewed";
    /// Giá sau giảm tự động (lớp `product`, không điều kiện giỏ) của biến thể đổi — cho
    /// `catalog_storefront_entries`. `after`: `{"org_id", "variant_ids": [...]}`; bên nhận hỏi lại
    /// giá bằng [`Promotions::automatic_prices`].
    pub const AUTOMATIC_PRICES_CHANGED: &str = "promotions.automatic_prices_changed";
}

// Lớp cộng dồn của một chương trình (D-222).
/// Stacking class of a promotion: `product` (line items), `order` (whole order, shared out to lines) or `shipping`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[schema(as = PromotionsPromotionClass)]
pub enum PromotionClass {
    // Giảm trên dòng hàng (flash sale, giảm theo sản phẩm / danh mục, mua X tặng Y…).
    Product,
    // Giảm trên cả đơn (giá trị đơn tối thiểu, bậc theo số tiền…), phân về dòng.
    Order,
    // Giảm phí vận chuyển.
    Shipping,
}

// Một dòng của giỏ.
/// One line of a cart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsCartLine)]
pub struct CartLine {
    // Khoá dòng của bên gọi, duy nhất trong giỏ; kết quả trỏ về dòng bằng khoá này.
    pub line_key: String,
    pub variant_id: Uuid,
    // > 0.
    pub quantity: i64,
    // Giá một đơn vị `orders` lấy từ `Catalog::prices`, cùng tiền tệ với giỏ.
    pub unit_price: Money,
    // Dòng đang có giá gạch `compare_at` (điều kiện `exclude_discounted`, D-222).
    pub has_compare_at: bool,
}

impl CartLine {
    pub fn new(
        line_key: impl Into<String>,
        variant_id: Uuid,
        quantity: i64,
        unit_price: Money,
    ) -> Self {
        CartLine {
            line_key: line_key.into(),
            variant_id,
            quantity,
            unit_price,
            has_compare_at: false,
        }
    }
}

// Quà người mua chọn cho chương trình `gift_choice` (D-222).
/// A gift the buyer chose for a `gift_choice` promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = PromotionsGiftSelection)]
pub struct GiftSelection {
    pub promotion_id: Uuid,
    pub variant_id: Uuid,
    pub quantity: i64,
}

// Giỏ cần tính khuyến mại. Dựng bằng [`Cart::new`] rồi gán các trường cần thiết.
/// A cart to compute promotions for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsCart)]
pub struct Cart {
    // Cửa hàng bán.
    #[schema(value_type = Uuid)]
    pub org_id: OrgId,
    pub currency: Currency,
    // Người mua đã đăng nhập (cần cho voucher và hạn mức mỗi người).
    pub buyer_id: Option<Uuid>,
    // Khoá cho hạn mức mỗi người, do `orders` tính: `user:<id>`, băm số điện thoại, băm địa chỉ
    // giao chuẩn hoá. Ở đây chỉ lưu bản băm (D-222).
    pub buyer_keys: Vec<String>,
    // ID nhóm / phân khúc khách của bước 47 (mờ với `promotions`).
    pub customer_groups: Vec<Uuid>,
    // Hạng thành viên (bước 47).
    pub membership_tier: Option<String>,
    // Kênh bán (bước 48).
    pub channel: Option<String>,
    // `web` / `app` / `pos`.
    pub client: Option<String>,
    // Phương thức thanh toán đã chọn; đổi thì tính lại.
    pub payment_method: Option<String>,
    // Hãng giao đã chọn; đổi thì tính lại.
    pub carrier: Option<String>,
    // Mã tỉnh giao hàng (theo `address`).
    pub ship_to_province_code: Option<String>,
    // Người mua chưa có đơn hoàn tất nào trong khách — `orders` điền (`first_order_only`,
    // `new_customer`; ngược lại là `returning_customer`).
    pub first_order: bool,
    pub lines: Vec<CartLine>,
    // Phí vận chuyển đã báo (bước 44), để tính miễn / giảm phí.
    pub shipping_fee: Option<Money>,
    // Coupon người mua nhập (tối đa `AUTOXIDE_PROMOTIONS_MAX_CODES_PER_CART`).
    pub codes: Vec<String>,
    // Voucher trong ví của `buyer_id`.
    pub voucher_ids: Vec<Uuid>,
    pub gift_selections: Vec<GiftSelection>,
    // Thời điểm tính.
    pub at: DateTime<Utc>,
    // Ngôn ngữ của `reason.text`.
    pub locale: String,
}

impl Cart {
    // Giỏ rỗng của cửa hàng `org_id` lúc `at`, ngôn ngữ `en`.
    pub fn new(org_id: OrgId, currency: Currency, at: DateTime<Utc>) -> Self {
        Cart {
            org_id,
            currency,
            buyer_id: None,
            buyer_keys: Vec::new(),
            customer_groups: Vec::new(),
            membership_tier: None,
            channel: None,
            client: None,
            payment_method: None,
            carrier: None,
            ship_to_province_code: None,
            first_order: false,
            lines: Vec::new(),
            shipping_fee: None,
            codes: Vec::new(),
            voucher_ids: Vec::new(),
            gift_selections: Vec::new(),
            at,
            locale: "en".to_owned(),
        }
    }
}

// Lý do người mua đọc được (Luật 19/2023): `code` là i18n key, `params` điền vào câu, `text` là
// câu đã dịch theo `Cart.locale`.
/// Why a discount applies or not, for the buyer: `code` (i18n key), `params` and the translated `text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = PromotionsReason)]
pub struct Reason {
    pub code: String,
    pub params: BTreeMap<String, String>,
    pub text: String,
}

// Ai trả tiền giảm giá (D-222).
/// Who pays for a discount.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[schema(as = PromotionsPayerKind)]
pub enum PayerKind {
    Platform,
    Seller,
    Brand,
    PaymentPartner,
}

// Phần tiền giảm một bên trả.
/// The part of a discount one payer pays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = PromotionsFundingShare)]
pub struct FundingShare {
    pub payer_kind: PayerKind,
    #[schema(value_type = Option<Uuid>)]
    pub payer_org_id: Option<OrgId>,
    pub amount: Money,
}

// Một khoản giảm đã phân về một dòng của giỏ.
/// A discount shared out to one cart line (amount for the whole line).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsLineDiscount)]
pub struct LineDiscount {
    pub line_key: String,
    pub promotion_id: Uuid,
    pub class: PromotionClass,
    // Tiền giảm của cả dòng (không phải mỗi đơn vị); > 0.
    pub amount: Money,
    pub reason: Reason,
    // Đã bị cắt về trần pháp luật.
    pub capped_by_law: bool,
    // Đã bị cắt về sàn giá của `catalog` (quy tắc `block`).
    pub capped_by_floor: bool,
    // Ai trả; tổng khớp `amount` từng đồng.
    pub funding: Vec<FundingShare>,
}

// Một khoản giảm phí vận chuyển.
/// A discount on the shipping fee.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsShippingDiscount)]
pub struct ShippingDiscount {
    pub promotion_id: Uuid,
    pub amount: Money,
    pub reason: Reason,
    pub funding: Vec<FundingShare>,
}

// Hàng tặng / quà: `orders` thêm thành dòng riêng giá 0 và giữ tồn như dòng thường.
/// A free item or gift: add it as its own line at price 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsGiftLine)]
pub struct GiftLine {
    pub promotion_id: Uuid,
    pub variant_id: Uuid,
    pub quantity: i64,
    pub reason: Reason,
}

// Chương trình `gift_choice` đã đạt mà người mua chưa chọn quà: gợi ý, không chặn đặt hàng.
/// A `gift_choice` promotion the cart reached without choosing a gift yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsGiftChoice)]
pub struct GiftChoice {
    pub promotion_id: Uuid,
    // Biến thể được chọn (còn suất).
    pub options: Vec<Uuid>,
    // Số món được chọn.
    pub quantity: i64,
}

// Khi trả món điều kiện thì quà xử lý thế nào (D-222).
/// What happens to the gift when the qualifying item is returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = PromotionsGiftReturnPolicy)]
pub enum GiftReturnPolicy {
    // Trả cả quà (mặc định).
    ReturnGift,
    // Giữ quà, trừ giá trị quà vào tiền hoàn.
    DeductGiftValue,
    // Giữ quà, không trừ.
    Keep,
}

// Trả hàng làm đơn rớt dưới ngưỡng thì thu lại khoản giảm lớp `order` không (cài đặt kế thừa
// `promotions.clawback_on_return`, D-222).
/// Whether order discounts are taken back when a return drops the order below the minimum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = PromotionsClawbackMode)]
pub enum ClawbackMode {
    // Không thu lại (mặc định).
    None,
    // Thu lại phần giảm của các dòng còn giữ khi đơn rớt dưới ngưỡng.
    Proportional,
}

// Một chương trình đã áp vào giỏ.
/// A promotion applied to the cart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsAppliedPromotion)]
pub struct AppliedPromotion {
    pub promotion_id: Uuid,
    // Coupon người mua nhập, nếu áp qua mã.
    pub code: Option<String>,
    // Voucher trong ví, nếu áp qua voucher.
    pub voucher_id: Option<Uuid>,
    pub class: PromotionClass,
    // Tiêu đề và điều kiện theo `Cart.locale` (Luật 19/2023).
    pub title: String,
    pub terms: String,
    // Tổng tiền giảm của chương trình trong giỏ (gồm cả phí vận chuyển).
    pub amount: Money,
    // Giá trị đơn tối thiểu của chương trình, nếu có — cho thu lại khi trả hàng.
    pub min_order: Option<Money>,
    pub gift_return_policy: GiftReturnPolicy,
    // Huỷ trọn đơn sau khi xác nhận thì trả lượt dùng.
    pub return_usage_on_cancel: bool,
}

// Mã hay voucher không dùng được cho giỏ — không làm lỗi cả giỏ.
/// A code or voucher that does not apply to the cart, with the reason (the cart still works).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = PromotionsRejectedCode)]
pub struct RejectedCode {
    pub code: Option<String>,
    pub voucher_id: Option<Uuid>,
    pub reason: Reason,
}

// Khoản giảm bị giữ lại hẳn trên một dòng: chưa có giá tham chiếu ngay trước khuyến mại (không áp
// được trần pháp luật nên không giảm), hay trần / sàn giá cắt về 0.
/// A discount withheld on a line by the legal limit or the price floor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsWithheldDiscount)]
pub struct WithheldDiscount {
    pub promotion_id: Uuid,
    pub line_key: String,
    pub reason: Reason,
}

// Kết quả tính khuyến mại cho một giỏ. `orders` lưu nguyên bản vào đơn (ảnh chụp cho đổi trả,
// thuế, quyết toán).
/// Promotions computed for a cart; orders store it as a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsEvaluation)]
pub struct Evaluation {
    pub currency: Currency,
    // Mọi khoản giảm lớp `product` và `order`, đã phân về từng dòng.
    pub lines: Vec<LineDiscount>,
    // Tổng giảm phí vận chuyển.
    pub shipping_discount: Money,
    pub shipping: Vec<ShippingDiscount>,
    pub gifts: Vec<GiftLine>,
    pub gift_choices: Vec<GiftChoice>,
    // Tổng mọi khoản giảm (dòng + vận chuyển).
    pub total_discount: Money,
    pub applied: Vec<AppliedPromotion>,
    pub rejected_codes: Vec<RejectedCode>,
    // Khoản giảm bị giữ lại vì luật / sàn giá, kèm lý do.
    pub withheld: Vec<WithheldDiscount>,
    // Chế độ thu lại khi trả hàng của cửa hàng lúc tính.
    pub clawback_on_return: ClawbackMode,
    // Dấu vân tay của kết quả; `orders` gửi lại khi giữ ([`ReserveRequest::expected_fingerprint`]).
    pub fingerprint: String,
}

impl Evaluation {
    // Tổng tiền giảm của dòng `line_key` (mọi chương trình).
    pub fn line_discount(&self, line_key: &str) -> Money {
        let total: i64 = self
            .lines
            .iter()
            .filter(|line| line.line_key == line_key)
            .map(|line| line.amount.minor())
            .sum();
        Money::new(total, self.currency)
    }
}

// Một mã / voucher trong bộ chọn của website (D-201).
/// A code or voucher in the promotion picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsOffer)]
pub struct Offer {
    pub promotion_id: Uuid,
    pub code: Option<String>,
    pub voucher_id: Option<Uuid>,
    pub class: PromotionClass,
    pub title: String,
    pub terms: String,
    // Tiền giảm cho giỏ này khi dùng riêng mã / voucher này (0 khi chưa đủ điều kiện).
    pub discount: Money,
    // Số lượt còn lại — chỉ có khi chương trình có giới hạn lượt, là số thật.
    pub remaining_uses: Option<i64>,
    pub ends_at: DateTime<Utc>,
}

// Mã / voucher chưa dùng được, kèm lý do ("thêm 120.000 ₫", "chỉ cho đơn đầu tiên").
/// A code or voucher the cart does not qualify for yet, with the reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsIneligibleOffer)]
pub struct IneligibleOffer {
    pub offer: Offer,
    pub reason: Reason,
}

// Tổ hợp mã + voucher người mua nên chọn.
/// Codes and vouchers to use together.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = PromotionsSelection)]
pub struct Selection {
    pub codes: Vec<String>,
    pub voucher_ids: Vec<Uuid>,
}

// Mã và voucher cho bộ chọn của một giỏ (D-201, D-222).
/// Codes and vouchers usable for a cart, those not yet, and the best combination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsEligibility)]
pub struct Eligibility {
    // Coupon `public` và voucher trong ví dùng được cho giỏ, giảm nhiều trước.
    pub usable: Vec<Offer>,
    // Coupon `public` và voucher trong ví chưa đủ điều kiện.
    pub ineligible: Vec<IneligibleOffer>,
    // Tổ hợp lợi nhất theo luật cộng dồn.
    pub best_selection: Selection,
    // Tiền giảm của `best_selection` (cùng các giảm tự động).
    pub best_total_discount: Money,
}

// Yêu cầu giữ lượt cho một đơn.
/// Hold the promotion usage of a cart for an order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsReserveRequest)]
pub struct ReserveRequest {
    pub order_id: Uuid,
    pub cart: Cart,
    // Dấu vân tay người mua đã xem; khác kết quả tính lại → `promotions.changed`.
    pub expected_fingerprint: Option<String>,
    // Hạn giữ; tối đa `AUTOXIDE_PROMOTIONS_MAX_HOLD_HOURS` sau bây giờ.
    pub expires_at: DateTime<Utc>,
}

impl ReserveRequest {
    pub fn new(order_id: Uuid, cart: Cart, expires_at: DateTime<Utc>) -> Self {
        ReserveRequest { order_id, cart, expected_fingerprint: None, expires_at }
    }

    pub fn expecting(mut self, fingerprint: impl Into<String>) -> Self {
        self.expected_fingerprint = Some(fingerprint.into());
        self
    }
}

// Trạng thái lượt giữ của một đơn.
/// State of the promotion usage held for an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = PromotionsRedemptionStatus)]
pub enum RedemptionStatus {
    Held,
    Confirmed,
    Released,
    Expired,
}

// Lượt giữ của một đơn.
/// The promotion usage held for an order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsRedemption)]
pub struct Redemption {
    pub id: Uuid,
    pub order_id: Uuid,
    pub status: RedemptionStatus,
    // Ảnh chụp kết quả tính lúc giữ.
    pub evaluation: Evaluation,
    // Hạn giữ (khi `Held`).
    pub expires_at: Option<DateTime<Utc>>,
}

// Trạng thái của voucher trong ví.
/// State of a voucher in a wallet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = PromotionsVoucherStatus)]
pub enum VoucherStatus {
    Available,
    Held,
    Used,
    Expired,
    Revoked,
}

// Voucher vào ví bằng cách nào.
/// How a voucher came into the wallet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = PromotionsVoucherSource)]
pub enum VoucherSource {
    // Người mua bấm "Lưu".
    Claim,
    // Người mua nhập mã nhận.
    ClaimCode,
    // Nhân viên cấp tay.
    Grant,
    Loyalty,
    PurchaseReward,
    Birthday,
    Referral,
    Compensation,
    Automation,
}

// Lệnh cấp một voucher vào ví (D-222). Gọi lại cùng `promotion_id` + `user_id` + `source` +
// `source_ref` thì trả voucher cũ, không cấp thêm.
/// Put a voucher into a user's wallet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsGrantVoucher)]
pub struct GrantVoucher {
    pub promotion_id: Uuid,
    pub user_id: Uuid,
    pub source: VoucherSource,
    // Mã tham chiếu của bên cấp, vd `birthday:2026`, `points:<id>`, `purchase:<order_id>`;
    // 1–200 ký tự.
    pub source_ref: String,
}

impl GrantVoucher {
    pub fn new(
        promotion_id: Uuid,
        user_id: Uuid,
        source: VoucherSource,
        source_ref: impl Into<String>,
    ) -> Self {
        GrantVoucher { promotion_id, user_id, source, source_ref: source_ref.into() }
    }
}

// Một voucher trong ví.
/// A one-time voucher in a user's wallet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsVoucher)]
pub struct Voucher {
    pub id: Uuid,
    pub promotion_id: Uuid,
    // `None` khi tài khoản đã xoá (voucher đã dùng giữ cho chứng từ).
    pub holder_user_id: Option<Uuid>,
    pub status: VoucherStatus,
    pub source: VoucherSource,
    pub source_ref: String,
    pub valid_from: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub issued_at: DateTime<Utc>,
    pub used_order_id: Option<Uuid>,
}

// Giá sau giảm tự động của một biến thể (lớp `product`, không điều kiện giỏ) — cho danh sách
// công khai của `catalog` (D-200, D-208…D-218).
/// The automatic sale price of a variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
#[schema(as = PromotionsAutomaticPrice)]
pub struct AutomaticPrice {
    pub variant_id: Uuid,
    pub promotion_id: Uuid,
    // Giá một đơn vị sau giảm.
    pub sale_price: Money,
    pub ends_at: DateTime<Utc>,
}

/// Khuyến mại cho module khác.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro này đổi hàm
// `async` thành hàm trả về `Box<dyn Future>` để gọi được.
#[async_trait::async_trait]
pub trait Promotions: Send + Sync {
    /// Tính giảm giá cho giỏ — không giữ gì, không ghi gì. Mã / voucher sai không làm lỗi cả
    /// giỏ (vào `rejected_codes`). Lỗi: giỏ sai hình dạng (`validation_failed`), quá nhiều mã
    /// (`promotions.too_many_codes`).
    async fn evaluate(&self, tenant: &TenantContext, cart: &Cart) -> Result<Evaluation, Error>;

    /// Mã `public`, voucher trong ví dùng được và chưa đủ điều kiện (kèm lý do) cho giỏ, cùng
    /// `best_selection` — không ghi gì. `orders` phục vụ ở
    /// `GET /api/v1/orders/carts/{id}/promotions`.
    async fn eligible(&self, tenant: &TenantContext, cart: &Cart) -> Result<Eligibility, Error>;

    /// Tính lại và giữ mọi lượt (chương trình, mã, voucher, ngân sách, ngày, mỗi khoá người mua,
    /// flash sale, quà) trong MỘT transaction: đủ hết hoặc không giữ gì. Gọi lại cùng `order_id`
    /// và cùng giỏ → `Replayed`; cùng `order_id` mà giỏ khác → `reference_reused`. Hết lượt →
    /// `promotions.usage_exhausted`, `promotions.customer_limit_reached`,
    /// `promotions.flash_sold_out`, `promotions.budget_exhausted`, `promotions.daily_limit_reached`,
    /// `promotions.gift_out_of_stock`, `promotions.voucher_used`; kết quả khác lúc xem →
    /// `promotions.changed`.
    async fn reserve(
        &self,
        tenant: &TenantContext,
        request: &ReserveRequest,
    ) -> Result<Submitted<Redemption>, Error>;

    /// Giữ → đã dùng. Gọi lại: trả như cũ. Đã hết hạn / đã trả: `promotions.redemption_expired`;
    /// không có: `promotions.redemption_not_found`.
    async fn confirm(&self, tenant: &TenantContext, order_id: Uuid) -> Result<Redemption, Error>;

    /// Trả lượt (huỷ đơn, thanh toán hỏng). Đã xác nhận thì trả lượt của chương trình cho phép
    /// (`return_usage_on_cancel`); voucher về ví nếu còn hạn. Gọi lại: trả như cũ, không cộng
    /// lại lần nữa. Không có lượt giữ cho đơn: `Ok(None)`.
    async fn release(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
    ) -> Result<Option<Redemption>, Error>;

    /// Lượt giữ của đơn `order_id`, nếu có.
    async fn redemption(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
    ) -> Result<Option<Redemption>, Error>;

    /// Cấp voucher vào ví. Gọi lại cùng `source` + `source_ref` → `Replayed` với voucher cũ.
    /// Lỗi: `promotions.not_found`, `promotions.voucher_quota_exhausted`,
    /// `promotions.claim_limit_reached`, chương trình không phải `voucher` hay đã kết thúc
    /// (`promotions.invalid_transition`).
    async fn grant_voucher(
        &self,
        tenant: &TenantContext,
        request: &GrantVoucher,
    ) -> Result<Submitted<Voucher>, Error>;

    /// Ví voucher của một tài khoản, hết hạn gần nhất trước.
    async fn vouchers_of(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        status: Option<VoucherStatus>,
    ) -> Result<Vec<Voucher>, Error>;

    /// Giá sau giảm tự động lớp `product` (không điều kiện giỏ) của các biến thể ở cửa hàng
    /// `org_id` lúc `at`, từ giá niêm yết `prices` của bên gọi. Biến thể không được giảm thì vắng.
    async fn automatic_prices(
        &self,
        tenant: &TenantContext,
        org_id: OrgId,
        at: DateTime<Utc>,
        prices: &[(Uuid, Money)],
    ) -> Result<Vec<AutomaticPrice>, Error>;
}

/// Quy tắc trả hàng và huỷ cho `orders` (D-222): hàm thuần trên ảnh chụp [`Evaluation`] đã lưu
/// vào đơn — không gọi database.
pub mod returns {
    use autoxide_kernel::error::Error;
    use autoxide_kernel::money::Money;

    use super::{ClawbackMode, Evaluation, PromotionClass};

    /// Tiền hoàn cho `returning` đơn vị của một dòng đã trả `paid` cho `bought` đơn vị, khi trước
    /// đó đã trả lại `already_returned` đơn vị. Mỗi đơn vị nhận phần nguyên của `paid / bought`,
    /// đơn vị CUỐI nhận phần dư — nên tổng hoàn qua nhiều lần trả đúng bằng `paid`, không vượt.
    /// Số lượng sai (âm, vượt số mua) → `promotions.invalid_return`.
    pub fn line_refund(
        paid: Money,
        bought: i64,
        already_returned: i64,
        returning: i64,
    ) -> Result<Money, Error> {
        if bought <= 0
            || already_returned < 0
            || returning < 0
            || already_returned.checked_add(returning).is_none_or(|total| total > bought)
        {
            return Err(invalid_return());
        }
        let unit = paid.minor() / bought;
        let mut refund = unit.checked_mul(returning).ok_or_else(invalid_return)?;
        if already_returned + returning == bought && returning > 0 {
            // Đơn vị cuối nhận phần dư.
            refund += paid.minor() - unit * bought;
        }
        Ok(Money::new(refund, paid.currency()))
    }

    /// Tiền giảm lớp `order` phải thu lại khi trả hàng làm đơn còn `kept_subtotal` (giá trị
    /// hàng còn giữ, trước giảm). [`ClawbackMode::None`] → 0. [`ClawbackMode::Proportional`]:
    /// với mỗi chương trình lớp `order` có `min_order` mà `kept_subtotal` rớt dưới ngưỡng, thu lại
    /// phần giảm đã phân cho các dòng còn giữ (`kept_line_keys`).
    pub fn order_clawback(
        evaluation: &Evaluation,
        kept_line_keys: &[&str],
        kept_subtotal: Money,
    ) -> Result<Money, Error> {
        let mut total = Money::zero(evaluation.currency);
        if evaluation.clawback_on_return == ClawbackMode::None {
            return Ok(total);
        }
        for applied in &evaluation.applied {
            let Some(min_order) = applied.min_order else { continue };
            if applied.class != PromotionClass::Order
                || kept_subtotal.compare(min_order)? != std::cmp::Ordering::Less
            {
                continue;
            }
            for line in &evaluation.lines {
                if line.promotion_id == applied.promotion_id
                    && kept_line_keys.contains(&line.line_key.as_str())
                {
                    total = total.checked_add(line.amount)?;
                }
            }
        }
        Ok(total)
    }

    fn invalid_return() -> Error {
        Error::new(
            autoxide_kernel::error::ErrorKind::Validation,
            "promotions.invalid_return",
            "The returned quantity must be between 0 and the quantity bought.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::returns::line_refund;
    use autoxide_kernel::money::{Currency, Money};

    #[test]
    fn hoan_tung_phan_khong_vuot_tien_da_tra() {
        let vnd = Currency::parse("VND").unwrap();
        let paid = Money::new(100, vnd);
        let first = line_refund(paid, 3, 0, 1).unwrap();
        let second = line_refund(paid, 3, 1, 1).unwrap();
        let last = line_refund(paid, 3, 2, 1).unwrap();
        assert_eq!((first.minor(), second.minor(), last.minor()), (33, 33, 34));
        assert_eq!(line_refund(paid, 3, 0, 3).unwrap().minor(), 100);
        assert!(line_refund(paid, 3, 2, 2).is_err());
    }
}
