//! Contract of the `orders` module for other modules (`marketplace` step 46, `customers` 47, `channels`
//! 48, `affiliate` 49, `auth`, `support`, `reporting`…): only what is here is used from outside
//! (`autoxide_orders::api::…`). This file does not reference the module's internals.
//!
//! Two ways to connect to `orders`:
//!
//! 1. A module registered AFTER `orders` gets the trait at startup:
//!
//!    ```text
//!    let orders = context.require::<dyn autoxide_orders::api::Orders>()?;
//!    ```
//!
//! 2. A module registered AFTER `orders` that `orders` needs to call (the marketplace seller, customer
//!    groups, points / gift cards) PLUGS itself into [`OrderExtensions`] in its own `Module::start` —
//!    `orders` starts first so it cannot `require` a trait of a later module (D-198,
//!    D-20261118-step45-orders-api).
//!
//! The order ID is a `Uuid` (UUIDv7), like `order_id` in the contracts of `inventory`, `promotions`,
//! `shipping`. The shared hold reference code: [`hold_reference`] (`orders:<order_id>`).

use autoxide_kernel::time::Timestamp;
use std::fmt;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, TenantContext, TenantId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Names of the events `orders` emits (payload `{"before": …, "after": …}`; `after` always has `order_id`,
/// `tenant_id`, `org_id`, `channel` for an event about one order). No event carries a secret, token or
/// `Cart-Token`. An event about one order (placed, confirmed, status change…) also has `buyer_key` (a
/// per-tenant HMAC contact key of the buyer's phone number, not the number), `placed_at` and `goods_paid`
/// (what the buyer paid for goods: after discounts, including tax, excluding shipping fees — the basis for
/// points in step 47, D-226).
pub mod events {
    /// An order just created (`pending_payment` or `confirmed`).
    pub const ORDER_PLACED: &str = "orders.order_placed";
    /// An order confirmed (fully paid, COD-eligible, POS, channel): `inventory`, `promotions`, `Tender`
    /// confirm their holds. `after`: `order_id`, `hold_reference`, lines, warehouse.
    pub const ORDER_CONFIRMED: &str = "orders.order_confirmed";
    /// Every change of `status`.
    pub const ORDER_STATUS_CHANGED: &str = "orders.order_status_changed";
    /// An order cancelled: holders release their holds. `after`: `order_id`, `hold_reference`, `cause`.
    pub const ORDER_CANCELLED: &str = "orders.order_cancelled";
    /// An order fully delivered. `after.delivered_lines` lists every line with units left after cancellations:
    /// `line_key`, `variant_id`, `quantity` (units no shipment carried; `orders.fulfillment_delivered` covers the
    /// others), `delivered_quantity` (every unit of the line, now with the buyer) and `serials`. `inventory`
    /// issues each line up to `delivered_quantity` (`sale` lines).
    pub const ORDER_DELIVERED: &str = "orders.order_delivered";
    /// One shipment of an order reached the buyer: `after.order_id`, `after.fulfillment_id`, `after.lines`
    /// (`line_key`, `variant_id`, `quantity` carried by this shipment, `delivered_quantity` = units of the line
    /// delivered so far by every shipment, `serials` recorded for the line). `inventory` issues each line up to
    /// `delivered_quantity` (`sale` lines), so units already issued by hand are not issued again.
    pub const FULFILLMENT_DELIVERED: &str = "orders.fulfillment_delivered";
    /// Serial numbers were recorded for a line whose goods already reached the buyer: `after.order_id`,
    /// `after.line_id`, `after.line_key`, `after.delivered_quantity`, `after.serials` (every serial of the
    /// line). `inventory` issues the serial-tracked units it still held for want of serial numbers.
    pub const LINE_SERIALS_RECORDED: &str = "orders.line_serials_recorded";
    /// An order completed (return window over, no longer deferred): the trigger for commission, points, seller payout.
    pub const ORDER_COMPLETED: &str = "orders.order_completed";
    /// A request to ship a parcel: `after.shipment` is the `ShipmentRequest` of `shipping`.
    pub const FULFILLMENT_REQUESTED: &str = "orders.fulfillment_requested";
    /// A return that needs a carrier pickup: `after.shipment` is a `ShipmentRequest` (`direction = return`).
    pub const RETURN_PICKUP_REQUESTED: &str = "orders.return_pickup_requested";
    /// An order imported from an external channel that is short of stock.
    pub const STOCK_SHORTAGE: &str = "orders.stock_shortage";
    /// A payment received for an order is recorded.
    pub const PAYMENT_RECORDED: &str = "orders.payment_recorded";
    /// Money refunded (lines and refunded amount): the `Tender` of step 47 listens to refund points / balance.
    pub const REFUND_COMPLETED: &str = "orders.refund_completed";
    /// The buyer requests a return (with `photo_count`, no photo IDs).
    pub const RETURN_REQUESTED: &str = "orders.return_requested";
    /// Returned goods are back in the warehouse: `inventory` restocks. `after`: `return_id`, `order_id`,
    /// `warehouse_org_id`, `lines[{variant_id, quantity, sellable, lot_code?, serials[]}]`.
    pub const RETURN_RECEIVED: &str = "orders.return_received";
    /// A return request changes status (approved, rejected, goods received, refunded, closed).
    pub const RETURN_STATUS_CHANGED: &str = "orders.return_status_changed";
    /// A store-pickup order is ready: `after.pickup_code` (a 6-digit pickup code sent to the buyer),
    /// `hold_until`, `pickup_org_id`.
    pub const PICKUP_READY: &str = "orders.pickup_ready";
    /// Less than 24 hours until the pickup hold expires (sent once).
    pub const PICKUP_REMINDER: &str = "orders.pickup_reminder";
    /// The pickup hold expired (`after.action`: `cancel_refund` or `keep_waiting`).
    pub const PICKUP_EXPIRED: &str = "orders.pickup_expired";
    /// The delivery date of a pre-order item slipped later than the promised date: the buyer may cancel for
    /// free (`after.promised_on`, `after.expected_on`, `after.line_id`).
    pub const PREORDER_DELAYED: &str = "orders.preorder_delayed";
    /// The not-yet-in-stock pre-order part of a cancelled order (`after.reason`: `preorder_stopped`,
    /// `buyer_cancelled`; `after.lines`, `after.amount` refunded).
    pub const PREORDER_CANCELLED: &str = "orders.preorder_cancelled";
    /// An e-invoice has been attached to the order.
    pub const INVOICE_LINKED: &str = "orders.invoice_linked";
    /// An abandoned cart (buyer, cart summary) — for step 47.
    pub const CART_ABANDONED: &str = "orders.cart_abandoned";
    /// A review is posted.
    pub const REVIEW_PUBLISHED: &str = "orders.review_published";
    /// A review is hidden / reopened.
    pub const REVIEW_MODERATED: &str = "orders.review_moderated";
    /// A point-of-sale shift is closed.
    pub const POS_SHIFT_CLOSED: &str = "orders.pos_shift_closed";
    /// A warranty ticket changes status.
    pub const WARRANTY_CLAIM_CHANGED: &str = "orders.warranty_claim_changed";
    /// The COD block list changes.
    pub const COD_BLOCK_CHANGED: &str = "orders.cod_block_changed";
    /// A request to pay the remainder of a partially prepaid order (amount, deadline).
    pub const PREORDER_BALANCE_REQUESTED: &str = "orders.preorder_balance_requested";
    /// The deadline to pay the remainder has passed.
    pub const PREORDER_BALANCE_OVERDUE: &str = "orders.preorder_balance_overdue";
    /// The buyer (a guest, via a second-factor code) changes the shipping address before the goods leave the
    /// warehouse. `after`: `order_id`, `number`, `by`; it carries no address.
    pub const DELIVERY_ADDRESS_CHANGED: &str = "orders.delivery_address_changed";
    /// A guest order is attached to an account that has proven a phone number. `after`: `buyer_user_id`,
    /// `order_ids`.
    pub const ORDERS_CLAIMED: &str = "orders.orders_claimed";
}

/// The shared hold reference code of an order across every holder (`Tender`, journal): `orders:<id>`.
pub fn hold_reference(order_id: Uuid) -> String {
    format!("orders:{order_id}")
}

/// Status of an order (state machine, spec 45).
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

/// Cash-flow mode of an order (D-216). The first three modes share names with `FlowMode` of `ledger`;
/// `ChannelCollect`: an external sales channel (step 48) collects the money then returns it to the seller.
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

/// The source an order is credited to (affiliate marketing, livestream — step 49).
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

/// A line of an order, enough for commission, marketplace fees, points, reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OrderLineSummary {
    pub id: Uuid,
    pub variant_id: Uuid,
    pub product_id: Uuid,
    /// The first category of the product at order time (if any).
    pub category_id: Option<Uuid>,
    /// The selling node of the line (store / seller).
    pub seller_org_id: OrgId,
    pub quantity: i64,
    /// The amount the buyer paid for the line: after discounts, including tax.
    pub net: Money,
    /// The line's tax (already included in `net`).
    pub tax: Money,
    pub returned_quantity: i64,
}

