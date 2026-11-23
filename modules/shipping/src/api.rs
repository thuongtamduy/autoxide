//! Contract of the `shipping` module for other modules (`orders`, the carrier connectors
//! `shipping-<carrier>`, `support`…): only what is here is used from outside
//! (`autoxide_shipping::api::…`). Other modules get the traits at startup:
//!
//! ```text
//! let rates = context.require::<dyn autoxide_shipping::api::ShippingRates>()?;
//! let book = context.require::<dyn autoxide_shipping::api::AddressBook>()?;
//! let carriers = context.require::<dyn autoxide_shipping::api::CarrierRegistry>()?;
//! ```
//!
//! That module must register AFTER `shipping` in `crates/app/src/modules.rs`. No module reads
//! the `shipping_…` tables (architecture rule 1). This file does not reference the module's internals
//! (`crate::…`), so the fake [`fake::FakeCarrier`] can be used from the testkit and step 45.
//!
//! `shipping` **does not depend on `orders`**: it receives shipping requests through events (names in
//! [`events`]), with payload `after.shipment` being a [`ShipmentRequest`]; it reports status through its own
//! events.

use autoxide_kernel::time::{Date, Timestamp};
use std::fmt;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use autoxide_address::api::Address;

/// Event names: `shipping` listens to those of `orders` and emits those for other modules.
pub mod events {
    /// `orders` asks to ship part of / a whole order: payload `{"after": {"shipment": ShipmentRequest}}`.
    pub const ORDERS_FULFILLMENT_REQUESTED: &str = "orders.fulfillment_requested";
    /// `orders` asks to pick up returned goods from the buyer (`direction = return`), payload as above.
    pub const ORDERS_RETURN_PICKUP_REQUESTED: &str = "orders.return_pickup_requested";
    /// `orders` cancels an order: payload `{"after": {"order_id": "<uuid>"}}` — cancels shipments not yet picked up.
    pub const ORDERS_ORDER_CANCELLED: &str = "orders.order_cancelled";

    /// A shipment was just created (`pending`). Payload `{"after": ShipmentEvent}`.
    pub const SHIPMENT_CREATED: &str = "shipping.shipment_created";
    /// Booked with the carrier, with a tracking number.
    pub const SHIPMENT_BOOKED: &str = "shipping.shipment_booked";
    /// Every status change (including the dedicated events below).
    pub const SHIPMENT_STATUS_CHANGED: &str = "shipping.shipment_status_changed";
    pub const SHIPMENT_DELIVERED: &str = "shipping.shipment_delivered";
    pub const SHIPMENT_DELIVERY_FAILED: &str = "shipping.shipment_delivery_failed";
    pub const SHIPMENT_RETURNED: &str = "shipping.shipment_returned";
    pub const SHIPMENT_CANCELLED: &str = "shipping.shipment_cancelled";
    /// Booking with the carrier failed for good (the shipment returns to `pending`).
    pub const BOOKING_FAILED: &str = "shipping.booking_failed";
    /// The carrier collected the COD money (delivered) — `orders` records the order as paid.
    pub const COD_COLLECTED: &str = "shipping.cod_collected";
    /// The carrier's statement matches: the COD money has reached the seller / the collecting partner (D-216).
    pub const COD_SETTLED: &str = "shipping.cod_settled";
    /// The statement disagrees with a shipment (or a shipment is overdue and not yet in a statement).
    pub const COD_MISMATCH: &str = "shipping.cod_mismatch";
    /// A COD discrepancy was resolved (reason code + note).
    pub const COD_DISCREPANCY_RESOLVED: &str = "shipping.cod_discrepancy_resolved";
    /// A carrier account was added / changed / checked (carries no credentials).
    pub const CARRIER_ACCOUNT_CHANGED: &str = "shipping.carrier_account_changed";
}

// ---- Sổ địa chỉ người mua -----------------------------------------------------------------------

/// An address in a buyer's address book.
///
/// A Vietnamese address: `address` follows the two-level administrative units (module `address`). A foreign
/// address is generic: `address.province_code` / `province_name` is the region (state, province…),
/// `address.commune_name` is the city, `commune_code` is empty, with `postal_code`.
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

