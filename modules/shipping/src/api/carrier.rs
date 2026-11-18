//! Hãng vận chuyển: bộ nối là module riêng (`shipping-<hãng>`) cài [`ShippingCarrier`] và đăng ký
//! vào [`CarrierRegistry`]. Mở ra ngoài qua `autoxide_shipping::api::…` (`pub use` ở `api.rs`).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Address, Direction, ShipmentStatus};

// ---- Hãng vận chuyển: bộ nối là module riêng ----------------------------------------------------

/// Thông tin đăng nhập của tài khoản hãng (đã giải mã). Không in ra `Debug`, không serialize.
#[derive(Clone, Default)]
pub struct CarrierCredentials(BTreeMap<String, String>);

impl CarrierCredentials {
    pub fn new(values: BTreeMap<String, String>) -> Self {
        CarrierCredentials(values)
    }

    /// Giá trị của trường `name` (vd `token`, `shop_id`, `webhook_secret`).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// Tên các trường có giá trị (để báo "đã đặt", không bao giờ trả giá trị).
    pub fn names(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }
}

impl fmt::Debug for CarrierCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CarrierCredentials({} fields, redacted)", self.0.len())
    }
}

/// Một tài khoản của khách ở một hãng vận chuyển.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CarrierAccount {
    pub id: Uuid,
    pub carrier_code: String,
    pub sandbox: bool,
    /// Cài đặt không bí mật (kho lấy hàng mặc định của hãng, dịch vụ…).
    pub settings: serde_json::Value,
    pub credentials: CarrierCredentials,
}

impl CarrierAccount {
    pub fn new(
        id: Uuid,
        carrier_code: impl Into<String>,
        sandbox: bool,
        settings: serde_json::Value,
        credentials: CarrierCredentials,
    ) -> Self {
        CarrierAccount { id, carrier_code: carrier_code.into(), sandbox, settings, credentials }
    }
}

/// Bộ nối làm được gì.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CarrierCapabilities {
    /// Một vận đơn nhiều kiện.
    pub multi_parcel: bool,
    /// Thu hộ khi giao.
    pub cod: bool,
    /// In nhãn.
    pub label: bool,
    /// Huỷ qua API.
    pub cancel: bool,
    /// Bảng kê COD qua API.
    pub statement_api: bool,
    /// Gửi webhook trạng thái (không thì `shipping.poll_tracking` hỏi định kỳ).
    pub webhooks: bool,
}

impl CarrierCapabilities {
    /// Mọi khả năng (bộ giả).
    pub fn all() -> Self {
        CarrierCapabilities {
            multi_parcel: true,
            cod: true,
            label: true,
            cancel: true,
            statement_api: true,
            webhooks: true,
        }
    }
}

/// Một kiện gửi hãng.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CarrierParcel {
    pub weight_grams: i64,
    pub length_mm: Option<i64>,
    pub width_mm: Option<i64>,
    pub height_mm: Option<i64>,
}

impl CarrierParcel {
    pub fn new(weight_grams: i64) -> Self {
        CarrierParcel { weight_grams, length_mm: None, width_mm: None, height_mm: None }
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

/// Hỏi phí của hãng.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CarrierQuoteRequest {
    /// Nơi lấy hàng (địa chỉ của nút xuất), nếu biết.
    pub origin: Option<Address>,
    pub destination: Address,
    pub parcels: Vec<CarrierParcel>,
    pub declared_value: Money,
    pub cod_amount: Option<Money>,
    /// Dịch vụ của hãng phương thức chọn; `None`: mọi dịch vụ.
    pub service_code: Option<String>,
}

impl CarrierQuoteRequest {
    pub fn new(destination: Address, parcels: Vec<CarrierParcel>, declared_value: Money) -> Self {
        CarrierQuoteRequest {
            origin: None,
            destination,
            parcels,
            declared_value,
            cod_amount: None,
            service_code: None,
        }
    }

    pub fn origin(mut self, origin: Option<Address>) -> Self {
        self.origin = origin;
        self
    }

    pub fn service(mut self, code: Option<String>) -> Self {
        self.service_code = code;
        self
    }

    pub fn cod(mut self, amount: Option<Money>) -> Self {
        self.cod_amount = amount;
        self
    }
}

/// Một mức phí hãng báo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CarrierRate {
    pub service_code: String,
    pub amount: Money,
    pub estimated_days: Option<(u16, u16)>,
}

impl CarrierRate {
    pub fn new(service_code: impl Into<String>, amount: Money) -> Self {
        CarrierRate { service_code: service_code.into(), amount, estimated_days: None }
    }