/// A summary snapshot of an order for other modules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OrderSummary {
    pub id: Uuid,
    pub tenant_id: TenantId,
    /// The selling node (store / seller).
    pub org_id: OrgId,
    pub number: String,
    pub buyer_user_id: Option<Uuid>,
    /// `web`, `pos` or an external channel code.
    pub channel: String,
    pub status: OrderStatus,
    pub funds_flow: FundsFlow,
    pub total: Money,
    pub placed_at: Timestamp,
    pub lines: Vec<OrderLineSummary>,
    pub attribution: Option<Attribution>,
}

/// Reads orders.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro này đổi hàm
// `async` thành hàm trả `Box<dyn Future>` để gọi được.
#[async_trait::async_trait]
pub trait Orders: Send + Sync {
    /// One order, or `None`.
    async fn find(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<OrderSummary>, Error>;

    /// IDs of the DELIVERED lines of variant `variant_id` for buyer `buyer` (orders `delivered` /
    /// `completed`) — reviews, warranty, the `verified_buyer` label on Q&A (D-202).
    async fn delivered_lines(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        variant_id: Uuid,
    ) -> Result<Vec<Uuid>, Error>;
}

/// A line of an order imported from outside: the price is decided by the source, not recomputed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ImportedLine {
    pub variant_id: Uuid,
    pub quantity: i64,
    /// Price of one unit before discount, including tax.
    pub unit_price: Money,
    /// Total discount of the whole line.
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

/// An order from an external channel or a data import (steps 48, 56).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ImportedOrder {
    /// The selling node.
    pub org_id: OrgId,
    /// Channel code (`shopee`, `tiktok`, `import`…).
    pub channel: String,
    /// Order ID at the channel; (`channel`, `external_ref`) is the deduplication key.
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
    pub placed_at: Timestamp,
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
        placed_at: Timestamp,
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

/// An order from outside (step 48 sales channels, 56 data import).
#[async_trait::async_trait]
pub trait OrderIntake: Send + Sync {
    /// Deduplicated by (`channel`, `external_ref`) — resending returns `Replayed`. Holds and confirms stock
    /// at once; a shortage does NOT reject (the channel already sold): the order gets `stock_shortage` and
    /// the event `orders.stock_shortage`. The order goes straight to `confirmed`.
    async fn import(
        &self,
        tenant: &TenantContext,
        order: ImportedOrder,
    ) -> Result<Submitted<OrderSummary>, Error>;

