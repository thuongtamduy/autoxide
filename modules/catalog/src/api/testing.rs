//! Shared fake catalog for other modules' tests (feature `testing`, declared only in
//! `[dev-dependencies]`; convention D-20261122-module-test-fakes).
//!
//! Keeps variants, price history, price floors and combos in memory. Prices follow history like the real
//! `catalog`: `prices` takes the most recent price in effect up to `at` (inclusive), `reference_prices` takes
//! the price in effect BEFORE `at` (the reference price of a promotion). `display_prices`: for a variant it
//! is the current price, for a product the lowest price among variants on sale. A discount price source can
//! be plugged in (`CatalogExtensions`) so the promotions module exercises the real path.
//!
//! Each module builds its own variants (weight, SKU…) and then calls `insert_variant`; [`variant`] gives the
//! default.
//!
//! ```ignore
//! let catalog = Arc::new(FakeCatalog::new());
//! let id = Uuid::now_v7();
//! catalog.insert_variant(variant(id, org));
//! catalog.set_price(id, Money::new(100_000, Currency::VND));
//! let app = TestApp::builder(test).provide::<dyn Catalog>(catalog.clone())…;
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_kernel::time::{SignedDuration, Timestamp};
use uuid::Uuid;

use super::{
    BundleComponent, Catalog, CatalogExtensions, DisplayPrices, FloorAction, PriceFloor,
    PriceQuery, PriceSnapshot, ProductKind, ProductStatus, SaleMode, SalePriceSource,
    VariantSnapshot,
};

#[derive(Default)]
struct State {
    /// In the order added.
    variants: Vec<VariantSnapshot>,
    /// Price history of each variant: (effective from, price).
    prices: HashMap<Uuid, Vec<(Timestamp, Money)>>,
    floors: HashMap<Uuid, (Money, FloorAction)>,
    bundles: HashMap<Uuid, Vec<BundleComponent>>,
    sale_source: Option<Arc<dyn SalePriceSource>>,
}

/// Fake catalog. Publish it with `TestApp::builder(…).provide::<dyn Catalog>(catalog.clone())` (and
/// `provide::<dyn CatalogExtensions>` when the module under test plugs in a discount price source).
#[derive(Default)]
pub struct FakeCatalog {
    state: Mutex<State>,
}

// Khoá `Mutex` mà không `unwrap`: test khác panic giữa chừng thì vẫn đọc tiếp dữ liệu bên trong.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Default variant `id` of branch `org`: a physical item on sale of a new product, SKU is the ID,
/// name "Item", tax `vat.standard`, needs shipping, tracks stock, sold when in stock.
pub fn variant(id: Uuid, org: OrgId) -> VariantSnapshot {
    VariantSnapshot {
        variant_id: id,
        product_id: Uuid::now_v7(),
        org_id: org,
        kind: ProductKind::Physical,
        status: ProductStatus::Active,
        variant_active: true,
        sku: id.to_string(),
        barcode: None,
        name: "Item".to_owned(),
        variant_label: String::new(),
        locale: "vi".to_owned(),
        tax_category: "vat.standard".to_owned(),
        category_ids: Vec::new(),
        weight_grams: None,
        length_mm: None,
        width_mm: None,
        height_mm: None,
        requires_shipping: true,
        track_inventory: true,
        sale_mode: SaleMode::InStock,
        preorder_available_on: None,
        preorder_limit: None,
        preorder_ends_at: None,
        allow_backorder: false,
        lot_tracked: false,
        serial_tracked: false,
        warranty_months: None,
        image_media_id: None,
        updated_at: Timestamp::now(),
    }
}

fn price_at(
    history: &[(Timestamp, Money)],
    at: Timestamp,
    before: bool,
) -> Option<(Timestamp, Money)> {
    history
        .iter()
        .filter(|(from, _)| if before { *from < at } else { *from <= at })
        .max_by_key(|(from, _)| *from)
        .copied()
}

impl FakeCatalog {
    pub fn new() -> Self {
        FakeCatalog::default()
    }

    /// Adds (or replaces) the variant carrying the exact `variant_id`.
    pub fn insert_variant(&self, variant: VariantSnapshot) {
        let mut state = lock(&self.state);
        state.variants.retain(|old| old.variant_id != variant.variant_id);
        state.variants.push(variant);
    }

    /// Edits variant `id` (e.g. stop selling) if it exists.
    pub fn edit_variant(&self, id: Uuid, edit: impl FnOnce(&mut VariantSnapshot)) {
        if let Some(found) =
            lock(&self.state).variants.iter_mut().find(|variant| variant.variant_id == id)
        {
            edit(found);
        }
    }

    /// Variant `id`, if it exists.
    pub fn variant_of(&self, id: Uuid) -> Option<VariantSnapshot> {
        lock(&self.state).variants.iter().find(|variant| variant.variant_id == id).cloned()
    }

    /// Price of variant `id` is `price`, in effect from 30 days ago (older history is dropped).
    pub fn set_price(&self, id: Uuid, price: Money) {
        let from = Timestamp::now() - SignedDuration::from_hours(24 * 30);
        lock(&self.state).prices.insert(id, vec![(from, price)]);
    }

