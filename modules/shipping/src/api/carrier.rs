//! Shipping carriers: a connector is its own module (`shipping-<carrier>`) that implements
//! [`ShippingCarrier`] and registers into [`CarrierRegistry`]. Exposed through `autoxide_shipping::api::…`
//! (`pub use` in `api.rs`).

use autoxide_kernel::time::{Date, Timestamp};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Address, Direction, ShipmentStatus};

// ---- Hãng vận chuyển: bộ nối là module riêng ----------------------------------------------------

/// Credentials of a carrier account (decrypted). Not printed by `Debug`, not serialized.
#[derive(Clone, Default)]
pub struct CarrierCredentials(BTreeMap<String, String>);

impl CarrierCredentials {
    pub fn new(values: BTreeMap<String, String>) -> Self {
        CarrierCredentials(values)
    }

    /// Value of field `name` (e.g. `token`, `shop_id`, `webhook_secret`).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// Names of the fields that have values (to report "set", never to return the value).
    pub fn names(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }
}

impl fmt::Debug for CarrierCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CarrierCredentials({} fields, redacted)", self.0.len())
    }
}

/// A tenant's account with a shipping carrier.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CarrierAccount {
    pub id: Uuid,
    pub carrier_code: String,
    pub sandbox: bool,
    /// Non-secret settings (the carrier's default pickup warehouse, service…).
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

/// What the connector can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CarrierCapabilities {
    /// A multi-package waybill.
    pub multi_parcel: bool,
    /// Cash collection on delivery.
    pub cod: bool,
    /// Label printing.
    pub label: bool,
    /// Cancel through the API.
    pub cancel: bool,
    /// COD statement through the API.
    pub statement_api: bool,
    /// Sends status webhooks (otherwise `shipping.poll_tracking` polls periodically).
    pub webhooks: bool,
}

impl CarrierCapabilities {
    /// Every capability (the fake connector).
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

/// The heaviest load `shipping` hands to a carrier, in grams (1,000 tonnes): the chargeable weight of a
/// quote, one parcel, and all parcels of a shipment together. Inputs over it are refused with
/// `shipping.load_too_large` (422), so a connector adding up the parcels of one request never overflows
/// `i64`; a connector still checks its own sums and the carrier's own limits.
pub const MAX_PARCEL_WEIGHT_GRAMS: i64 = 1_000_000_000;

/// The longest side of an item or parcel `shipping` accepts, in mm (as `catalog`: 10,000,000 mm).
pub const MAX_SIDE_MM: i64 = 10_000_000;

/// A package sent to the carrier.
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

/// Asks the carrier for a fee.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CarrierQuoteRequest {
    /// Pickup location (the address of the shipping branch), if known.
    pub origin: Option<Address>,
    pub destination: Address,
    pub parcels: Vec<CarrierParcel>,
    pub declared_value: Money,
    pub cod_amount: Option<Money>,
    /// Carrier service chosen by the method; `None`: every service.
    pub service_code: Option<String>,
    /// Recipient (name, phone number) when the cart already has one — a carrier that requires a recipient
    /// when quoting need not be sent a placeholder value. `None`: not known.
    pub recipient: Option<(String, String)>,
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
            recipient: None,
        }
    }

    /// Recipient (name, phone number), if known.
    pub fn recipient(mut self, recipient: Option<(String, String)>) -> Self {
        self.recipient = recipient;
        self
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

/// A fee quoted by the carrier.
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

/// Creates a waybill with the carrier.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CarrierShipmentRequest {
    /// Autoxide's waybill ID: if the carrier receives the same code again it does not create a second order
    /// (or the connector first looks up [`ShippingCarrier::find_by_reference`] itself).
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

/// The waybill the carrier created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CarrierShipment {
    pub client_reference: String,
    pub tracking_number: String,
    /// Tracking code of each package (in send order), when the carrier issues them separately.
    pub parcel_tracking_numbers: Vec<String>,
    pub status: ShipmentStatus,
    /// Fee the carrier charges.
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

/// Label file.
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

/// A status update from the carrier (webhook or periodic polling).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CarrierUpdate {
    /// Dedupe key (if the carrier has none, the connector hashes the body + time).
    pub event_id: String,
    pub tracking_number: String,
    pub status: ShipmentStatus,
    /// The carrier's verbatim status.
    pub carrier_status: String,
    /// Converted to UTC (Vietnamese carriers often send local time without a time zone).
    pub occurred_at: Timestamp,
    /// The carrier reports it collected (for reconciliation only; the real COD amount comes from the delivery request).
    pub cod_collected: Option<Money>,
    pub fee: Option<Money>,
    pub note: String,
}