    /// A channel reports status — goes through the same state machine, with cause `channel`.
    async fn external_transition(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        to: OrderStatus,
        cause_ref: &str,
    ) -> Result<(), Error>;
}

/// A refund decided by another module (a step 46 dispute): same service as `POST /{id}/refunds`.
#[async_trait::async_trait]
pub trait OrderRefunds: Send + Sync {
    /// Returns the ID of the refund; the same `reference` + the same content → `Replayed`.
    async fn refund(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        amount: Money,
        reason: &str,
        reference: &str,
    ) -> Result<Submitted<Uuid>, Error>;
}

/// Defers completion of an order (a step 46 dispute, a return request).
#[async_trait::async_trait]
pub trait OrderHolds: Send + Sync {
    /// Calling again with the same `reference` adds no line.
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

/// Looks up the verified number of an order-view token (D-202). `auth` uses the kernel's neutral version
/// (`autoxide_kernel::sms::VerifiedPhoneTokens`, the same implementation) so as not to depend on `orders`.
#[async_trait::async_trait]
pub trait OrderLookups: Send + Sync {
    /// The phone number (E.164) that passed OTP for order-view token `token`, while the token is still valid.
    async fn verified_phone(
        &self,
        tenant: &TenantContext,
        token: &str,
    ) -> Result<Option<String>, Error>;
}

/// A cart line handed to [`SellerSplit`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SplitLine {
    pub line_key: String,
    pub variant_id: Uuid,
    /// The node that owns the product per `catalog`.
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

/// A group of lines from the same seller (one child order).
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

/// Splits a cart by seller (step 46 plugs in).
#[async_trait::async_trait]
pub trait SellerSplit: Send + Sync {
    /// Groups lines by selling node. Errors if a seller may not sell (KYC, temporarily locked).
    async fn split(
        &self,
        tenant: &TenantContext,
        lines: &[SplitLine],
    ) -> Result<Vec<SellerGroup>, Error>;
}

/// Pricing / promotion profile of the buyer (step 47 plugs in).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BuyerProfile {
    /// Customer group / segment (for `promotions`).
    pub groups: Vec<Uuid>,
    /// Membership tier.
    pub membership_tier: Option<String>,
    /// Customer group code for `catalog` price lists.
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

/// A way to pay with an internal value (points, gift card, balance — step 47 plugs in). Confirm / refund:
/// the implementer listens to `orders.order_confirmed` / `orders.order_cancelled` / `orders.refund_completed`.
#[async_trait::async_trait]
pub trait Tender: Send + Sync {
    /// `points`, `gift_card`, `store_credit`.
    fn kind(&self) -> &'static str;

