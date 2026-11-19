//! Hợp đồng của module `orders` cho module khác (`marketplace` bước 46, `customers` 47, `channels`
//! 48, `affiliate` 49, `auth`, `support`, `reporting`…): chỉ những gì ở đây được dùng từ bên ngoài
//! (`autoxide_orders::api::…`). File này không tham chiếu phần bên trong module.
//!
//! Hai cách nối với `orders`:
//!
//! 1. Module đăng ký SAU `orders` lấy trait lúc khởi động:
//!
//!    ```text
//!    let orders = context.require::<dyn autoxide_orders::api::Orders>()?;
//!    ```
//!
//! 2. Module đăng ký SAU `orders` mà `orders` cần gọi tới (người bán của sàn, nhóm khách, điểm /
//!    thẻ quà tặng) tự CẮM vào [`OrderExtensions`] trong `Module::start` của nó — `orders` khởi động
//!    trước nên không `require` được trait của module sau (D-198, D-20261118-step45-orders-api).
//!
//! Mã đơn là `Uuid` (UUIDv7), như `order_id` trong hợp đồng của `inventory`, `promotions`,
//! `shipping`. Mã tham chiếu giữ chỗ chung: [`hold_reference`] (`orders:<order_id>`).

use std::fmt;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, TenantContext, TenantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Tên event `orders` phát (payload `{"before": …, "after": …}`; `after` luôn có `order_id`,
/// `tenant_id`, `org_id`, `channel` khi là event của một đơn). Không event nào mang secret, token
/// hay `Cart-Token`.
pub mod events {
    /// Đơn vừa tạo (`pending_payment` hay `confirmed`).
    pub const ORDER_PLACED: &str = "orders.order_placed";
    /// Đơn đã xác nhận (đã thu đủ, COD đủ điều kiện, POS, kênh): `inventory`, `promotions`, `Tender`
    /// xác nhận chỗ giữ. `after`: `order_id`, `hold_reference`, dòng, kho.
    pub const ORDER_CONFIRMED: &str = "orders.order_confirmed";
    /// Mọi lần đổi `status`.
    pub const ORDER_STATUS_CHANGED: &str = "orders.order_status_changed";
    /// Đơn bị huỷ: bên giữ chỗ trả lại. `after`: `order_id`, `hold_reference`, `cause`.
    pub const ORDER_CANCELLED: &str = "orders.order_cancelled";
    /// Đơn đã giao xong.
    pub const ORDER_DELIVERED: &str = "orders.order_delivered";
    /// Đơn hoàn tất (hết hạn đổi trả, không còn hoãn): mốc hoa hồng, điểm, chi trả người bán.
    pub const ORDER_COMPLETED: &str = "orders.order_completed";
    /// Yêu cầu giao một kiện: `after.shipment` là `ShipmentRequest` của `shipping`.
    pub const FULFILLMENT_REQUESTED: &str = "orders.fulfillment_requested";
    /// Trả hàng cần hãng tới lấy: `after.shipment` là `ShipmentRequest` (`direction = return`).
    pub const RETURN_PICKUP_REQUESTED: &str = "orders.return_pickup_requested";
    /// Đơn nhập từ kênh ngoài thiếu hàng.
    pub const STOCK_SHORTAGE: &str = "orders.stock_shortage";
    /// Ghi nhận một khoản thu của đơn.
    pub const PAYMENT_RECORDED: &str = "orders.payment_recorded";
    /// Đã hoàn tiền (dòng và số tiền hoàn): `Tender` của bước 47 nghe để hoàn điểm / số dư.
    pub const REFUND_COMPLETED: &str = "orders.refund_completed";
    /// Người mua yêu cầu trả hàng (kèm `photo_count`, không có ID ảnh).
    pub const RETURN_REQUESTED: &str = "orders.return_requested";
    /// Hàng trả đã về kho: `inventory` nhập lại. `after`: `return_id`, `order_id`,
    /// `warehouse_org_id`, `lines[{variant_id, quantity, sellable, lot_code?, serials[]}]`.
    pub const RETURN_RECEIVED: &str = "orders.return_received";
    /// Một hoá đơn điện tử đã gắn vào đơn.
    pub const INVOICE_LINKED: &str = "orders.invoice_linked";
    /// Giỏ bị bỏ dở (người mua, tóm tắt giỏ) — cho bước 47.
    pub const CART_ABANDONED: &str = "orders.cart_abandoned";
    /// Đánh giá được đăng.
    pub const REVIEW_PUBLISHED: &str = "orders.review_published";
    /// Đánh giá bị ẩn / mở lại.
    pub const REVIEW_MODERATED: &str = "orders.review_moderated";
    /// Đóng ca bán tại quầy.
    pub const POS_SHIFT_CLOSED: &str = "orders.pos_shift_closed";
    /// Phiếu bảo hành đổi trạng thái.
    pub const WARRANTY_CLAIM_CHANGED: &str = "orders.warranty_claim_changed";
    /// Danh sách chặn COD đổi.
    pub const COD_BLOCK_CHANGED: &str = "orders.cod_block_changed";
    /// Yêu cầu trả phần còn lại của đơn trả trước một phần (số tiền, hạn).
    pub const PREORDER_BALANCE_REQUESTED: &str = "orders.preorder_balance_requested";
    /// Quá hạn trả phần còn lại.
    pub const PREORDER_BALANCE_OVERDUE: &str = "orders.preorder_balance_overdue";
}

/// Mã tham chiếu giữ chỗ chung của một đơn ở mọi bên giữ chỗ (`Tender`, nhật ký): `orders:<id>`.
pub fn hold_reference(order_id: Uuid) -> String {
    format!("orders:{order_id}")
}

/// Trạng thái của đơn (máy trạng thái, đặc tả 45).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    PendingPayment,
    Confirmed,
    Preparing,
    Shipping,
    Delivered,
    DeliveryFailed,
    Completed,
    Cancelled,
}