/// A buyer's address book, for `orders` (cart, checkout).
#[async_trait::async_trait]
pub trait AddressBook: Send + Sync {
    /// Address `id` of buyer `buyer_id`; another person's or missing →
    /// `shipping.address_not_found`.
    async fn get(
        &self,
        tenant: &TenantContext,
        buyer_id: Uuid,
        id: Uuid,
    ) -> Result<BuyerAddress, Error>;
    /// The buyer's default address, if any.
    async fn default_for(
        &self,
        tenant: &TenantContext,
        buyer_id: Uuid,
    ) -> Result<Option<BuyerAddress>, Error>;
}

// ---- Báo giá, cho `orders` ----------------------------------------------------------------------

/// A quote request: the ways to ship a cart from store `org_id` to `destination`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct QuoteRequest {
    /// The selling store (methods inherit down the organization tree).
    pub org_id: OrgId,
    /// The dispatch warehouse if already known (from an `inventory` hold).
    pub origin_org_id: Option<OrgId>,
    /// `None`: only the store-pickup method is returned.
    pub destination: Option<Address>,
    pub items: Vec<QuoteItem>,
    /// Order value after discounts, to evaluate free shipping.
    pub order_value: Money,
    /// Time of calculation (estimated delivery date, cutoff time).
    pub at: Timestamp,
    /// Language of the method names.
    pub locale: String,
    /// The recipient (name, phone number) when already known, passed to the carrier when quoting the fee.
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

    /// The recipient (name, phone number), if already known.
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

/// A cart line, with figures taken from the `VariantSnapshot` of `catalog` (`shipping` does not call
/// `catalog`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct QuoteItem {
    pub variant_id: Uuid,
    pub quantity: i64,
    pub requires_shipping: bool,
    /// Weight of one item, in grams.
    pub weight_grams: Option<i64>,
    pub length_mm: Option<i64>,
    pub width_mm: Option<i64>,
    pub height_mm: Option<i64>,
    /// Item code, so `shipping.weight_missing` names the right item.
    pub sku: String,
}

impl QuoteItem {
    /// An item that must be shipped but has no weight / dimensions yet.
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

    /// Length × width × height, in mm.
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
    /// A fee table the tenant sets itself (zone × weight tier).
    TableRate,
    /// A fee quoted by the carrier.
    Carrier,
    /// Pick up in store.
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

/// Estimated delivery date: an absolute date in the time zone of the delivery calendar (D-201).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Eta {
    pub earliest_date: Date,
    pub latest_date: Date,
    /// If ordered now it makes today's run (the cutoff time is still within the day), otherwise `None`.
    pub order_by: Option<Timestamp>,
    /// IANA time zone of the two dates above, e.g. `Asia/Ho_Chi_Minh`.
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

/// A valid way to ship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ShippingOption {
    pub method_id: Uuid,
    pub method_code: String,
    pub name: String,
    pub kind: MethodKind,
    /// The fee the buyer pays (0 when free), including the VAT on the shipping fee.
    pub amount: Money,
    pub free_shipping_applied: bool,
    /// The method's free-shipping threshold (so `orders` can say "buy X more for free shipping").
    pub free_over: Option<Money>,
    /// The tax group of the shipping fee in the tax engine (step 36), e.g. `vat.standard`; `orders` records
    /// the shipping fee's tax line on the invoice under this group (D-208…D-218).
    pub tax_category: String,
    pub carrier_code: Option<String>,
    pub service_code: Option<String>,
    /// The store that hands over the goods, for `Pickup`.
    pub pickup_org_id: Option<OrgId>,
    pub chargeable_weight_grams: i64,
    /// Delivery days (minimum, maximum) as the method declares.
    pub estimated_days: Option<(u16, u16)>,
    pub eta: Option<Eta>,
    /// Store pickup: when the goods are ready (preparation + opening hours).
    pub ready_at: Option<Timestamp>,
    /// Store pickup: the number of days the goods are held awaiting pickup.
    pub hold_days: Option<u16>,
    /// Fingerprint (method, amount, destination) — `orders` sends it back when placing the order; a mismatch →
    /// `shipping.option_changed`.
    pub option_key: String,
}

