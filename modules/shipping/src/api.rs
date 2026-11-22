//! Hợp đồng của module `shipping` cho module khác (`orders`, bộ nối hãng vận chuyển
//! `shipping-<hãng>`, `support`…): chỉ những gì ở đây được dùng từ bên ngoài
//! (`autoxide_shipping::api::…`). Module khác lấy trait lúc khởi động:
//!
//! ```text
//! let rates = context.require::<dyn autoxide_shipping::api::ShippingRates>()?;
//! let book = context.require::<dyn autoxide_shipping::api::AddressBook>()?;
//! let carriers = context.require::<dyn autoxide_shipping::api::CarrierRegistry>()?;
//! ```
//!
//! Module đó phải đăng ký SAU `shipping` trong `crates/app/src/modules.rs`. Không module nào đọc
//! bảng `shipping_…` (luật kiến trúc 1). File này không tham chiếu phần bên trong module
//! (`crate::…`), để bộ giả [`fake::FakeCarrier`] dùng được từ testkit và bước 45.
//!
//! `shipping` **không phụ thuộc `orders`**: nhận yêu cầu giao qua event (tên ở [`events`]), payload
//! `after.shipment` là [`ShipmentRequest`]; báo trạng thái qua event của chính nó.

use autoxide_kernel::time::{Date, Timestamp};
use std::fmt;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use autoxide_address::api::Address;

/// Tên event: `shipping` nghe của `orders` và phát cho module khác.
pub mod events {
    /// `orders` xin giao một phần / cả đơn: payload `{"after": {"shipment": ShipmentRequest}}`.
    pub const ORDERS_FULFILLMENT_REQUESTED: &str = "orders.fulfillment_requested";
    /// `orders` xin lấy hàng trả từ người mua (`direction = return`), payload như trên.
    pub const ORDERS_RETURN_PICKUP_REQUESTED: &str = "orders.return_pickup_requested";
    /// `orders` huỷ đơn: payload `{"after": {"order_id": "<uuid>"}}` — huỷ các vận đơn chưa lấy hàng.
    pub const ORDERS_ORDER_CANCELLED: &str = "orders.order_cancelled";

    /// Vận đơn vừa tạo (`pending`). Payload `{"after": ShipmentEvent}`.
    pub const SHIPMENT_CREATED: &str = "shipping.shipment_created";
    /// Đã đặt với hãng, có mã vận đơn.
    pub const SHIPMENT_BOOKED: &str = "shipping.shipment_booked";
    /// Mọi lần đổi trạng thái (gồm cả các event riêng bên dưới).
    pub const SHIPMENT_STATUS_CHANGED: &str = "shipping.shipment_status_changed";
    pub const SHIPMENT_DELIVERED: &str = "shipping.shipment_delivered";
    pub const SHIPMENT_DELIVERY_FAILED: &str = "shipping.shipment_delivery_failed";
    pub const SHIPMENT_RETURNED: &str = "shipping.shipment_returned";
    pub const SHIPMENT_CANCELLED: &str = "shipping.shipment_cancelled";
    /// Đặt với hãng thất bại hẳn (vận đơn về `pending`).
    pub const BOOKING_FAILED: &str = "shipping.booking_failed";
    /// Hãng đã thu tiền COD (giao xong) — `orders` ghi nhận đơn đã thu.
    pub const COD_COLLECTED: &str = "shipping.cod_collected";
    /// Bảng kê của hãng khớp: tiền COD đã về người bán / đối tác thu hộ (D-216).
    pub const COD_SETTLED: &str = "shipping.cod_settled";
    /// Bảng kê lệch với vận đơn (hay vận đơn quá hạn chưa vào bảng kê).
    pub const COD_MISMATCH: &str = "shipping.cod_mismatch";
    /// Một chênh lệch COD được giải quyết (mã lý do + ghi chú).
    pub const COD_DISCREPANCY_RESOLVED: &str = "shipping.cod_discrepancy_resolved";
    /// Tài khoản hãng thêm / sửa / kiểm (không mang thông tin đăng nhập).
    pub const CARRIER_ACCOUNT_CHANGED: &str = "shipping.carrier_account_changed";
}