impl OrderStatus {
    pub const ALL: [OrderStatus; 8] = [
        OrderStatus::PendingPayment,
        OrderStatus::Confirmed,
        OrderStatus::Preparing,
        OrderStatus::Shipping,
        OrderStatus::Delivered,
        OrderStatus::DeliveryFailed,
        OrderStatus::Completed,
        OrderStatus::Cancelled,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            OrderStatus::PendingPayment => "pending_payment",
            OrderStatus::Confirmed => "confirmed",
            OrderStatus::Preparing => "preparing",
            OrderStatus::Shipping => "shipping",
            OrderStatus::Delivered => "delivered",
            OrderStatus::DeliveryFailed => "delivery_failed",
            OrderStatus::Completed => "completed",
            OrderStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        OrderStatus::ALL.into_iter().find(|status| status.as_str() == raw)
    }
}

impl fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Chế độ dòng tiền của đơn (D-216). Ba chế độ đầu cùng tên với `FlowMode` của `ledger`;
/// `ChannelCollect`: kênh bán ngoài (bước 48) thu tiền rồi trả lại cho người bán.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundsFlow {
    SelfCollect,
    PartnerCollect,
    SplitAtGateway,
    ChannelCollect,
}

impl FundsFlow {
    pub const fn as_str(self) -> &'static str {
        match self {
            FundsFlow::SelfCollect => "self_collect",
            FundsFlow::PartnerCollect => "partner_collect",
            FundsFlow::SplitAtGateway => "split_at_gateway",
            FundsFlow::ChannelCollect => "channel_collect",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        [
            FundsFlow::SelfCollect,
            FundsFlow::PartnerCollect,
            FundsFlow::SplitAtGateway,
            FundsFlow::ChannelCollect,
        ]
        .into_iter()
        .find(|flow| flow.as_str() == raw)
    }
}

/// Nguồn đơn được ghi công (tiếp thị liên kết, livestream — bước 49).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Attribution {
    pub click_id: Option<Uuid>,
    pub code: Option<String>,
    pub live_session_id: Option<Uuid>,
}

impl Attribution {
    pub fn new(
        click_id: Option<Uuid>,
        code: Option<String>,
        live_session_id: Option<Uuid>,
    ) -> Self {
        Attribution { click_id, code, live_session_id }
    }
}

/// Một dòng của đơn, đủ cho hoa hồng, phí sàn, điểm, báo cáo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OrderLineSummary {
    pub id: Uuid,
    pub variant_id: Uuid,
    pub product_id: Uuid,
    /// Danh mục đầu tiên của sản phẩm lúc đặt (nếu có).
    pub category_id: Option<Uuid>,
    /// Nút bán của dòng (cửa hàng / người bán).
    pub seller_org_id: OrgId,
    pub quantity: i64,
    /// Số tiền người mua trả cho dòng: sau giảm, gồm thuế.
    pub net: Money,
    /// Thuế của dòng (đã gồm trong `net`).
    pub tax: Money,
    pub returned_quantity: i64,
}

/// Ảnh tóm tắt một đơn cho module khác.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OrderSummary {
    pub id: Uuid,
    pub tenant_id: TenantId,
    /// Nút bán (cửa hàng / người bán).
    pub org_id: OrgId,
    pub number: String,
    pub buyer_user_id: Option<Uuid>,
    /// `web`, `pos` hay mã kênh ngoài.
    pub channel: String,
    pub status: OrderStatus,
    pub funds_flow: FundsFlow,
    pub total: Money,
    pub placed_at: DateTime<Utc>,
    pub lines: Vec<OrderLineSummary>,
    pub attribution: Option<Attribution>,
}