/// A quote.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Quote {
    /// Cheapest first. Empty when the cart has no item to ship.
    pub options: Vec<ShippingOption>,
    /// Methods unusable for this cart / destination, with a reason code.
    pub unavailable: Vec<UnavailableMethod>,
}

/// A method that is unusable and why (`shipping.weight_missing`, `shipping.carrier_unavailable`…).
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

/// Quotes shipping fees (published by the `shipping` module).
#[async_trait::async_trait]
pub trait ShippingRates: Send + Sync {
    /// The valid ways to ship, cheapest first. A carrier error / timeout: that method goes into `unavailable`,
    /// without breaking the whole quote. Do not call inside a write transaction (it may call the carrier).
    async fn quote(&self, tenant: &TenantContext, request: &QuoteRequest) -> Result<Quote, Error>;

    /// Rechecks a shipping option at order time: recomputes the quote and returns exactly the method with
    /// `option_key`; a changed fee or destination → `shipping.option_changed` (409).
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
    /// Ship to the buyer.
    Outbound,
    /// Pick up returned goods from the buyer back to the warehouse.
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

/// A request to ship (or to pick up returned goods). `(order_id, request_key)` is the deduplication key:
/// resending the same content creates no second shipment; the same key with different content is rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ShipmentRequest {
    pub order_id: Uuid,
    pub order_number: String,
    /// An order may ship several times (partial shipments): one key per time.
    pub request_key: String,
    pub direction: Direction,
    /// The selling store.
    pub org_id: OrgId,
    /// The dispatch warehouse / the warehouse receiving returned goods.
    pub origin_org_id: OrgId,
    pub method_id: Uuid,
    pub option_key: Option<String>,
    /// The buyer (the order owner) — so automatic replies about the journey go only to the order owner (D-230).
    pub buyer_id: Option<Uuid>,
    pub recipient_name: String,
    pub recipient_phone: String,
    pub address: Address,
    pub items: Vec<ShipmentItem>,
    pub declared_value: Money,
    /// The money the carrier collects on delivery; `None`: no collection on behalf.
    pub cod_amount: Option<Money>,
    /// The shipping fee quoted to the buyer (from the quote).
    pub quoted_fee: Option<Money>,
    pub note: String,
}

impl ShipmentRequest {
    /// An outbound shipping request (`Outbound`), with no COD, the quoted fee, a note.
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

/// A line of goods in a shipping request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ShipmentItem {
    /// Key of the order line (set by `orders`).
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
    /// Just created, not yet booked with the carrier.
    Pending,
    /// Calling the carrier to book.
    Booking,
    /// Has a tracking number, awaiting pickup by the carrier.
    ReadyToShip,
    PickedUp,
    InTransit,
    OutForDelivery,
    Delivered,
    DeliveryFailed,
    Returning,
    Returned,
    /// Store pickup: the goods are ready.
    ReadyForPickup,
    /// Store pickup: the buyer has collected them.
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

    /// Final state: cannot move on.
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

/// The `after` payload of every `shipping.shipment_*` event. Carries no personal data (the recipient's name,
/// phone number, address).
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
    /// For `shipping.cod_collected` / `cod_settled`: the COD amount of the shipment.
    pub cod_amount: Option<Money>,
    pub at: Timestamp,
}

pub use carrier::*;

/// A shared error for the carrier connectors to use (`code` is the contract and i18n key, translations in the `shipping` module).
pub mod errors {
    use autoxide_kernel::error::{Error, ErrorKind};

    /// `shipping.carrier_error` (422): the carrier rejected the request.
    pub fn carrier_error(message: impl ToString) -> Error {
        Error::new(
            ErrorKind::Validation,
            "shipping.carrier_error",
            "The shipping carrier refused: {message}.",
        )
        .with_param("message", message)
    }

    /// `shipping.carrier_unavailable` (503): the carrier did not respond / timed out.
    pub fn carrier_unavailable() -> Error {
        Error::unavailable(
            "shipping.carrier_unavailable",
            "The shipping carrier is not answering. Try again later.",
        )
    }

    /// `shipping.carrier_unauthorized` (401): the carrier rejected the credentials.
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