impl CarrierUpdate {
    pub fn new(
        event_id: impl Into<String>,
        tracking_number: impl Into<String>,
        status: ShipmentStatus,
        occurred_at: Timestamp,
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

/// The carrier's COD statement for a period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CodStatement {
    /// The carrier's statement code (guards against duplicate imports).
    pub reference: String,
    pub period_from: Date,
    pub period_to: Date,
    pub lines: Vec<CodStatementLine>,
}

impl CodStatement {
    pub fn new(
        reference: impl Into<String>,
        period_from: Date,
        period_to: Date,
        lines: Vec<CodStatementLine>,
    ) -> Self {
        CodStatement { reference: reference.into(), period_from, period_to, lines }
    }
}

/// A line of a COD statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CodStatementLine {
    pub tracking_number: String,
    /// Money the carrier collected.
    pub cod: Money,
    /// Fee the carrier deducted.
    pub fee: Money,
}

impl CodStatementLine {
    pub fn new(tracking_number: impl Into<String>, cod: Money, fee: Money) -> Self {
        CodStatementLine { tracking_number: tracking_number.into(), cod, fee }
    }
}

/// A shipping carrier. A real connector (GHN, GHTK…) is its own module `shipping-<carrier>`, registered AFTER
/// `shipping` and calling [`CarrierRegistry::register`] in `Module::start`. Every function makes network
/// calls; the `shipping` module never calls them inside a write transaction.
#[async_trait::async_trait]
pub trait ShippingCarrier: Send + Sync {
    /// Stable code (`ghn`, `ghtk`, `fake`…): appears in the webhook path and on the carrier account.
    fn code(&self) -> &'static str;

    fn capabilities(&self) -> CarrierCapabilities;

    /// Checks the credentials (the "Test connection" button); wrong → error `Unauthenticated`.
    async fn check(&self, account: &CarrierAccount) -> Result<(), Error>;

    async fn quote(
        &self,
        account: &CarrierAccount,
        request: &CarrierQuoteRequest,
    ) -> Result<Vec<CarrierRate>, Error>;

    /// Creates a waybill. `request.client_reference` = Autoxide's waybill ID: calling again with the same
    /// code does not create a second order.
    async fn create_shipment(
        &self,
        account: &CarrierAccount,
        request: &CarrierShipmentRequest,
    ) -> Result<CarrierShipment, Error>;

    /// Finds a waybill already created by `client_reference` (when the previous create lost its connection midway).
    async fn find_by_reference(
        &self,
        account: &CarrierAccount,
        client_reference: &str,
    ) -> Result<Option<CarrierShipment>, Error>;

    /// Cancels. The carrier already picked up the goods → `shipping.cancel_too_late`.
    async fn cancel(&self, account: &CarrierAccount, tracking_number: &str) -> Result<(), Error>;

    async fn label(&self, account: &CarrierAccount, tracking_number: &str) -> Result<Label, Error>;

    /// Full journey, oldest first.
    async fn track(
        &self,
        account: &CarrierAccount,
        tracking_number: &str,
    ) -> Result<Vec<CarrierUpdate>, Error>;

    /// Verifies the webhook signature over the raw body (constant-time comparison) and only then parses the
    /// body into updates; a wrong signature → `shipping.webhook_signature` (401), the body is not read.
    fn parse_webhook(
        &self,
        account: &CarrierAccount,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<Vec<CarrierUpdate>, Error>;

    /// COD statements for the days `[from, to]`, when the carrier has an API (otherwise import a CSV).
    async fn cod_statement(
        &self,
        _account: &CarrierAccount,
        _from: Date,
        _to: Date,
    ) -> Result<Option<CodStatement>, Error> {
        Ok(None)
    }
}

/// Registry of shipping carriers (published by the `shipping` module): a connector calls `register` in `Module::start`.
pub trait CarrierRegistry: Send + Sync {
    /// Registers a connector; a duplicate `code` → error.
    fn register(&self, carrier: Arc<dyn ShippingCarrier>) -> Result<(), Error>;
    fn get(&self, code: &str) -> Option<Arc<dyn ShippingCarrier>>;
    /// Codes of the registered carriers, in alphabetical order.
    fn codes(&self) -> Vec<&'static str>;
}
