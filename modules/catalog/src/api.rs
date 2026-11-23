//! Contract of the `catalog` module for other modules (`inventory`, `promotions`, `shipping`, `orders`,
//! `channels`, `marketplace`…): only what is here may be used from outside
//! (`autoxide_catalog::api::…`). Other modules obtain the trait at startup:
//!
//! ```text
//! let catalog = context.require::<dyn autoxide_catalog::api::Catalog>()?;
//! ```
//!
//! That module must be registered AFTER `catalog` in `crates/app/src/modules.rs`. No module reads
//! the `catalog_…` tables (architecture rule 1). This file does not reference the module's internals.

use autoxide_kernel::time::{Date, Timestamp};
use std::collections::HashMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Fake catalog for other modules' tests (feature `testing`).
#[cfg(feature = "testing")]
pub mod testing;

/// Maximum number of IDs per call to a function that takes a list of IDs.
pub const MAX_IDS: usize = 500;

/// Names of the events `catalog` emits that other modules often listen to (payload `{"before": …, "after": …}`).
pub mod events {
    /// A product was just created.
    pub const PRODUCT_CREATED: &str = "catalog.product_created";
    /// A product's information changed.
    pub const PRODUCT_UPDATED: &str = "catalog.product_updated";
    /// A product's status changed (`draft`, `active`, `inactive`, `archived`).
    pub const PRODUCT_STATUS_CHANGED: &str = "catalog.product_status_changed";
    /// A product was archived.
    pub const PRODUCT_ARCHIVED: &str = "catalog.product_archived";
    /// A product was deleted (only products that have never been sold).
    pub const PRODUCT_DELETED: &str = "catalog.product_deleted";
    /// A variant was just created.
    pub const VARIANT_CREATED: &str = "catalog.variant_created";
    /// A variant's information changed (SKU, weight, sale mode…).
    pub const VARIANT_UPDATED: &str = "catalog.variant_updated";
    /// A variant was deleted.
    pub const VARIANT_DELETED: &str = "catalog.variant_deleted";
    /// A price row was added: price list, variant, new price, `valid_from`.
    pub const PRICE_CHANGED: &str = "catalog.price_changed";
    /// A scheduled price just started or a temporary price just expired (checked every minute): `after` has
    /// `variant_ids` and the `from` / `to` range examined. The prices currently applied to those variants may
    /// have changed.
    pub const PRICE_EFFECTIVE: &str = "catalog.price_effective";
    /// The public price of a product at a selling branch changed (D-233): `before` / `after` are both
    /// `{"org_id", "product_id", "price": Money | null}` — the automatic-discount price if any, otherwise the
    /// lowest price among the variants; `null` when the product is not visible there. The strikethrough price
    /// is not used. The event is only a signal: listeners re-read the current price if needed.
    pub const DISPLAY_PRICE_CHANGED: &str = "catalog.display_price_changed";

    /// Event of the `inventory` module (step 42) that `catalog` listens to in order to get costs for the
    /// price floor (D-222). `catalog` reads the payload
    /// `{"after": {"org_id": "<uuid>", "variant_id": "<uuid>", "unit_cost": {"amount": "<integer in the smallest unit>", "currency": "VND"}}}`
    /// (the JSON form of `Money`); if a field is missing, the event is ignored.
    pub const INVENTORY_COST_CHANGED: &str = "inventory.cost_changed";

    /// Event of the `promotions` module (step 43): the automatic-discount prices of changed variants.
    /// `catalog` reads `after.variant_ids`, rebuilds the storefront, and asks for prices through the
    /// [`SalePriceSource`](super::SalePriceSource) that `promotions` plugs in with
    /// [`CatalogExtensions`](super::CatalogExtensions). If a field is missing, the event is ignored.
    pub const PROMOTIONS_AUTOMATIC_PRICES_CHANGED: &str = "promotions.automatic_prices_changed";

    /// Event of the `orders` module (step 45): an order completed. `catalog` reads `after.order_id` and
    /// `after.lines[].{product_id, quantity}` to count best sellers (sort `best_selling`) and "frequently
    /// bought together"; receiving the same order again does not count it twice.
    pub const ORDERS_ORDER_COMPLETED: &str = "orders.order_completed";