// ---- Sổ địa chỉ người mua -----------------------------------------------------------------------

/// Một địa chỉ trong sổ của người mua.
///
/// Địa chỉ Việt Nam: `address` theo đơn vị hành chính hai cấp (module `address`). Địa chỉ nước
/// ngoài dạng chung: `address.province_code` / `province_name` là vùng (bang, tỉnh…),
/// `address.commune_name` là thành phố, `commune_code` rỗng, kèm `postal_code`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BuyerAddress {
    pub id: Uuid,
    pub buyer_id: Uuid,
    pub label: String,
    pub recipient_name: String,
    pub phone: String,
    pub address: Address,
    pub postal_code: Option<String>,
    pub is_default: bool,
    pub updated_at: Timestamp,
}

/// Sổ địa chỉ của người mua, cho `orders` (giỏ, đặt hàng).
#[async_trait::async_trait]
pub trait AddressBook: Send + Sync {
    /// Địa chỉ `id` của người mua `buyer_id`; của người khác hay không có →
    /// `shipping.address_not_found`.
    async fn get(
        &self,
        tenant: &TenantContext,
        buyer_id: Uuid,
        id: Uuid,
    ) -> Result<BuyerAddress, Error>;
    /// Địa chỉ mặc định của người mua, nếu có.
    async fn default_for(
        &self,
        tenant: &TenantContext,
        buyer_id: Uuid,
    ) -> Result<Option<BuyerAddress>, Error>;
}

// ---- Báo giá, cho `orders` ----------------------------------------------------------------------

/// Câu hỏi báo giá: các cách giao một giỏ hàng từ cửa hàng `org_id` tới `destination`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct QuoteRequest {
    /// Cửa hàng bán (phương thức kế thừa theo cây tổ chức).
    pub org_id: OrgId,
    /// Kho xuất nếu đã biết (từ chỗ giữ của `inventory`).
    pub origin_org_id: Option<OrgId>,
    /// `None`: chỉ trả phương thức nhận tại cửa hàng.
    pub destination: Option<Address>,
    pub items: Vec<QuoteItem>,
    /// Giá trị đơn sau giảm giá, để xét miễn phí vận chuyển.
    pub order_value: Money,
    /// Lúc tính (ngày giao dự kiến, giờ chốt).
    pub at: Timestamp,
    /// Ngôn ngữ của tên phương thức.
    pub locale: String,
    /// Người nhận (tên, số điện thoại) khi đã biết, chuyển cho hãng lúc báo phí.
    #[serde(default)]
    pub recipient: Option<(String, String)>,
}

impl QuoteRequest {
    pub fn new(org_id: OrgId, items: Vec<QuoteItem>, order_value: Money, at: Timestamp) -> Self {
        QuoteRequest {
            org_id,
            origin_org_id: None,
            destination: None,
            items,
            order_value,
            at,
            locale: "en".to_owned(),
            recipient: None,
        }
    }

    /// Người nhận (tên, số điện thoại), nếu đã biết.
    pub fn recipient(mut self, name: impl Into<String>, phone: impl Into<String>) -> Self {
        self.recipient = Some((name.into(), phone.into()));
        self
    }

    pub fn destination(mut self, address: Address) -> Self {
        self.destination = Some(address);
        self
    }

    pub fn origin(mut self, org: OrgId) -> Self {
        self.origin_org_id = Some(org);
        self
    }

    pub fn locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = locale.into();
        self
    }
}

/// Một dòng của giỏ, số liệu lấy từ `VariantSnapshot` của `catalog` (`shipping` không gọi
/// `catalog`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct QuoteItem {
    pub variant_id: Uuid,
    pub quantity: i64,
    pub requires_shipping: bool,
    /// Cân nặng một món, gram.
    pub weight_grams: Option<i64>,
    pub length_mm: Option<i64>,
    pub width_mm: Option<i64>,
    pub height_mm: Option<i64>,
    /// Mã hàng, để báo `shipping.weight_missing` cho đúng món.
    pub sku: String,
}

