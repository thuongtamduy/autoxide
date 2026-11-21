//! `Catalog` và `Inventory` giả cho báo yêu thích: giá công khai và tồn đặt tay, đăng ký báo có hàng
//! của `inventory` theo tài khoản.

use autoxide_kernel::time::Timestamp;
use std::collections::HashMap;
use std::sync::Mutex;

use autoxide_catalog::api::{
    BundleComponent, Catalog, DisplayPrices, PriceFloor, PriceQuery, PriceSnapshot, VariantSnapshot,
};
use autoxide_inventory::api::{
    Availability, FulfilRequest, Inventory, ReleaseReason, Reservation, ReserveRequest,
    RestockRequest,
};
use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, TenantContext, root_of};
use serde_json::json;
use uuid::Uuid;

pub(crate) fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::parse("VND").unwrap())
}

/// Giá công khai theo sản phẩm / biến thể; biến thể → sản phẩm.
#[derive(Default)]
pub(crate) struct FakeCatalog {
    pub products: Mutex<HashMap<Uuid, Money>>,
    pub variant_prices: Mutex<HashMap<Uuid, Money>>,
    pub owners: Mutex<HashMap<Uuid, Uuid>>,
}

impl FakeCatalog {
    /// Sản phẩm có một biến thể, giá `price`; trả (sản phẩm, biến thể).
    pub(crate) fn product(&self, price: i64) -> (Uuid, Uuid) {
        let (product, variant) = (Uuid::now_v7(), Uuid::now_v7());
        self.owners.lock().unwrap().insert(variant, product);
        self.set_price(product, variant, price);
        (product, variant)
    }

    pub(crate) fn set_price(&self, product: Uuid, variant: Uuid, price: i64) {
        self.products.lock().unwrap().insert(product, vnd(price));
        self.variant_prices.lock().unwrap().insert(variant, vnd(price));
    }
}

fn snapshot(variant: Uuid, product: Uuid) -> VariantSnapshot {
    let json = json!({
        "variant_id": variant, "product_id": product, "org_id": root_of(DEFAULT_TENANT),
        "kind": "physical", "status": "active", "variant_active": true,
        "sku": format!("SKU-{}", variant.simple()), "barcode": null, "name": "Item", "variant_label": "",
        "locale": "vi", "tax_category": "vat.standard", "category_ids": [],
        "weight_grams": 100, "length_mm": null, "width_mm": null, "height_mm": null,
        "requires_shipping": true, "track_inventory": true, "sale_mode": "in_stock",
        "preorder_available_on": null, "preorder_limit": null, "preorder_ends_at": null,
        "allow_backorder": false, "lot_tracked": false, "serial_tracked": false,
        "warranty_months": null, "image_media_id": null, "updated_at": Timestamp::now(),
    });
    // `VariantSnapshot` là `#[non_exhaustive]`: crate khác dựng qua serde.
    serde_json::from_value(json).unwrap()
}

#[async_trait::async_trait]
impl Catalog for FakeCatalog {
    async fn variants(
        &self,
        _: &TenantContext,
        ids: &[Uuid],
        _: Option<&str>,
    ) -> Result<Vec<VariantSnapshot>, Error> {
        let owners = self.owners.lock().unwrap();
        Ok(ids
            .iter()
            .filter_map(|id| owners.get(id).map(|product| snapshot(*id, *product)))
            .collect())
    }
    async fn variant_by_code(
        &self,
        _: &TenantContext,
        _: &str,
        _: Option<&str>,
    ) -> Result<Option<VariantSnapshot>, Error> {
        Ok(None)
    }
    async fn prices(&self, _: &TenantContext, _: &PriceQuery) -> Result<Vec<PriceSnapshot>, Error> {
        Ok(Vec::new())
    }
    async fn reference_prices(
        &self,
        _: &TenantContext,
        _: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error> {
        Ok(Vec::new())
    }
    async fn price_floors(
        &self,
        _: &TenantContext,
        _: OrgId,
        _: Currency,
        _: &[Uuid],
    ) -> Result<Vec<PriceFloor>, Error> {
        Ok(Vec::new())
    }
    async fn display_prices(
        &self,
        _: &TenantContext,
        product_ids: &[Uuid],
        variant_ids: &[Uuid],
    ) -> Result<DisplayPrices, Error> {
        let mut prices = DisplayPrices::default();
        let products = self.products.lock().unwrap();
        let variants = self.variant_prices.lock().unwrap();
        for id in product_ids {
            if let Some(price) = products.get(id) {
                prices.products.insert(*id, *price);
            }
        }
        for id in variant_ids {
            if let Some(price) = variants.get(id) {
                prices.variants.insert(*id, *price);
            }
        }
        Ok(prices)
    }
    async fn bundle_components(
        &self,
        _: &TenantContext,
        _: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<BundleComponent>>, Error> {
        Ok(HashMap::new())
    }
}

/// Tồn bán được theo biến thể; đăng ký báo có hàng của `inventory` theo (tài khoản, biến thể).
#[derive(Default)]
pub(crate) struct FakeInventory {
    pub available: Mutex<HashMap<Uuid, i64>>,
    pub own_alerts: Mutex<Vec<(Uuid, Uuid)>>,
}

#[async_trait::async_trait]
impl Inventory for FakeInventory {
    async fn availability(
        &self,
        _: &TenantContext,
        _: OrgId,
        variant_ids: &[Uuid],
    ) -> Result<Vec<Availability>, Error> {
        let available = self.available.lock().unwrap();
        Ok(variant_ids
            .iter()
            .filter_map(|id| {
                let amount = *available.get(id)?;
                // `Availability` là `#[non_exhaustive]`: dựng qua serde.
                serde_json::from_value(json!({
                    "variant_id": id, "warehouse_org_id": root_of(DEFAULT_TENANT),
                    "on_hand": amount, "available": amount, "incoming": 0,
                }))
                .ok()
            })
            .collect())
    }
    async fn reserve(
        &self,
        _: &TenantContext,
        _: &ReserveRequest,
    ) -> Result<Submitted<Reservation>, Error> {
        Err(Error::internal("not used"))
    }
    async fn confirm(&self, _: &TenantContext, _: Uuid) -> Result<Reservation, Error> {
        Err(Error::internal("not used"))
    }
    async fn release(
        &self,
        _: &TenantContext,
        _: Uuid,
        _: ReleaseReason,
    ) -> Result<Option<Reservation>, Error> {
        Ok(None)
    }
    async fn fulfil(&self, _: &TenantContext, _: &FulfilRequest) -> Result<Reservation, Error> {
        Err(Error::internal("not used"))
    }
    async fn restock(&self, _: &TenantContext, _: &RestockRequest) -> Result<(), Error> {
        Ok(())
    }
    async fn in_stock_at(
        &self,
        _: &TenantContext,
        _: OrgId,
        _: &[Uuid],
    ) -> Result<Vec<Uuid>, Error> {
        Ok(Vec::new())
    }
    async fn restock_alerts_of(
        &self,
        _: &TenantContext,
        user_id: Uuid,
        variant_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, Error> {
        let own = self.own_alerts.lock().unwrap();
        Ok(variant_ids.iter().copied().filter(|id| own.contains(&(user_id, *id))).collect())
    }
}