/// Đọc đơn.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro này đổi hàm
// `async` thành hàm trả `Box<dyn Future>` để gọi được.
#[async_trait::async_trait]
pub trait Orders: Send + Sync {
    /// Một đơn, hoặc `None`.
    async fn find(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<OrderSummary>, Error>;

    /// ID các dòng ĐÃ GIAO của biến thể `variant_id` cho người mua `buyer` (đơn `delivered` /
    /// `completed`) — đánh giá, bảo hành, nhãn `verified_buyer` của hỏi đáp (D-202).
    async fn delivered_lines(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        variant_id: Uuid,
    ) -> Result<Vec<Uuid>, Error>;
}

/// Một dòng của đơn nhập từ ngoài: giá do nguồn quyết, không tính lại.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ImportedLine {
    pub variant_id: Uuid,
    pub quantity: i64,
    /// Giá một đơn vị trước giảm, gồm thuế.
    pub unit_price: Money,
    /// Tổng giảm của cả dòng.
    pub discount: Money,
}

impl ImportedLine {
    pub fn new(variant_id: Uuid, quantity: i64, unit_price: Money) -> Self {
        let discount = Money::zero(unit_price.currency());
        ImportedLine { variant_id, quantity, unit_price, discount }
    }

    #[must_use]
    pub fn discount(mut self, discount: Money) -> Self {
        self.discount = discount;
        self
    }
}

/// Đơn từ kênh ngoài hay nhập dữ liệu (bước 48, 56).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ImportedOrder {
    /// Nút bán.
    pub org_id: OrgId,
    /// Mã kênh (`shopee`, `tiktok`, `import`…).
    pub channel: String,
    /// Mã đơn ở kênh; (`channel`, `external_ref`) là khoá chống trùng.
    pub external_ref: String,
    pub currency: Currency,
    pub buyer_user_id: Option<Uuid>,
    pub contact_name: String,
    pub contact_phone: Option<String>,
    pub lines: Vec<ImportedLine>,
    pub shipping_fee: Money,
    pub funds_flow: FundsFlow,
    /// `channel`, `cod`…
    pub payment_method: String,
    pub placed_at: DateTime<Utc>,
}

impl ImportedOrder {
    #[expect(clippy::too_many_arguments, reason = "một đơn nhập có chừng đó trường bắt buộc")]
    pub fn new(
        org_id: OrgId,
        channel: impl Into<String>,
        external_ref: impl Into<String>,
        currency: Currency,
        contact_name: impl Into<String>,
        lines: Vec<ImportedLine>,
        payment_method: impl Into<String>,
        placed_at: DateTime<Utc>,
    ) -> Self {
        ImportedOrder {
            org_id,
            channel: channel.into(),
            external_ref: external_ref.into(),
            currency,
            buyer_user_id: None,
            contact_name: contact_name.into(),
            contact_phone: None,
            lines,
            shipping_fee: Money::zero(currency),
            funds_flow: FundsFlow::ChannelCollect,
            payment_method: payment_method.into(),
            placed_at,
        }
    }
}

/// Đơn từ ngoài (bước 48 kênh bán, 56 nhập dữ liệu).
#[async_trait::async_trait]
pub trait OrderIntake: Send + Sync {
    /// Không trùng theo (`channel`, `external_ref`) — gửi lại trả `Replayed`. Giữ và xác nhận tồn
    /// ngay; thiếu hàng KHÔNG từ chối (kênh đã bán): đơn có `stock_shortage` và event
    /// `orders.stock_shortage`. Đơn vào thẳng `confirmed`.
    async fn import(
        &self,
        tenant: &TenantContext,
        order: ImportedOrder,
    ) -> Result<Submitted<OrderSummary>, Error>;

    /// Kênh báo trạng thái — đi qua cùng máy trạng thái, nguyên nhân `channel`.
    async fn external_transition(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        to: OrderStatus,
        cause_ref: &str,
    ) -> Result<(), Error>;
}

/// Hoàn tiền do module khác quyết (tranh chấp của bước 46): cùng service với `POST /{id}/refunds`.
#[async_trait::async_trait]
pub trait OrderRefunds: Send + Sync {
    /// Trả ID khoản hoàn; cùng `reference` + cùng nội dung → `Replayed`.
    async fn refund(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        amount: Money,
        reason: &str,
        reference: &str,
    ) -> Result<Submitted<Uuid>, Error>;
}

/// Hoãn hoàn tất đơn (tranh chấp của bước 46, yêu cầu trả hàng).
#[async_trait::async_trait]
pub trait OrderHolds: Send + Sync {
    /// Gọi lại cùng `reference` không thêm dòng.
    async fn hold_completion(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        reference: &str,
    ) -> Result<(), Error>;