impl QuoteItem {
    /// Món cần giao, chưa có cân nặng / kích thước.
    pub fn new(variant_id: Uuid, quantity: i64) -> Self {
        QuoteItem {
            variant_id,
            quantity,
            requires_shipping: true,
            weight_grams: None,
            length_mm: None,
            width_mm: None,
            height_mm: None,
            sku: String::new(),
        }
    }

    pub fn sku(mut self, sku: impl Into<String>) -> Self {
        self.sku = sku.into();
        self
    }

    pub fn requires_shipping(mut self, requires: bool) -> Self {
        self.requires_shipping = requires;
        self
    }

    pub fn weight(mut self, grams: Option<i64>) -> Self {
        self.weight_grams = grams;
        self
    }

    /// Dài × rộng × cao, mm.
    pub fn dimensions(
        mut self,
        length_mm: Option<i64>,
        width_mm: Option<i64>,
        height_mm: Option<i64>,
    ) -> Self {
        self.length_mm = length_mm;
        self.width_mm = width_mm;
        self.height_mm = height_mm;
        self
    }
}

/// Kind of shipping method: `table_rate` (the shop's own weight tiers per zone), `carrier` (the fee
/// a carrier account quotes), `pickup` (collect in store).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = ShippingMethodKind)]
pub enum MethodKind {
    /// Bảng phí khách tự đặt (vùng × bậc cân).
    TableRate,
    /// Phí do hãng vận chuyển báo.
    Carrier,
    /// Nhận tại cửa hàng.
    Pickup,
}

impl MethodKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MethodKind::TableRate => "table_rate",
            MethodKind::Carrier => "carrier",
            MethodKind::Pickup => "pickup",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "table_rate" => Some(MethodKind::TableRate),
            "carrier" => Some(MethodKind::Carrier),
            "pickup" => Some(MethodKind::Pickup),
            _ => None,
        }
    }
}

/// Ngày giao dự kiến: ngày tuyệt đối theo múi giờ của lịch giao (D-201).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Eta {
    pub earliest_date: Date,
    pub latest_date: Date,
    /// Đặt trước lúc này thì kịp chuyến hôm nay (giờ chốt còn trong ngày), không thì `None`.
    pub order_by: Option<Timestamp>,
    /// Múi giờ IANA của hai ngày trên, vd `Asia/Ho_Chi_Minh`.
    pub time_zone: String,
}

impl Eta {
    pub fn new(
        earliest_date: Date,
        latest_date: Date,
        order_by: Option<Timestamp>,
        time_zone: impl Into<String>,
    ) -> Self {
        Eta { earliest_date, latest_date, order_by, time_zone: time_zone.into() }
    }
}

/// Một cách giao hợp lệ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ShippingOption {
    pub method_id: Uuid,
    pub method_code: String,
    pub name: String,
    pub kind: MethodKind,
    /// Phí người mua trả (0 khi miễn phí), đã gồm thuế GTGT của phí giao.
    pub amount: Money,
    pub free_shipping_applied: bool,
    /// Ngưỡng miễn phí của phương thức (để `orders` báo "mua thêm X để miễn phí").
    pub free_over: Option<Money>,
    /// Nhóm thuế của phí giao trong engine thuế (bước 36), vd `vat.standard`; `orders` ghi
    /// dòng thuế của phí giao lên hoá đơn theo nhóm này (D-208…D-218).
    pub tax_category: String,
    pub carrier_code: Option<String>,
    pub service_code: Option<String>,
    /// Cửa hàng nhận hàng, với `Pickup`.
    pub pickup_org_id: Option<OrgId>,
    pub chargeable_weight_grams: i64,
    /// Số ngày giao (ít nhất, nhiều nhất) như phương thức khai.
    pub estimated_days: Option<(u16, u16)>,
    pub eta: Option<Eta>,
    /// Nhận tại cửa hàng: lúc hàng sẵn sàng (chuẩn bị + giờ mở cửa).
    pub ready_at: Option<Timestamp>,
    /// Nhận tại cửa hàng: số ngày giữ hàng chờ nhận.
    pub hold_days: Option<u16>,
    /// Dấu vân tay (phương thức, số tiền, đích) — `orders` gửi lại khi đặt; lệch →
    /// `shipping.option_changed`.
    pub option_key: String,
}