    /// Review event of the `orders` module (step 45): a review was published, hidden or reopened. `catalog`
    /// reads `after.product_id` and
    /// `after.product_rating = {"count": <number of visible reviews>, "sum": <total stars>}` — the product's
    /// **current total** (as in `orders_review_stats`), not the delta, so receiving it again or out of order
    /// is still correct. If a field is missing, the event is ignored.
    pub const ORDERS_REVIEW_PUBLISHED: &str = "orders.review_published";
    /// Like [`ORDERS_REVIEW_PUBLISHED`], when a review is hidden or reopened.
    pub const ORDERS_REVIEW_MODERATED: &str = "orders.review_moderated";

    /// Event of the `orders` module (step 45): an order was confirmed. `catalog` grants digital-goods
    /// download rights when `after.payment_status = "paid"`, reading `after.order_id`, `after.buyer_user_id`
    /// (may be `null`) and `after.lines[].{line_id, variant_id}`. Receiving the same order again grants
    /// nothing more (keyed by `line_id` and file).
    pub const ORDERS_ORDER_CONFIRMED: &str = "orders.order_confirmed";
    /// Event of the `orders` module: an order was cancelled — `catalog` revokes every download right of `after.order_id`.
    pub const ORDERS_ORDER_CANCELLED: &str = "orders.order_cancelled";
    /// Event of the `orders` module: a refund completed. A refund not tied to a return
    /// (`after.return_id = null`) is a whole-order or partial refund decided by the seller — `catalog`
    /// revokes the download rights of `after.order_id`. A refund for a return concerns physical goods only
    /// (digital goods cannot be returned), so it does not touch download rights.
    pub const ORDERS_REFUND_COMPLETED: &str = "orders.refund_completed";

    // Hai bộ đọc dưới đây là hợp đồng payload: `catalog` đọc event của `orders` bằng đúng chúng,
    // và test của `orders` dùng chúng để kiểm event mình phát khớp hợp đồng.

    /// A line of a completed order: product and quantity.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SoldLine {
        pub product_id: uuid::Uuid,
        pub quantity: i64,
    }

    /// Reads `after.order_id` and `after.lines[].{product_id, quantity}` of
    /// [`ORDERS_ORDER_COMPLETED`]; lines with quantity ≤ 0 are dropped. `None` when a field is missing.
    pub fn completed_order(payload: &serde_json::Value) -> Option<(uuid::Uuid, Vec<SoldLine>)> {
        let after = &payload["after"];
        let order: uuid::Uuid = after.get("order_id")?.as_str()?.parse().ok()?;
        let lines = after
            .get("lines")?
            .as_array()?
            .iter()
            .filter_map(|line| {
                let product_id = line.get("product_id")?.as_str()?.parse().ok()?;
                let quantity = line.get("quantity")?.as_i64()?;
                (quantity > 0).then_some(SoldLine { product_id, quantity })
            })
            .collect();
        Some((order, lines))
    }

    /// Reads `after.product_id` and `after.product_rating.{count, sum}` of the review event:
    /// `(product, number of reviews, total stars)`. `None` when a field is missing or the total is impossible
    /// (each review is 1–5 stars).
    pub fn product_rating(payload: &serde_json::Value) -> Option<(uuid::Uuid, i64, i64)> {
        let after = &payload["after"];
        let product: uuid::Uuid = after.get("product_id")?.as_str()?.parse().ok()?;
        let rating = after.get("product_rating")?;
        let count = rating.get("count")?.as_i64()?;
        let sum = rating.get("sum")?.as_i64()?;
        (count >= 0 && sum >= count && sum <= count.saturating_mul(5))
            .then_some((product, count, sum))
    }
}

/// Product kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductKind {
    /// Physical goods: need shipping, have stock.
    Physical,
    /// Digital goods: downloaded, not shipped, no stock.
    Digital,
    /// Service: not shipped, no stock.
    Service,
    /// Combo / product bundle: stock is deducted per component.
    Bundle,
}