    async fn release_completion(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        reference: &str,
    ) -> Result<(), Error>;
}

/// Tra cứu cho `auth` (tạo tài khoản từ số đã xác thực qua tra đơn vãng lai, D-202).
#[async_trait::async_trait]
pub trait OrderLookups: Send + Sync {
    /// Số điện thoại (E.164) đã qua OTP của token xem đơn `token`, khi token còn hạn.
    async fn verified_phone(
        &self,
        tenant: &TenantContext,
        token: &str,
    ) -> Result<Option<String>, Error>;
}

/// Một dòng giỏ đưa cho [`SellerSplit`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SplitLine {
    pub line_key: String,
    pub variant_id: Uuid,
    /// Nút sở hữu sản phẩm theo `catalog`.
    pub owner_org_id: OrgId,
    pub quantity: i64,
}

impl SplitLine {
    pub fn new(
        line_key: impl Into<String>,
        variant_id: Uuid,
        owner_org_id: OrgId,
        quantity: i64,
    ) -> Self {
        SplitLine { line_key: line_key.into(), variant_id, owner_org_id, quantity }
    }
}

/// Một nhóm dòng của cùng một người bán (một đơn con).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SellerGroup {
    pub seller_org_id: OrgId,
    pub funds_flow: FundsFlow,
    pub line_keys: Vec<String>,
}

impl SellerGroup {
    pub fn new(seller_org_id: OrgId, funds_flow: FundsFlow, line_keys: Vec<String>) -> Self {
        SellerGroup { seller_org_id, funds_flow, line_keys }
    }
}

/// Tách giỏ theo người bán (bước 46 cắm vào).
#[async_trait::async_trait]
pub trait SellerSplit: Send + Sync {
    /// Gom dòng theo nút bán. Lỗi nếu người bán không được bán (KYC, tạm khoá).
    async fn split(
        &self,
        tenant: &TenantContext,
        lines: &[SplitLine],
    ) -> Result<Vec<SellerGroup>, Error>;
}

/// Hồ sơ giá / khuyến mại của người mua (bước 47 cắm vào).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BuyerProfile {
    /// Nhóm / phân khúc khách (cho `promotions`).
    pub groups: Vec<Uuid>,
    /// Hạng thành viên.
    pub membership_tier: Option<String>,
    /// Mã nhóm khách cho bảng giá của `catalog`.
    pub price_group: Option<String>,
}

impl BuyerProfile {
    pub fn new(
        groups: Vec<Uuid>,
        membership_tier: Option<String>,
        price_group: Option<String>,
    ) -> Self {
        BuyerProfile { groups, membership_tier, price_group }
    }
}

#[async_trait::async_trait]
pub trait BuyerGroups: Send + Sync {
    async fn profile_of(&self, tenant: &TenantContext, buyer: Uuid) -> Result<BuyerProfile, Error>;
}

/// Một cách trả bằng giá trị nội bộ (điểm, thẻ quà tặng, số dư — bước 47 cắm vào). Xác nhận / hoàn
/// lại: bên cài nghe `orders.order_confirmed` / `orders.order_cancelled` / `orders.refund_completed`.
#[async_trait::async_trait]
pub trait Tender: Send + Sync {
    /// `points`, `gift_card`, `store_credit`.
    fn kind(&self) -> &'static str;

    /// Giữ tối đa `max` theo `reference` tới `expires_at`; trả số đã giữ. Gọi lại cùng `reference`
    /// trả lại kết quả cũ.
    async fn hold(
        &self,
        tenant: &TenantContext,
        reference: &str,
        buyer: Option<Uuid>,
        input: &Value,
        max: Money,
        expires_at: DateTime<Utc>,
    ) -> Result<Money, Error>;

    /// Trả chỗ giữ của `reference` (không có thì không làm gì).
    async fn release(&self, tenant: &TenantContext, reference: &str) -> Result<(), Error>;
}

/// Điểm cắm cho module đăng ký SAU `orders` (bước 46–49): gọi trong `Module::start` của họ. Mỗi
/// điểm cắm `set_…` chỉ cắm được một lần; cắm lần hai → lỗi khởi động. Không ai cắm thì `orders`
/// chạy như không có tính năng đó (một người bán, không nhóm khách, không điểm).
pub trait OrderExtensions: Send + Sync {
    fn set_seller_split(&self, split: std::sync::Arc<dyn SellerSplit>) -> Result<(), Error>;
    fn set_buyer_groups(&self, groups: std::sync::Arc<dyn BuyerGroups>) -> Result<(), Error>;
    /// Hai `Tender` cùng `kind()` → lỗi.
    fn add_tender(&self, tender: std::sync::Arc<dyn Tender>) -> Result<(), Error>;
}