    /// Holds at most `max` under `reference` until `expires_at`; returns the amount held. Calling again with
    /// the same `reference` returns the earlier result.
    async fn hold(
        &self,
        tenant: &TenantContext,
        reference: &str,
        buyer: Option<Uuid>,
        input: &Value,
        max: Money,
        expires_at: Timestamp,
    ) -> Result<Money, Error>;

    /// Releases the hold of `reference` (does nothing if none exists).
    async fn release(&self, tenant: &TenantContext, reference: &str) -> Result<(), Error>;

    /// The most buyer `buyer` can pay with this tender for a cart that still has `max` to pay — quotes the
    /// cart before placing the order (e.g. the maximum usable points). Writes nothing. `None`: cannot be
    /// quoted in advance (needs a code such as a gift card, not signed in, no program). Defaults to `None` so
    /// older implementers need no change (D-038).
    async fn limit(
        &self,
        tenant: &TenantContext,
        buyer: Option<Uuid>,
        max: Money,
    ) -> Result<Option<TenderLimit>, Error> {
        let _ = (tenant, buyer, max);
        Ok(None)
    }
}

/// The most a tender can apply to a cart ([`Tender::limit`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TenderLimit {
    /// The maximum amount that can be deducted (not more than what is still to be paid).
    pub amount: Money,
    /// The tender's own unit count (e.g. points), when there is one.
    pub units: Option<i64>,
}

impl TenderLimit {
    pub fn new(amount: Money, units: Option<i64>) -> Self {
        TenderLimit { amount, units }
    }
}

/// Plug points for modules registered AFTER `orders` (steps 46–49): called in their `Module::start`. Each
/// `set_…` plug point can be plugged only once; a second plug → a startup error. If nobody plugs in, `orders`
/// runs as if it had no such feature (one seller, no customer groups, no points).
pub trait OrderExtensions: Send + Sync {
    fn set_seller_split(&self, split: std::sync::Arc<dyn SellerSplit>) -> Result<(), Error>;
    fn set_buyer_groups(&self, groups: std::sync::Arc<dyn BuyerGroups>) -> Result<(), Error>;
    /// Two `Tender`s with the same `kind()` → error.
    fn add_tender(&self, tender: std::sync::Arc<dyn Tender>) -> Result<(), Error>;
}
