//! `Catalog` và `Inventory` giả cho báo yêu thích: giá công khai và tồn đặt tay, đăng ký báo có hàng
//! của `inventory` theo tài khoản.

use std::collections::HashMap;
use std::sync::Mutex;

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

pub(crate) use autoxide_catalog::api::testing::FakeCatalog;

/// Cách `customers` thêm hàng vào catalog giả dùng chung: sản phẩm một biến thể, giá công khai.
pub(crate) trait CatalogExt {
    /// Sản phẩm có một biến thể, giá `price`; trả (sản phẩm, biến thể).
    fn product(&self, price: i64) -> (Uuid, Uuid);
    /// Đổi giá công khai của biến thể (và vì thế của sản phẩm một biến thể).
    fn reprice(&self, variant: Uuid, price: i64);
}

impl CatalogExt for FakeCatalog {
    fn product(&self, price: i64) -> (Uuid, Uuid) {
        let id = Uuid::now_v7();
        let mut variant = autoxide_catalog::api::testing::variant(id, root_of(DEFAULT_TENANT));
        variant.sku = format!("SKU-{}", id.simple());
        variant.weight_grams = Some(100);
        let product = variant.product_id;
        self.insert_variant(variant);
        self.reprice(id, price);
        (product, id)
    }

    fn reprice(&self, variant: Uuid, price: i64) {
        self.set_price(variant, vnd(price));
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