/// Báo giá.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Quote {
    /// Rẻ trước. Rỗng khi giỏ không có món nào cần giao.
    pub options: Vec<ShippingOption>,
    /// Phương thức không dùng được cho giỏ / đích này, kèm mã lý do.
    pub unavailable: Vec<UnavailableMethod>,
}

/// Phương thức không dùng được và lý do (`shipping.weight_missing`, `shipping.carrier_unavailable`…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UnavailableMethod {
    pub method_code: String,
    pub reason_code: String,
}

impl UnavailableMethod {
    pub fn new(method_code: impl Into<String>, reason_code: impl Into<String>) -> Self {
        UnavailableMethod { method_code: method_code.into(), reason_code: reason_code.into() }
    }
}

/// Báo giá phí giao (module `shipping` công bố).
#[async_trait::async_trait]
pub trait ShippingRates: Send + Sync {
    /// Các cách giao hợp lệ, rẻ trước. Hãng lỗi / quá giờ: phương thức đó vào `unavailable`,
    /// không làm hỏng cả báo giá. Không gọi trong transaction ghi (có thể gọi hãng).
    async fn quote(&self, tenant: &TenantContext, request: &QuoteRequest) -> Result<Quote, Error>;

    /// Kiểm lại một cách giao lúc đặt hàng: tính lại báo giá, trả đúng phương thức có
    /// `option_key`; phí hay đích đã đổi → `shipping.option_changed` (409).
    async fn confirm_option(
        &self,
        tenant: &TenantContext,
        request: &QuoteRequest,
        option_key: &str,
    ) -> Result<ShippingOption, Error>;
}

// ---- Yêu cầu giao: `orders` nhúng vào payload event --------------------------------------------

/// Direction of a shipment: `outbound` (to the buyer) or `return` (picked up from the buyer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = ShippingDirection)]
pub enum Direction {
    /// Giao cho người mua.
    Outbound,
    /// Lấy hàng trả từ người mua về kho.
    Return,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Outbound => "outbound",
            Direction::Return => "return",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "outbound" => Some(Direction::Outbound),
            "return" => Some(Direction::Return),
            _ => None,
        }
    }
}

/// Một yêu cầu giao (hay lấy hàng trả). `(order_id, request_key)` là khoá chống trùng: gửi lại
/// cùng nội dung không tạo vận đơn thứ hai; cùng khoá khác nội dung bị từ chối.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ShipmentRequest {
    pub order_id: Uuid,
    pub order_number: String,
    /// Một đơn có thể giao nhiều lần (giao một phần): mỗi lần một khoá.
    pub request_key: String,
    pub direction: Direction,
    /// Cửa hàng bán.
    pub org_id: OrgId,
    /// Kho xuất / kho nhận hàng trả.
    pub origin_org_id: OrgId,
    pub method_id: Uuid,
    pub option_key: Option<String>,
    /// Người mua (chủ đơn) — để trả lời tự động về hành trình chỉ cho chủ đơn (D-230).
    pub buyer_id: Option<Uuid>,
    pub recipient_name: String,
    pub recipient_phone: String,
    pub address: Address,
    pub items: Vec<ShipmentItem>,
    pub declared_value: Money,
    /// Tiền hãng thu hộ khi giao; `None`: không thu hộ.
    pub cod_amount: Option<Money>,
    /// Phí giao đã báo người mua (từ báo giá).
    pub quoted_fee: Option<Money>,
    pub note: String,
}