/// Product status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductStatus {
    Draft,
    Active,
    Inactive,
    Archived,
}

/// Sale mode of a variant (D-204).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaleMode {
    /// Sold only when in stock.
    InStock,
    /// Sold even when out of stock (shipped when available).
    Backorder,
    /// Pre-order of goods not yet arrived (`preorder_available_on`).
    Preorder,
}

/// Snapshot of a variant, enough to capture into a document and for other modules to decide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VariantSnapshot {
    pub variant_id: Uuid,
    pub product_id: Uuid,
    /// Branch that owns the product (chain, store or seller).
    pub org_id: OrgId,
    pub kind: ProductKind,
    /// The product's; a disabled variant has `variant_active = false`.
    pub status: ProductStatus,
    pub variant_active: bool,
    pub sku: String,
    pub barcode: Option<String>,
    /// Product name in the requested `locale`, otherwise in the product's default language.
    pub name: String,
    /// Label of the option combination, e.g. "Red / M"; empty when the product has no options.
    pub variant_label: String,
    /// Actual language of `name` and `variant_label`.
    pub locale: String,
    /// Tax group code of the tax engine (step 36), e.g. `vat.standard`.
    pub tax_category: String,
    /// Every category of the product PLUS their ancestors.
    pub category_ids: Vec<Uuid>,
    pub weight_grams: Option<i64>,
    pub length_mm: Option<i64>,
    pub width_mm: Option<i64>,
    pub height_mm: Option<i64>,
    /// `false` for digital goods and services.
    pub requires_shipping: bool,
    /// `false` for digital goods, services and combos (a combo deducts stock per component).
    pub track_inventory: bool,
    pub sale_mode: SaleMode,
    /// Expected availability date when `Preorder` (in the branch's time zone).
    pub preorder_available_on: Option<Date>,
    /// Total pre-order slots; `None` = unlimited.
    pub preorder_limit: Option<i64>,
    /// Pre-orders no longer accepted.
    pub preorder_ends_at: Option<Timestamp>,
    /// Legacy flag, still readable until `inventory` and `orders` have fully moved to `sale_mode` (D-204):
    /// `true` when `sale_mode` is `Backorder`.
    pub allow_backorder: bool,
    pub lot_tracked: bool,
    pub serial_tracked: bool,
    /// Warranty months; `None` = no warranty.
    pub warranty_months: Option<i32>,
    pub image_media_id: Option<Uuid>,
    /// Version of the variant, for the capturing side to record.
    pub updated_at: Timestamp,
}

/// Price query: prices of the variants at store `org_id` at `at`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PriceQuery {
    /// Selling store.
    pub org_id: OrgId,
    /// B2B company (step 59; currently `None`) — D-222.
    pub company_id: Option<Uuid>,
    /// Customer group code (provided by step 47; currently free text).
    pub customer_group: Option<String>,
    /// Sales channel (step 48; currently `None`) — D-222.
    pub channel: Option<String>,
    /// Market (step 63; currently `None`) — D-222.
    pub market: Option<String>,
    pub currency: Currency,
    pub at: Timestamp,
    /// At most [`MAX_IDS`].
    pub variant_ids: Vec<Uuid>,
}

impl PriceQuery {
    /// General price query (no customer group, channel or market) at store `org_id` at `at`.
    pub fn new(org_id: OrgId, currency: Currency, at: Timestamp, variant_ids: Vec<Uuid>) -> Self {
        PriceQuery {
            org_id,
            company_id: None,
            customer_group: None,
            channel: None,
            market: None,
            currency,
            at,
            variant_ids,
        }
    }

    /// Queries the price of customer group `group`.
    pub fn for_customer_group(mut self, group: impl Into<String>) -> Self {
        self.customer_group = Some(group.into());
        self
    }

    /// Queries the price of sales channel `channel` (step 48).
    pub fn for_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = Some(channel.into());
        self
    }

    /// Queries the price of market `market` (step 63).
    pub fn for_market(mut self, market: impl Into<String>) -> Self {
        self.market = Some(market.into());
        self
    }

    /// Queries the price of B2B company `company_id` (step 59).
    pub fn for_company(mut self, company_id: Uuid) -> Self {
        self.company_id = Some(company_id);
        self
    }
}

