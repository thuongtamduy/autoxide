//! Contract of the `inventory` module for other modules (`orders`, `catalog`, `channels`, reports…):
//! only what is here may be used from outside (`autoxide_inventory::api::…`). Other modules
//! obtain the traits at startup with `context.require::<dyn …>()` and must be registered AFTER `inventory`
//! in `crates/app/src/modules.rs`. No module reads the `inventory_…` tables (architecture rule 1).
//! This file does not reference the module's internals.

use autoxide_kernel::error::Error;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_kernel::time::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum number of IDs per call to a function that takes a list of IDs.
pub const MAX_IDS: usize = 500;

/// Names of the events `inventory` emits that other modules often listen to (payload `{"before": …, "after": …}`).
pub mod events {
    /// Stock of a row (variant × warehouse) changed. `after`: `level_id`, `org_id`, `variant_id`,
    /// `on_hand`, `available`, `reserved`; `before`: `on_hand`, `available`. Several changes to
    /// one row in one transaction are merged into one event.
    pub const LEVEL_CHANGED: &str = "inventory.level_changed";
    /// Sellable stock went from above to ≤ the reorder point (`reorder_point`).
    pub const LOW_STOCK: &str = "inventory.low_stock";
    /// Sellable stock went from above 0 down to ≤ 0.
    pub const OUT_OF_STOCK: &str = "inventory.out_of_stock";
    /// A row of the movement ledger was recorded.
    pub const MOVEMENT_RECORDED: &str = "inventory.movement_recorded";
    /// The average cost of a variant at a warehouse changed. `after`: `org_id`, `variant_id`,
    /// `unit_cost` (the JSON form of `Money`) — `catalog` listens to get costs for the price floor (D-222).
    pub const COST_CHANGED: &str = "inventory.cost_changed";
    /// Goods were held for an order (`after`: the hold).
    pub const RESERVATION_HELD: &str = "inventory.reservation_held";
    /// A hold was confirmed (the order was placed).
    pub const RESERVATION_CONFIRMED: &str = "inventory.reservation_confirmed";
    /// A hold was released (`after.reason`: `cancelled`, `payment_failed`, `checkout_aborted`, `manual`).
    pub const RESERVATION_RELEASED: &str = "inventory.reservation_released";
    /// A hold expired before being confirmed.
    pub const RESERVATION_EXPIRED: &str = "inventory.reservation_expired";
    /// Incoming stock was allocated to a pre-order line (D-204). `after`: `reservation_id`, `order_id`,
    /// `line_key`, `variant_id`, `warehouse_org_id`, `quantity` (this time), `allocated_quantity`
    /// (cumulative) — `orders` moves the order to preparation once fully allocated.
    pub const PREORDER_ALLOCATED: &str = "inventory.preorder_allocated";
}

/// A line of an order that needs goods held.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReserveLine {
    /// Order line code (unique within the order, 1–100 characters): issuing stock and error replies use this code.
    pub line_key: String,
    pub variant_id: Uuid,
    /// 1…1 000 000 units (combo: number of combos).
    pub quantity: i64,
}

impl ReserveLine {
    pub fn new(line_key: impl Into<String>, variant_id: Uuid, quantity: i64) -> Self {
        ReserveLine { line_key: line_key.into(), variant_id, quantity }
    }
}

/// Command to hold goods for a whole order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReserveRequest {
    /// Reference code: an order holds goods once.
    pub order_id: Uuid,
    /// Store / sales channel: the warehouses serving it are candidates.
    pub selling_org_id: OrgId,
    /// Pick up at the store, sold at the counter: take only from this warehouse.
    pub fixed_org_id: Option<OrgId>,
    /// Province code (`address` module) of the delivery place, to prefer warehouses in the same province.
    pub ship_to_province: Option<String>,
    /// Payment deadline set by `orders` (≤ `AUTOXIDE_INVENTORY_MAX_HOLD_HOURS` from now).
    pub expires_at: Timestamp,
    pub lines: Vec<ReserveLine>,
}

impl ReserveRequest {
    pub fn new(
        order_id: Uuid,
        selling_org_id: OrgId,
        expires_at: Timestamp,
        lines: Vec<ReserveLine>,
    ) -> Self {
        ReserveRequest {
            order_id,
            selling_org_id,
            fixed_org_id: None,
            ship_to_province: None,
            expires_at,
            lines,
        }
    }