    pub fn days(mut self, min: u16, max: u16) -> Self {
        self.estimated_days = Some((min, max));
        self
    }
}

/// Tạo vận đơn ở hãng.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CarrierShipmentRequest {
    /// ID vận đơn của Autoxide: hãng nhận lại cùng mã thì không tạo đơn thứ hai (hoặc bộ nối tự
    /// tra [`ShippingCarrier::find_by_reference`] trước).
    pub client_reference: String,
    pub service_code: Option<String>,
    pub direction: Direction,
    pub origin: Option<Address>,
    pub recipient_name: String,
    pub recipient_phone: String,
    pub destination: Address,
    pub parcels: Vec<CarrierParcel>,
    pub declared_value: Money,
    pub cod_amount: Option<Money>,
    pub note: String,
}

impl CarrierShipmentRequest {
    pub fn new(
        client_reference: impl Into<String>,
        recipient: (String, String),
        destination: Address,
        parcels: Vec<CarrierParcel>,
        declared_value: Money,
    ) -> Self {
        CarrierShipmentRequest {
            client_reference: client_reference.into(),
            service_code: None,
            direction: Direction::Outbound,
            origin: None,
            recipient_name: recipient.0,
            recipient_phone: recipient.1,
            destination,
            parcels,
            declared_value,
            cod_amount: None,
            note: String::new(),
        }
    }

    pub fn service(mut self, code: Option<String>) -> Self {
        self.service_code = code;
        self
    }

    pub fn direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    pub fn origin(mut self, origin: Option<Address>) -> Self {
        self.origin = origin;
        self
    }

    pub fn cod(mut self, amount: Option<Money>) -> Self {
        self.cod_amount = amount;
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }
}

/// Vận đơn hãng đã tạo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CarrierShipment {
    pub client_reference: String,
    pub tracking_number: String,
    /// Mã vận đơn của từng kiện (theo thứ tự gửi), khi hãng cấp riêng.
    pub parcel_tracking_numbers: Vec<String>,
    pub status: ShipmentStatus,
    /// Phí hãng tính.
    pub fee: Option<Money>,
}

impl CarrierShipment {
    pub fn new(client_reference: impl Into<String>, tracking_number: impl Into<String>) -> Self {
        CarrierShipment {
            client_reference: client_reference.into(),
            tracking_number: tracking_number.into(),
            parcel_tracking_numbers: Vec::new(),
            status: ShipmentStatus::ReadyToShip,
            fee: None,
        }
    }

    pub fn fee(mut self, fee: Option<Money>) -> Self {
        self.fee = fee;
        self
    }

    pub fn parcels(mut self, tracking_numbers: Vec<String>) -> Self {
        self.parcel_tracking_numbers = tracking_numbers;
        self
    }
}

/// File nhãn.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Label {
    /// `application/pdf`, `image/png`…
    pub content_type: String,
    pub bytes: Vec<u8>,
}

impl Label {
    pub fn new(content_type: impl Into<String>, bytes: Vec<u8>) -> Self {
        Label { content_type: content_type.into(), bytes }
    }
}

/// Một cập nhật trạng thái từ hãng (webhook hay hỏi định kỳ).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CarrierUpdate {
    /// Khoá chống trùng (hãng không có thì bộ nối băm thân + thời gian).
    pub event_id: String,
    pub tracking_number: String,
    pub status: ShipmentStatus,
    /// Nguyên văn trạng thái của hãng.
    pub carrier_status: String,
    /// Đã đổi sang UTC (hãng Việt Nam hay gửi giờ địa phương không kèm múi giờ).
    pub occurred_at: DateTime<Utc>,
    /// Hãng báo đã thu (chỉ để đối chiếu; số tiền COD thật lấy từ yêu cầu giao).
    pub cod_collected: Option<Money>,
    pub fee: Option<Money>,
    pub note: String,
}

impl CarrierUpdate {
    pub fn new(
        event_id: impl Into<String>,
        tracking_number: impl Into<String>,
        status: ShipmentStatus,
        occurred_at: DateTime<Utc>,
    ) -> Self {
        CarrierUpdate {
            event_id: event_id.into(),
            tracking_number: tracking_number.into(),
            status,
            carrier_status: status.as_str().to_owned(),
            occurred_at,
            cod_collected: None,
            fee: None,
            note: String::new(),
        }
    }

    pub fn carrier_status(mut self, raw: impl Into<String>) -> Self {
        self.carrier_status = raw.into();
        self
    }

    pub fn cod_collected(mut self, amount: Option<Money>) -> Self {
        self.cod_collected = amount;
        self
    }