    /// Replaces the whole price history of variant `id`: the points (effective from, price).
    pub fn set_price_history(&self, id: Uuid, history: Vec<(Timestamp, Money)>) {
        lock(&self.state).prices.insert(id, history);
    }

    /// Adds a price point: `price` in effect from `from`.
    pub fn push_price(&self, id: Uuid, price: Money, from: Timestamp) {
        lock(&self.state).prices.entry(id).or_default().push((from, price));
    }

    /// Price of variant `id` in effect at `at` (like `prices`), if any.
    pub fn price_of(&self, id: Uuid, at: Timestamp) -> Option<Money> {
        let state = lock(&self.state);
        price_at(state.prices.get(&id)?, at, false).map(|(_, price)| price)
    }

    /// Price floor of variant `id`: below `floor`, do `action`.
    pub fn set_floor(&self, id: Uuid, floor: Money, action: FloorAction) {
        lock(&self.state).floors.insert(id, (floor, action));
    }

    /// Components of combo `id`.
    pub fn set_bundle(&self, id: Uuid, components: Vec<BundleComponent>) {
        lock(&self.state).bundles.insert(id, components);
    }

    /// The discount price source another module plugged in (`CatalogExtensions::set_sale_price_source`).
    pub fn sale_source(&self) -> Option<Arc<dyn SalePriceSource>> {
        lock(&self.state).sale_source.clone()
    }

    fn prices_for(&self, query: &PriceQuery, before: bool) -> Vec<PriceSnapshot> {
        let state = lock(&self.state);
        query
            .variant_ids
            .iter()
            .filter_map(|id| {
                let (from, amount) = price_at(state.prices.get(id)?, query.at, before)?;
                Some(PriceSnapshot {
                    variant_id: *id,
                    price_list_id: Uuid::nil(),
                    price_id: Uuid::nil(),
                    amount,
                    tax_inclusive: true,
                    compare_at: None,
                    valid_from: from,
                    valid_to: None,
                })
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl Catalog for FakeCatalog {
    async fn variants(
        &self,
        _: &TenantContext,
        ids: &[Uuid],
        _: Option<&str>,
    ) -> Result<Vec<VariantSnapshot>, Error> {
        let state = lock(&self.state);
        Ok(ids
            .iter()
            .filter_map(|id| state.variants.iter().find(|variant| variant.variant_id == *id))
            .cloned()
            .collect())
    }

    async fn variant_by_code(
        &self,
        _: &TenantContext,
        code: &str,
        _: Option<&str>,
    ) -> Result<Option<VariantSnapshot>, Error> {
        let state = lock(&self.state);
        Ok(state
            .variants
            .iter()
            .find(|variant| variant.sku == code || variant.barcode.as_deref() == Some(code))
            .cloned())
    }

    async fn prices(
        &self,
        _: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error> {
        Ok(self.prices_for(query, false))
    }

    async fn reference_prices(
        &self,
        _: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error> {
        Ok(self.prices_for(query, true))
    }

    async fn price_floors(
        &self,
        _: &TenantContext,
        _: OrgId,
        _: Currency,
        variant_ids: &[Uuid],
    ) -> Result<Vec<PriceFloor>, Error> {
        let state = lock(&self.state);
        Ok(variant_ids
            .iter()
            .filter_map(|id| {
                let (floor, action) = state.floors.get(id)?;
                Some(PriceFloor {
                    variant_id: *id,
                    guard_id: Uuid::nil(),
                    min_price: Some(*floor),
                    min_margin_bp: None,
                    unit_cost: None,
                    floor: Some(*floor),
                    action: *action,
                })
            })
            .collect())
    }

    async fn display_prices(
        &self,
        _: &TenantContext,
        product_ids: &[Uuid],
        variant_ids: &[Uuid],
    ) -> Result<DisplayPrices, Error> {
        let state = lock(&self.state);
        let now = Timestamp::now();
        let current = |id: &Uuid| {
            state.prices.get(id).and_then(|history| price_at(history, now, false)).map(|p| p.1)
        };
        let mut prices = DisplayPrices::default();
        for id in variant_ids {
            if let Some(price) = current(id) {
                prices.variants.insert(*id, price);
            }
        }
        for product in product_ids {
            let lowest = state
                .variants
                .iter()
                .filter(|variant| variant.product_id == *product && variant.variant_active)
                .filter_map(|variant| current(&variant.variant_id))
                .min_by_key(|price| price.minor());
            if let Some(price) = lowest {
                prices.products.insert(*product, price);
            }
        }
        Ok(prices)
    }

    async fn bundle_components(
        &self,
        _: &TenantContext,
        variant_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<BundleComponent>>, Error> {
        let state = lock(&self.state);
        Ok(variant_ids
            .iter()
            .filter_map(|id| state.bundles.get(id).map(|parts| (*id, parts.clone())))
            .collect())
    }
}

impl CatalogExtensions for FakeCatalog {
    fn set_sale_price_source(&self, source: Arc<dyn SalePriceSource>) -> bool {
        let mut state = lock(&self.state);
        if state.sale_source.is_some() {
            return false;
        }
        state.sale_source = Some(source);
        true
    }
}