/// Price of a variant, enough to capture into a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PriceSnapshot {
    pub variant_id: Uuid,
    /// The winning price list, in price-list selection order (D-222).
    pub price_list_id: Uuid,
    /// The specific price row — captured into the document.
    pub price_id: Uuid,
    /// Price of one unit.
    pub amount: Money,
    /// Price includes tax.
    pub tax_inclusive: bool,
    /// Strikethrough price (for display only).
    pub compare_at: Option<Money>,
    pub valid_from: Timestamp,
    /// Expiry (temporary price); `None` = until a new row exists.
    pub valid_to: Option<Timestamp>,
}

/// What to do when a price is below the floor (D-222).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FloorAction {
    /// Can be saved, with a warning.
    Warn,
    /// Cannot be saved.
    Block,
}

/// The price floor in effect for a variant at a branch (D-222): the nearest branch's rule wins, the nearest
/// category wins. `promotions` checks it at activation and clamps prices when computing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PriceFloor {
    pub variant_id: Uuid,
    /// The winning price-floor rule.
    pub guard_id: Uuid,
    /// Absolute minimum price, if the rule has one.
    pub min_price: Option<Money>,
    /// Minimum profit margin over cost, in basis points (`1500` = 15%), if the rule has one.
    pub min_margin_bp: Option<i64>,
    /// Known unit cost (from the event `inventory.cost_changed`), if any.
    pub unit_cost: Option<Money>,
    /// Lowest allowed price: the larger of `min_price` and cost × (1 + margin); `None` when
    /// the rule has only a margin and the cost is not known.
    pub floor: Option<Money>,
    pub action: FloorAction,
}

/// A component of a combo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleComponent {
    pub variant_id: Uuid,
    /// Units of the component in one combo; > 0.
    pub quantity: i64,
}

/// Reads products, variants and prices for other modules.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro này đổi hàm
// `async` thành hàm trả về `Box<dyn Future>` để gọi được.
#[async_trait::async_trait]
pub trait Catalog: Send + Sync {
    /// Variants by ID, in the order of `ids`, skipping IDs that do not exist. More than [`MAX_IDS`] is the
    /// error `catalog.too_many_ids`. One call costs a few queries, independent of the number of IDs.
    async fn variants(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
        locale: Option<&str>,
    ) -> Result<Vec<VariantSnapshot>, Error>;

    /// One variant by SKU (case-insensitive, whitespace trimmed) or barcode — for selling at the
    /// counter, omnichannel.
    async fn variant_by_code(
        &self,
        tenant: &TenantContext,
        code: &str,
        locale: Option<&str>,
    ) -> Result<Option<VariantSnapshot>, Error>;