    /// Take goods only from warehouse `org` (pick up at the store, sold at the counter).
    pub fn from_warehouse(mut self, org: OrgId) -> Self {
        self.fixed_org_id = Some(org);
        self
    }

    /// Deliver to province `province` (`address` code): warehouses in the same province first, before the store's priority setting is applied.
    pub fn ship_to(mut self, province: impl Into<String>) -> Self {
        self.ship_to_province = Some(province.into());
        self
    }
}

/// State of a stock hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationStatus {
    /// Held, with a deadline (`expires_at`).
    Held,
    /// Order placed: held until issued or cancelled, no deadline.
    Confirmed,
    /// Every line issued (or the remainder released after a partial issue).
    Fulfilled,
    /// Released (cancelled, payment failed…) before anything was issued.
    Released,
    /// Expired while still `Held`: the goods went back to sellable stock.
    Expired,
}

impl ReservationStatus {
    /// Stable names: `held`, `confirmed`, `fulfilled`, `released`, `expired`.
    pub const fn as_str(self) -> &'static str {
        match self {
            ReservationStatus::Held => "held",
            ReservationStatus::Confirmed => "confirmed",
            ReservationStatus::Fulfilled => "fulfilled",
            ReservationStatus::Released => "released",
            ReservationStatus::Expired => "expired",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [
            ReservationStatus::Held,
            ReservationStatus::Confirmed,
            ReservationStatus::Fulfilled,
            ReservationStatus::Released,
            ReservationStatus::Expired,
        ]
        .into_iter()
        .find(|status| status.as_str() == text)
    }
}

/// A part of an order line held at one warehouse. An order line has several parts when it is a combo (one
/// part per component) or when the store allows splitting across warehouses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReservedLine {
    pub line_key: String,
    /// The real variant deducted (component, when the line is a combo).
    pub variant_id: Uuid,
    /// The combo variant of the order line, when the line is a combo.
    pub bundle_variant_id: Option<Uuid>,
    pub warehouse_org_id: OrgId,
    /// Units of this variant held.
    pub quantity: i64,
    pub fulfilled_quantity: i64,
    pub released_quantity: i64,
    /// Held when sellable stock is not enough (a product that may be sold when out of stock).
    pub backordered: bool,
    /// Pre-order line (D-204).
    pub preordered: bool,
    /// Pre-order line: units of incoming stock allocated to this line (cumulative, including those already issued).
    pub allocated_quantity: i64,
}

/// Stock hold of an order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Reservation {
    pub id: Uuid,
    pub order_id: Uuid,
    pub selling_org_id: OrgId,
    pub status: ReservationStatus,
    /// `None` once confirmed (or finished).
    pub expires_at: Option<Timestamp>,
    /// A line that does not track stock (digital goods, services) has no part here.
    pub lines: Vec<ReservedLine>,
}

/// A line of a stock issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FulfilLine {
    pub line_key: String,
    /// Units of the order line (combo: number of combos) issued this time.
    pub quantity: i64,
    /// Issued lot; when empty and the variant tracks lots, the lot expiring first is chosen automatically.
    pub lot_code: Option<String>,
    /// Required, `quantity` numbers in total, when the variant tracks serials.
    pub serials: Vec<String>,
}

impl FulfilLine {
    pub fn new(line_key: impl Into<String>, quantity: i64) -> Self {
        FulfilLine { line_key: line_key.into(), quantity, lot_code: None, serials: Vec::new() }
    }

    pub fn lot(mut self, lot_code: impl Into<String>) -> Self {
        self.lot_code = Some(lot_code.into());
        self
    }

    pub fn serials(mut self, serials: Vec<String>) -> Self {
        self.serials = serials;
        self
    }
}

/// Command to issue stock against the hold of an order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FulfilRequest {
    pub order_id: Uuid,
    /// Code of the issue (e.g. a waybill code): sending the same code again does not issue a second time.
    pub fulfilment_key: String,
    /// Empty: issue the whole remainder.
    pub lines: Vec<FulfilLine>,
}

impl FulfilRequest {
    pub fn new(order_id: Uuid, fulfilment_key: impl Into<String>, lines: Vec<FulfilLine>) -> Self {
        FulfilRequest { order_id, fulfilment_key: fulfilment_key.into(), lines }
    }
}

/// A line of returned goods to restock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestockLine {
    pub variant_id: Uuid,
    pub quantity: i64,
    /// `false`: only record it (damaged goods), do not add to stock.
    pub sellable: bool,
    pub lot_code: Option<String>,
    pub serials: Vec<String>,
}