    pub fn fee(mut self, fee: Option<Money>) -> Self {
        self.fee = fee;
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }
}

/// Bảng kê COD của hãng cho một kỳ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CodStatement {
    /// Mã bảng kê của hãng (chống nhập trùng).
    pub reference: String,
    pub period_from: NaiveDate,
    pub period_to: NaiveDate,
    pub lines: Vec<CodStatementLine>,
}

impl CodStatement {
    pub fn new(
        reference: impl Into<String>,
        period_from: NaiveDate,
        period_to: NaiveDate,
        lines: Vec<CodStatementLine>,
    ) -> Self {
        CodStatement { reference: reference.into(), period_from, period_to, lines }
    }
}

/// Một dòng của bảng kê COD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CodStatementLine {
    pub tracking_number: String,
    /// Tiền hãng đã thu.
    pub cod: Money,
    /// Phí hãng trừ.
    pub fee: Money,
}

impl CodStatementLine {
    pub fn new(tracking_number: impl Into<String>, cod: Money, fee: Money) -> Self {
        CodStatementLine { tracking_number: tracking_number.into(), cod, fee }
    }
}

/// Một hãng vận chuyển. Bộ nối thật (GHN, GHTK…) là module riêng `shipping-<hãng>`, đăng ký SAU
/// `shipping` và gọi [`CarrierRegistry::register`] trong `Module::start`. Mọi hàm gọi mạng; module
/// `shipping` không bao giờ gọi chúng trong transaction ghi.
#[async_trait::async_trait]
pub trait ShippingCarrier: Send + Sync {
    /// Mã ổn định (`ghn`, `ghtk`, `fake`…): nằm trong đường dẫn webhook và trên tài khoản hãng.
    fn code(&self) -> &'static str;

    fn capabilities(&self) -> CarrierCapabilities;

    /// Kiểm thông tin đăng nhập (nút "Kiểm tra kết nối"); sai → lỗi `Unauthenticated`.
    async fn check(&self, account: &CarrierAccount) -> Result<(), Error>;

    async fn quote(
        &self,
        account: &CarrierAccount,
        request: &CarrierQuoteRequest,
    ) -> Result<Vec<CarrierRate>, Error>;

    /// Tạo vận đơn. `request.client_reference` = ID vận đơn của Autoxide: gọi lại cùng mã không
    /// tạo đơn thứ hai.
    async fn create_shipment(
        &self,
        account: &CarrierAccount,
        request: &CarrierShipmentRequest,
    ) -> Result<CarrierShipment, Error>;

    /// Tìm vận đơn đã tạo theo `client_reference` (khi lần tạo trước mất kết nối giữa chừng).
    async fn find_by_reference(
        &self,
        account: &CarrierAccount,
        client_reference: &str,
    ) -> Result<Option<CarrierShipment>, Error>;

    /// Huỷ. Hãng đã lấy hàng → `shipping.cancel_too_late`.
    async fn cancel(&self, account: &CarrierAccount, tracking_number: &str) -> Result<(), Error>;

    async fn label(&self, account: &CarrierAccount, tracking_number: &str) -> Result<Label, Error>;

    /// Hành trình đầy đủ, cũ trước.
    async fn track(
        &self,
        account: &CarrierAccount,
        tracking_number: &str,
    ) -> Result<Vec<CarrierUpdate>, Error>;

    /// Kiểm chữ ký webhook trên thân thô (so sánh thời gian hằng) rồi mới đọc thân thành các cập
    /// nhật; chữ ký sai → `shipping.webhook_signature` (401), không đọc thân.
    fn parse_webhook(
        &self,
        account: &CarrierAccount,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<Vec<CarrierUpdate>, Error>;

    /// Bảng kê COD các ngày `[from, to]`, khi hãng có API (không có thì nhập CSV).
    async fn cod_statement(
        &self,
        _account: &CarrierAccount,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> Result<Option<CodStatement>, Error> {
        Ok(None)
    }
}

/// Sổ hãng vận chuyển (module `shipping` công bố): bộ nối gọi `register` trong `Module::start`.
pub trait CarrierRegistry: Send + Sync {
    /// Đăng ký bộ nối; trùng `code` → lỗi.
    fn register(&self, carrier: Arc<dyn ShippingCarrier>) -> Result<(), Error>;
    fn get(&self, code: &str) -> Option<Arc<dyn ShippingCarrier>>;
    /// Mã các hãng đã đăng ký, theo thứ tự chữ cái.
    fn codes(&self) -> Vec<&'static str>;
}