impl ShipmentRequest {
    /// Yêu cầu giao đi (`Outbound`), chưa có COD, phí báo giá, ghi chú.
    #[expect(clippy::too_many_arguments, reason = "một yêu cầu giao có chừng đó trường bắt buộc")]
    pub fn new(
        order_id: Uuid,
        order_number: impl Into<String>,
        request_key: impl Into<String>,
        org_id: OrgId,
        origin_org_id: OrgId,
        method_id: Uuid,
        recipient: (String, String),
        address: Address,
        items: Vec<ShipmentItem>,
        declared_value: Money,
    ) -> Self {
        ShipmentRequest {
            order_id,
            order_number: order_number.into(),
            request_key: request_key.into(),
            direction: Direction::Outbound,
            org_id,
            origin_org_id,
            method_id,
            option_key: None,
            buyer_id: None,
            recipient_name: recipient.0,
            recipient_phone: recipient.1,
            address,
            items,
            declared_value,
            cod_amount: None,
            quoted_fee: None,
            note: String::new(),
        }
    }

    pub fn direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    pub fn option_key(mut self, key: impl Into<String>) -> Self {
        self.option_key = Some(key.into());
        self
    }

    pub fn buyer(mut self, buyer_id: Uuid) -> Self {
        self.buyer_id = Some(buyer_id);
        self
    }

    pub fn cod(mut self, amount: Money) -> Self {
        self.cod_amount = Some(amount);
        self
    }

    pub fn quoted_fee(mut self, fee: Money) -> Self {
        self.quoted_fee = Some(fee);
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }
}

/// Một dòng hàng trong yêu cầu giao.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ShipmentItem {
    /// Khoá dòng của đơn (do `orders` đặt).
    pub order_line_key: String,
    pub variant_id: Uuid,
    pub sku: String,
    pub name: String,
    pub quantity: i64,
    pub weight_grams: Option<i64>,
    pub length_mm: Option<i64>,
    pub width_mm: Option<i64>,
    pub height_mm: Option<i64>,
}

impl ShipmentItem {
    pub fn new(
        order_line_key: impl Into<String>,
        variant_id: Uuid,
        sku: impl Into<String>,
        name: impl Into<String>,
        quantity: i64,
    ) -> Self {
        ShipmentItem {
            order_line_key: order_line_key.into(),
            variant_id,
            sku: sku.into(),
            name: name.into(),
            quantity,
            weight_grams: None,
            length_mm: None,
            width_mm: None,
            height_mm: None,
        }
    }

    pub fn weight(mut self, grams: Option<i64>) -> Self {
        self.weight_grams = grams;
        self
    }

    pub fn dimensions(
        mut self,
        length_mm: Option<i64>,
        width_mm: Option<i64>,
        height_mm: Option<i64>,
    ) -> Self {
        self.length_mm = length_mm;
        self.width_mm = width_mm;
        self.height_mm = height_mm;
        self
    }
}

/// Common status of a shipment, mapped from each carrier's own statuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = ShippingStatus)]
pub enum ShipmentStatus {
    /// Mới tạo, chưa đặt với hãng.
    Pending,
    /// Đang gọi hãng để đặt.
    Booking,
    /// Đã có mã vận đơn, chờ hãng lấy hàng.
    ReadyToShip,
    PickedUp,
    InTransit,
    OutForDelivery,
    Delivered,
    DeliveryFailed,
    Returning,
    Returned,
    /// Nhận tại cửa hàng: hàng đã sẵn sàng.
    ReadyForPickup,
    /// Nhận tại cửa hàng: người mua đã nhận.
    PickedUpByBuyer,
    Cancelled,
    Lost,
}