impl RestockLine {
    pub fn new(variant_id: Uuid, quantity: i64, sellable: bool) -> Self {
        RestockLine { variant_id, quantity, sellable, lot_code: None, serials: Vec::new() }
    }
}

/// Command to restock returned goods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestockRequest {
    /// E.g. `return:<return_id>`; permanent deduplication.
    pub reference: String,
    pub order_id: Option<Uuid>,
    pub warehouse_org_id: OrgId,
    pub lines: Vec<RestockLine>,
}

impl RestockRequest {
    pub fn new(
        reference: impl Into<String>,
        order_id: Option<Uuid>,
        warehouse_org_id: OrgId,
        lines: Vec<RestockLine>,
    ) -> Self {
        RestockRequest { reference: reference.into(), order_id, warehouse_org_id, lines }
    }
}

/// Stock of a variant at a warehouse serving a store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Availability {
    pub variant_id: Uuid,
    pub warehouse_org_id: OrgId,
    pub on_hand: i64,
    pub available: i64,
    pub incoming: i64,
}

/// Why the hold was released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseReason {
    Cancelled,
    PaymentFailed,
    Expired,
    CheckoutAborted,
    /// Staff manually clear a stuck hold.
    Manual,
}

impl ReleaseReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReleaseReason::Cancelled => "cancelled",
            ReleaseReason::PaymentFailed => "payment_failed",
            ReleaseReason::Expired => "expired",
            ReleaseReason::CheckoutAborted => "checkout_aborted",
            ReleaseReason::Manual => "manual",
        }
    }
}

/// Stock for other modules: hold → confirm → issue / release by `order_id` (`orders`, step
/// 45), restock returned goods, read sellable stock.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro này đổi hàm
// `async` thành hàm trả về `Box<dyn Future>` để gọi được.
#[async_trait::async_trait]
pub trait Inventory: Send + Sync {
    /// Stock of the variants at each warehouse serving `selling_org_id` (in warehouse-selection order), at most
    /// [`MAX_IDS`] variants. A variant with no stock at any warehouse is absent.
    async fn availability(
        &self,
        tenant: &TenantContext,
        selling_org_id: OrgId,
        variant_ids: &[Uuid],
    ) -> Result<Vec<Availability>, Error>;

    /// Holds goods for a whole order: all or nothing. Calling again with the same `order_id` + same content →
    /// `Submitted::Replayed`; different content → `reference_reused`. Combos are split into components;
    /// variants that do not track stock are skipped. Errors: `inventory.insufficient_stock`,
    /// `inventory.no_source`, `inventory.hold_too_long`, `inventory.preorder_sold_out`.
    async fn reserve(
        &self,
        tenant: &TenantContext,
        request: &ReserveRequest,
    ) -> Result<Submitted<Reservation>, Error>;

    /// Held → confirmed (deadline removed). Already confirmed / issued: returns as before. Expired / released:
    /// `inventory.reservation_expired`; none: `inventory.reservation_not_found`.
    async fn confirm(&self, tenant: &TenantContext, order_id: Uuid) -> Result<Reservation, Error>;

    /// Returns the un-issued part to sellable stock. No hold: `Ok(None)`; already released / finished:
    /// returns as before (safe to call again).
    async fn release(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        reason: ReleaseReason,
    ) -> Result<Option<Reservation>, Error>;

    /// Issues stock against a hold (a partial issue is allowed; a still-held hold is confirmed at the same
    /// time). Calling again with the same `fulfilment_key` + same content: returns as before, does not issue
    /// a second time.
    async fn fulfil(
        &self,
        tenant: &TenantContext,
        request: &FulfilRequest,
    ) -> Result<Reservation, Error>;

    /// Restocks returned goods. Calling again with the same `reference` + same content: does not add a second time.
    async fn restock(&self, tenant: &TenantContext, request: &RestockRequest) -> Result<(), Error>;

    /// The variants (among `variant_ids`, at most [`MAX_IDS`]) that are sellable right now at store
    /// `store_org_id` — the "only goods available at store X" filter of `catalog` (D-200).
    async fn in_stock_at(
        &self,
        tenant: &TenantContext,
        store_org_id: OrgId,
        variant_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, Error>;

    /// The variants (among `variant_ids`) for which account `user_id` has a confirmed back-in-stock
    /// subscription — so `customers` (step 47) does not send a duplicate message for the same restock
    /// (D-237).
    async fn restock_alerts_of(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        variant_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, Error>;
}