    /// Prices in effect at `query.at` (rows with `valid_from <= at`, not expired), in price-list
    /// selection order (D-222). A variant with no price is absent from the result.
    async fn prices(
        &self,
        tenant: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error>;

    /// Price in effect JUST BEFORE `query.at` (rows with `valid_from < at`) — the "price just before the
    /// promotion" (step 43). Same price lists as [`prices`](Self::prices).
    async fn reference_prices(
        &self,
        tenant: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error>;

    /// Price floors in effect for the variants at branch `org_id`, in currency `currency` (D-222). A variant
    /// with no rule is absent.
    async fn price_floors(
        &self,
        tenant: &TenantContext,
        org_id: OrgId,
        currency: Currency,
        variant_ids: &[Uuid],
    ) -> Result<Vec<PriceFloor>, Error>;

    /// Current public prices (D-233, wishlist price-drop alerts), at the branch that owns the product
    /// (nearest shared price list in its chain): per variant it is the list price or the automatic-discount
    /// price if lower; per product it is the lowest price among the variants on sale. Does not use the
    /// strikethrough price, customer-group prices or discount codes. Not for sale / no price: absent. Each
    /// list holds at most [`MAX_IDS`].
    ///
    /// The default implementation returns empty, so fakes in other modules' tests need no changes.
    async fn display_prices(
        &self,
        tenant: &TenantContext,
        product_ids: &[Uuid],
        variant_ids: &[Uuid],
    ) -> Result<DisplayPrices, Error> {
        let _ = (tenant, product_ids, variant_ids);
        Ok(DisplayPrices::default())
    }

    /// Components of the combo variants (a variant that is not a combo is absent).
    async fn bundle_components(
        &self,
        tenant: &TenantContext,
        variant_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<BundleComponent>>, Error>;
}

/// Public prices (D-233) per product and per variant, see [`Catalog::display_prices`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DisplayPrices {
    pub products: HashMap<Uuid, Money>,
    pub variants: HashMap<Uuid, Money>,
}

/// Automatic-discount price of a variant (from promotions needing no code and no cart condition).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SalePrice {
    pub variant_id: Uuid,
    /// Price of one unit after the discount, in the same currency as the list price that was sent.
    pub sale_price: Money,
    /// When the discounted price ends.
    pub ends_at: Timestamp,
}

impl SalePrice {
    pub fn new(variant_id: Uuid, sale_price: Money, ends_at: Timestamp) -> Self {
        SalePrice { variant_id, sale_price, ends_at }
    }
}

/// Source of automatic-discount prices for the public storefront. `promotions` registers AFTER `catalog` and
/// depends on `catalog`, so `catalog` cannot call its trait directly (a dependency cycle): the promotions
/// module implements this trait and plugs it in with [`CatalogExtensions::set_sale_price_source`].
#[async_trait::async_trait]
pub trait SalePriceSource: Send + Sync {
    /// Discounted prices of the variants at selling branch `org_id` at `at`, from the list prices `prices`
    /// (variant, price). A variant that gets no discount is absent. Call outside a transaction.
    async fn sale_prices(
        &self,
        tenant: &TenantContext,
        org_id: OrgId,
        at: Timestamp,
        prices: &[(Uuid, Money)],
    ) -> Result<Vec<SalePrice>, Error>;
}

/// A product about to go on sale, handed to [`ListingGate`] for review.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListingCandidate {
    pub product_id: Uuid,
    /// Branch that owns the product (the seller when a marketplace).
    pub org_id: OrgId,
    /// Categories of the product (without ancestors).
    pub category_ids: Vec<Uuid>,
    pub kind: ProductKind,
}

impl ListingCandidate {
    pub fn new(
        product_id: Uuid,
        org_id: OrgId,
        category_ids: Vec<Uuid>,
        kind: ProductKind,
    ) -> Self {
        ListingCandidate { product_id, org_id, category_ids, kind }
    }
}

/// Result of the listing review.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ListingDecision {
    Allow,
    /// Listing is not allowed. `code` is the stable error code of the plugged-in module (e.g.
    /// `marketplace.seller_not_verified`, translated in that module), `message` says clearly what to do.
    Deny {
        code: String,
        message: String,
    },
}

/// Listing condition set by another module (step 46: seller verified, product approved, category
/// paperwork complete). `catalog` asks before moving a product to `active`, outside a transaction.
#[async_trait::async_trait]
pub trait ListingGate: Send + Sync {
    async fn can_publish(
        &self,
        tenant: &TenantContext,
        candidate: &ListingCandidate,
    ) -> Result<ListingDecision, Error>;
}

/// Extension point for modules registered after `catalog`: obtain it with
/// `context.require::<dyn CatalogExtensions>()` in `Module::start`.
pub trait CatalogExtensions: Send + Sync {
    /// Plugs in the automatic-discount price source (one source; plugging a second keeps the first and returns `false`).
    fn set_sale_price_source(&self, source: std::sync::Arc<dyn SalePriceSource>) -> bool;

    /// Plugs in the listing condition (one condition; plugging a second keeps the first and returns `false`).
    /// Has a default implementation (accepts nothing, returns `false`) so existing implementations — e.g.
    /// fakes in other modules' tests — need no changes when the trait gains a function.
    fn set_listing_gate(&self, gate: std::sync::Arc<dyn ListingGate>) -> bool {
        let _ = gate;
        false
    }
}