impl ShipmentStatus {
    pub const ALL: [ShipmentStatus; 14] = [
        ShipmentStatus::Pending,
        ShipmentStatus::Booking,
        ShipmentStatus::ReadyToShip,
        ShipmentStatus::PickedUp,
        ShipmentStatus::InTransit,
        ShipmentStatus::OutForDelivery,
        ShipmentStatus::Delivered,
        ShipmentStatus::DeliveryFailed,
        ShipmentStatus::Returning,
        ShipmentStatus::Returned,
        ShipmentStatus::ReadyForPickup,
        ShipmentStatus::PickedUpByBuyer,
        ShipmentStatus::Cancelled,
        ShipmentStatus::Lost,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ShipmentStatus::Pending => "pending",
            ShipmentStatus::Booking => "booking",
            ShipmentStatus::ReadyToShip => "ready_to_ship",
            ShipmentStatus::PickedUp => "picked_up",
            ShipmentStatus::InTransit => "in_transit",
            ShipmentStatus::OutForDelivery => "out_for_delivery",
            ShipmentStatus::Delivered => "delivered",
            ShipmentStatus::DeliveryFailed => "delivery_failed",
            ShipmentStatus::Returning => "returning",
            ShipmentStatus::Returned => "returned",
            ShipmentStatus::ReadyForPickup => "ready_for_pickup",
            ShipmentStatus::PickedUpByBuyer => "picked_up_by_buyer",
            ShipmentStatus::Cancelled => "cancelled",
            ShipmentStatus::Lost => "lost",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        ShipmentStatus::ALL.into_iter().find(|status| status.as_str() == raw)
    }

    /// Trạng thái cuối: không đi tiếp được nữa.
    pub fn is_final(self) -> bool {
        matches!(
            self,
            ShipmentStatus::Delivered
                | ShipmentStatus::Returned
                | ShipmentStatus::PickedUpByBuyer
                | ShipmentStatus::Cancelled
                | ShipmentStatus::Lost
        )
    }
}

impl fmt::Display for ShipmentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Payload `after` của mọi event `shipping.shipment_*`. Không mang dữ liệu cá nhân (tên, số
/// điện thoại, địa chỉ người nhận).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ShipmentEvent {
    pub shipment_id: Uuid,
    pub order_id: Uuid,
    pub request_key: String,
    pub direction: Direction,
    pub org_id: OrgId,
    pub status: ShipmentStatus,
    pub tracking_number: Option<String>,
    pub carrier_code: Option<String>,
    /// Với `shipping.cod_collected` / `cod_settled`: số tiền COD của vận đơn.
    pub cod_amount: Option<Money>,
    pub at: Timestamp,
}

pub use carrier::*;

/// Lỗi chung cho bộ nối hãng dùng (`code` là hợp đồng và i18n key, bản dịch ở module `shipping`).
pub mod errors {
    use autoxide_kernel::error::{Error, ErrorKind};

    /// `shipping.carrier_error` (422): hãng từ chối yêu cầu.
    pub fn carrier_error(message: impl ToString) -> Error {
        Error::new(
            ErrorKind::Validation,
            "shipping.carrier_error",
            "The shipping carrier refused: {message}.",
        )
        .with_param("message", message)
    }

    /// `shipping.carrier_unavailable` (503): hãng không trả lời / quá giờ.
    pub fn carrier_unavailable() -> Error {
        Error::unavailable(
            "shipping.carrier_unavailable",
            "The shipping carrier is not answering. Try again later.",
        )
    }

    /// `shipping.carrier_unauthorized` (401): hãng từ chối thông tin đăng nhập.
    pub fn carrier_unauthorized() -> Error {
        Error::new(
            ErrorKind::Unauthenticated,
            "shipping.carrier_unauthorized",
            "The shipping carrier refused the account credentials. Check them and test again.",
        )
    }

    /// `shipping.webhook_signature` (401).
    pub fn webhook_signature() -> Error {
        Error::new(
            ErrorKind::Unauthenticated,
            "shipping.webhook_signature",
            "The webhook signature is not valid.",
        )
    }

    /// `shipping.cancel_too_late` (409).
    pub fn cancel_too_late() -> Error {
        Error::conflict(
            "shipping.cancel_too_late",
            "The carrier has already picked up the parcel; the shipment cannot be cancelled.",
        )
    }
}

mod carrier;
pub mod fake;
