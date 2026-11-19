//! Test của `orders` trên database gate đang chọn (gate chạy cả SQLite và Postgres). Dựng app như
//! binary bằng `TestApp` của testkit với `catalog`, `inventory`, `promotions`, `shipping` giả
//! (`fakes.rs`: biến thể, giá, tồn, mã giảm giá, cách giao đặt sẵn); gọi qua REST. Test với các
//! module thật nằm ở `crates/app/src/e2e/orders.rs`.

mod carts;
mod checkout;
mod fake_ledger;
mod fakes;
mod lifecycle;
mod pickups;
mod preorders;
mod returns;

pub(crate) use fake_ledger::FakeLedger;
pub(crate) use fakes::{FakeInventory, FakePromotions};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use autoxide_catalog::api::{
    BundleComponent, Catalog, PriceFloor, PriceQuery, PriceSnapshot, ProductKind, ProductStatus,
    SaleMode, VariantSnapshot,
};
use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Event, EventInfo, Outbox};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantContext};
use autoxide_shipping::api::{Quote, QuoteRequest, ShippingOption, ShippingRates};
use autoxide_testkit::{TestApp, TestAppBuilder, TestDatabase};
use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) fn vnd() -> Currency {
    Currency::parse("VND").unwrap()
}

pub(crate) fn money(minor: i64) -> Money {
    Money::new(minor, vnd())
}

/// Biến thể vật lý bán khi còn hàng, giá gồm thuế.
pub(crate) fn physical(id: Uuid, org: OrgId) -> VariantSnapshot {
    // `VariantSnapshot` là `#[non_exhaustive]` ở crate khác: dựng qua JSON.
    serde_json::from_value(json!({
        "variant_id": id, "product_id": Uuid::now_v7(), "org_id": org, "kind": ProductKind::Physical,
        "status": ProductStatus::Active, "variant_active": true, "sku": format!("SKU-{}", &id.simple().to_string()[24..]),
        "barcode": null, "name": "Item", "variant_label": "", "locale": "vi",
        "tax_category": "vat.standard", "category_ids": [], "weight_grams": 500,
        "length_mm": null, "width_mm": null, "height_mm": null, "requires_shipping": true,
        "track_inventory": true, "sale_mode": SaleMode::InStock, "preorder_available_on": null,
        "preorder_limit": null, "preorder_ends_at": null, "allow_backorder": false,
        "lot_tracked": false, "serial_tracked": false, "warranty_months": null,
        "image_media_id": null, "updated_at": Utc::now(),
    }))
    .unwrap()
}

/// `dyn Catalog` giả: biến thể và giá (gồm thuế) đặt sẵn.
#[derive(Default)]
pub(crate) struct FakeCatalog {
    pub variants: Mutex<HashMap<Uuid, VariantSnapshot>>,
    pub prices: Mutex<HashMap<Uuid, Money>>,
}

impl FakeCatalog {
    /// Thêm biến thể của nút `org` giá `price`, sửa thêm bằng `edit`.
    pub(crate) fn add_with(
        &self,
        org: OrgId,
        price: i64,
        edit: impl FnOnce(&mut VariantSnapshot),
    ) -> Uuid {
        let id = Uuid::now_v7();
        let mut variant = physical(id, org);
        edit(&mut variant);
        self.variants.lock().unwrap().insert(id, variant);
        self.prices.lock().unwrap().insert(id, money(price));
        id
    }

    pub(crate) fn add(&self, org: OrgId, price: i64) -> Uuid {
        self.add_with(org, price, |_| {})
    }

    pub(crate) fn set_price(&self, id: Uuid, price: i64) {
        self.prices.lock().unwrap().insert(id, money(price));
    }

    fn price(&self, id: Uuid, at: chrono::DateTime<Utc>) -> Option<PriceSnapshot> {
        let amount = *self.prices.lock().unwrap().get(&id)?;
        serde_json::from_value(json!({
            "variant_id": id, "price_list_id": Uuid::nil(), "price_id": Uuid::nil(),
            "amount": amount, "tax_inclusive": true, "compare_at": null,
            "valid_from": at - Duration::days(60), "valid_to": null,
        }))
        .ok()
    }
}

#[async_trait::async_trait]
impl Catalog for FakeCatalog {
    async fn variants(
        &self,
        _tenant: &TenantContext,
        ids: &[Uuid],
        _locale: Option<&str>,
    ) -> Result<Vec<VariantSnapshot>, Error> {
        let variants = self.variants.lock().unwrap();
        Ok(ids.iter().filter_map(|id| variants.get(id).cloned()).collect())
    }

    async fn variant_by_code(
        &self,
        _tenant: &TenantContext,
        code: &str,
        _locale: Option<&str>,
    ) -> Result<Option<VariantSnapshot>, Error> {
        let variants = self.variants.lock().unwrap();
        Ok(variants.values().find(|variant| variant.sku == code).cloned())
    }

    async fn prices(
        &self,
        _tenant: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error> {
        Ok(query.variant_ids.iter().filter_map(|id| self.price(*id, query.at)).collect())
    }

    async fn reference_prices(
        &self,
        _tenant: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error> {
        Ok(query.variant_ids.iter().filter_map(|id| self.price(*id, query.at)).collect())
    }

    async fn price_floors(
        &self,
        _tenant: &TenantContext,
        _org_id: OrgId,
        _currency: Currency,
        _variant_ids: &[Uuid],
    ) -> Result<Vec<PriceFloor>, Error> {
        Ok(Vec::new())
    }

    async fn bundle_components(
        &self,
        _tenant: &TenantContext,
        _variant_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<BundleComponent>>, Error> {
        Ok(HashMap::new())
    }
}

/// `dyn ShippingRates` giả: một cách giao `standard` phí `fee`, miễn phí từ `free_over`.
pub(crate) struct FakeShipping {
    pub fee: Mutex<i64>,
    pub free_over: Option<i64>,
}

impl FakeShipping {
    fn option(&self, request: &QuoteRequest) -> ShippingOption {
        let fee = *self.fee.lock().unwrap();
        let free = self.free_over.is_some_and(|over| request.order_value.minor() >= over);
        serde_json::from_value(json!({
            "method_id": Uuid::nil(), "method_code": "standard", "name": "Standard",
            "kind": "table_rate", "amount": money(if free { 0 } else { fee }),
            "free_shipping_applied": free, "free_over": self.free_over.map(money),
            "tax_category": "vat.standard", "carrier_code": "fake", "service_code": null,
            "pickup_org_id": null, "chargeable_weight_grams": 500, "estimated_days": [1, 3],
            "eta": null, "ready_at": null, "hold_days": null, "option_key": "standard",
        }))
        .unwrap()
    }
}

#[async_trait::async_trait]
impl ShippingRates for FakeShipping {
    async fn quote(&self, _tenant: &TenantContext, request: &QuoteRequest) -> Result<Quote, Error> {
        let options = [serde_json::to_value(self.option(request)).unwrap(), store_option(request)];
        Ok(serde_json::from_value(json!({"options": options, "unavailable": []})).unwrap())
    }

    async fn confirm_option(
        &self,
        _tenant: &TenantContext,
        request: &QuoteRequest,
        option_key: &str,
    ) -> Result<ShippingOption, Error> {
        if option_key == "store" {
            return Ok(serde_json::from_value(store_option(request)).unwrap());
        }
        Ok(self.option(request))
    }
}

/// Nhận tại chính cửa hàng bán, miễn phí, giữ hàng 2 ngày.
fn store_option(request: &QuoteRequest) -> Value {
    json!({
        "method_id": Uuid::from_u128(7), "method_code": "store", "name": "In store",
        "kind": "pickup", "amount": money(0), "free_shipping_applied": false, "free_over": null,
        "tax_category": "vat.standard", "carrier_code": null, "service_code": null,
        "pickup_org_id": request.org_id, "chargeable_weight_grams": 0, "estimated_days": null,
        "eta": null, "ready_at": null, "hold_days": 2, "option_key": "store",
    })
}

/// Cây gốc → chuỗi → cửa hàng A, B.
pub(crate) struct Tree {
    pub chain: OrgId,
    pub a: OrgId,
    pub b: OrgId,
}

/// App đã dựng cùng bộ giả.
pub(crate) struct Shop {
    pub app: TestApp,
    pub catalog: Arc<FakeCatalog>,
    pub inventory: Arc<FakeInventory>,
    pub promotions: Arc<FakePromotions>,
    pub shipping: Arc<FakeShipping>,
    pub ledger: Arc<FakeLedger>,
    /// Outbox của module giả `ledger` (phát `ledger.payment_succeeded`).
    pub ledger_outbox: Arc<Mutex<Option<Outbox>>>,
    /// Outbox của module giả `shipping` (phát event vận đơn).
    pub shipping_outbox: Arc<Mutex<Option<Outbox>>>,
    /// Outbox của module giả `catalog` và `inventory` (event hàng đặt trước).
    pub catalog_outbox: Arc<Mutex<Option<Outbox>>>,
    pub inventory_outbox: Arc<Mutex<Option<Outbox>>>,
    pub orgs: Arc<FakeOrgs>,
    pub media: Arc<fakes::FakeMedia>,
    pub tree: Tree,
}

impl std::ops::Deref for Shop {
    type Target = TestApp;
    fn deref(&self) -> &TestApp {
        &self.app
    }
}

/// Người dùng của app thử: `boss` (mọi quyền), `staff_a` (`orders.read` ở cửa hàng A), người mua
/// `buyer1`, `buyer2`.
pub(crate) async fn shop_with<'a>(
    test: &'a TestDatabase,
    extra: impl FnOnce(TestAppBuilder<'a>, &Tree) -> TestAppBuilder<'a>,
) -> Shop {
    let org = || OrgId::from_uuid(Uuid::now_v7());
    let tree = Tree { chain: org(), a: org(), b: org() };
    let catalog = Arc::new(FakeCatalog::default());
    let shipping = Arc::new(FakeShipping { fee: Mutex::new(30_000), free_over: Some(500_000) });
    let inventory = Arc::new(FakeInventory::default());
    let promotions = Arc::new(FakePromotions::default());
    let ledger = Arc::new(FakeLedger::default());
    let (emitter, ledger_outbox) = Emitter::new("ledger", &[PAYMENT_SUCCEEDED]);
    let (shipping_emitter, shipping_outbox) = Emitter::new("shipping", &SHIPPING_EVENTS);
    let (catalog_emitter, catalog_outbox) = Emitter::new(
        "catalog",
        &[
            crate::preorders::PREORDER_DATE_CHANGED,
            autoxide_catalog::api::events::VARIANT_UPDATED,
            autoxide_catalog::api::events::VARIANT_DELETED,
        ],
    );
    let (inventory_emitter, inventory_outbox) =
        Emitter::new("inventory", &[crate::preorders::PREORDER_ALLOCATED]);
    let orgs = Arc::new(FakeOrgs::default());
    let media = Arc::new(fakes::FakeMedia::default());
    let builder = TestApp::builder(test)
        .principal("boss", Principal::with_all_permissions(Uuid::from_u128(1)))
        .member("staff_a", 2, tree.a, &[("orders.read", autoxide_kernel::access::Scope::Unit)])
        .principal("buyer1", Principal::customer(Uuid::from_u128(101)))
        .principal("buyer2", Principal::customer(Uuid::from_u128(102)))
        .provide::<dyn Catalog>(catalog.clone())
        .provide::<dyn ShippingRates>(shipping.clone())
        .provide::<dyn autoxide_inventory::api::Inventory>(inventory.clone())
        .provide::<dyn autoxide_promotions::api::Promotions>(promotions.clone())
        .provide::<dyn autoxide_ledger::api::Ledger>(ledger.clone())
        .provide::<dyn autoxide_organizations::api::OrgDirectory>(orgs.clone())
        .provide::<dyn autoxide_media::api::Media>(media.clone())
        .module(emitter)
        .module(shipping_emitter)
        .module(catalog_emitter)
        .module(inventory_emitter);
    let app = extra(builder, &tree).module(crate::module()).build().await.unwrap();
    let write = app.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.chain, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.a, tree.chain, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.b, tree.chain, 8).await.unwrap();
    write.commit().await.unwrap();
    Shop {
        app,
        catalog,
        inventory,
        promotions,
        shipping,
        ledger,
        ledger_outbox,
        shipping_outbox,
        catalog_outbox,
        inventory_outbox,
        orgs,
        media,
        tree,
    }
}

pub(crate) async fn shop(test: &TestDatabase) -> Shop {
    shop_with(test, |builder, _| builder).await
}

/// Một lời gọi REST: `who` là token người dùng (hay `None`), `cart_token` là `Cart-Token`.
pub(crate) struct Call<'a> {
    pub app: &'a TestApp,
    pub who: Option<&'a str>,
    pub cart_token: Option<&'a str>,
}

impl<'a> Call<'a> {
    pub(crate) fn as_user(app: &'a TestApp, who: &'a str) -> Self {
        Call { app, who: Some(who), cart_token: None }
    }

    pub(crate) fn as_guest(app: &'a TestApp, cart_token: Option<&'a str>) -> Self {
        Call { app, who: None, cart_token }
    }

    /// Gọi `/api/v1/orders{uri}`; trả trạng thái và thân JSON (`null` khi rỗng).
    pub(crate) async fn send(
        &self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let method = Method::from_bytes(method.as_bytes()).unwrap();
        let mut request = self.app.request(method, &format!("/api/v1/orders{uri}"));
        if let Some(who) = self.who {
            request = request.token(who);
        }
        if let Some(token) = self.cart_token {
            request = request.header("Cart-Token", token);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let answer = request.send().await.unwrap();
        let status = answer.status();
        (status, if answer.body().is_empty() { Value::Null } else { answer.json() })
    }
}

/// Chạy hết event và job đang chờ.
pub(crate) async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..10 {
        let events = dispatcher.run_once().await.unwrap();
        if worker.run_once().await.unwrap() + events == 0 {
            break;
        }
    }
}

/// Số tiền (đơn vị nhỏ nhất) của trường tiền `value`.
pub(crate) fn minor(value: &Value) -> i64 {
    value["amount"].as_str().unwrap().parse().unwrap()
}

/// Tên event thanh toán xong của `ledger`.
pub(crate) const PAYMENT_SUCCEEDED: &str = "ledger.payment_succeeded";

/// `ledger.payment_succeeded` do module giả phát (thân phẳng như `ledger` thật).
#[derive(Debug, serde::Serialize)]
#[serde(transparent)]
pub(crate) struct PaymentSucceeded(pub Value);

impl Event for PaymentSucceeded {
    const NAME: &'static str = PAYMENT_SUCCEEDED;
}

/// Module giả phát event thay `ledger`: khai tên event, giữ `Outbox` sau `start`.
pub(crate) struct Emitter {
    name: &'static str,
    events: Vec<&'static str>,
    outbox: Arc<Mutex<Option<Outbox>>>,
}

impl Emitter {
    pub(crate) fn new(
        name: &'static str,
        events: &[&'static str],
    ) -> (Self, Arc<Mutex<Option<Outbox>>>) {
        let outbox = Arc::new(Mutex::new(None));
        (Emitter { name, events: events.to_vec(), outbox: outbox.clone() }, outbox)
    }
}

impl Module for Emitter {
    fn name(&self) -> &'static str {
        self.name
    }
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            autoxide_kernel::event::outbox_migration("m0001_events", self.name),
            autoxide_kernel::event::outbox_actor_migration("m0002_event_actor", self.name),
            autoxide_kernel::tenancy::migration::adopt(
                "m0003_tenancy_events",
                vec![autoxide_kernel::event::tenant_table(self.name)],
            ),
            autoxide_kernel::event::outbox_access_migration("k0001_event_access", self.name),
        ]
    }
    fn events(&self) -> Vec<EventInfo> {
        self.events.iter().map(|name| EventInfo::new(name, "Test event.")).collect()
    }
    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        *self.outbox.lock().unwrap() = Some(context.outbox()?);
        Ok(())
    }
}

/// Ghi `event` vào outbox của `ledger` giả (một transaction riêng).
pub(crate) async fn emit<E: Event>(shop: &Shop, event: &E) {
    emit_from(shop, &shop.ledger_outbox, event).await;
}

/// Ghi `event` vào outbox của module giả `outbox` (một transaction riêng).
pub(crate) async fn emit_from<E: Event>(
    shop: &Shop,
    outbox: &Arc<Mutex<Option<Outbox>>>,
    event: &E,
) {
    let outbox = outbox.lock().unwrap().clone().unwrap();
    let tenant = shop.default_tenant();
    let transaction = shop.database().begin_write().await.unwrap();
    outbox.record(&transaction, &tenant, Actor::system(), None, event).await.unwrap();
    transaction.commit().await.unwrap();
}

/// Job định kỳ chạy ngay: mỗi loại một kiểu (`Job::KIND` là hằng).
macro_rules! now_job {
    ($kind:ident, $name:expr) => {
        #[derive(Debug, serde::Serialize)]
        struct $kind {}

        impl autoxide_kernel::job::Job for $kind {
            const KIND: &'static str = $name;
        }
    };
}

now_job!(MarkAbandonedNow, crate::jobs::MARK_ABANDONED_CARTS);
now_job!(ExpireUnpaidNow, crate::jobs::EXPIRE_UNPAID);
now_job!(RecoverNow, crate::jobs::RECOVER_CHECKOUTS);
now_job!(CompleteNow, crate::jobs::COMPLETE_DELIVERED);
now_job!(UnacceptedNow, crate::jobs::EXPIRE_UNACCEPTED);
now_job!(PickupsNow, crate::jobs::EXPIRE_PICKUPS);

/// Đưa job định kỳ `kind` vào hàng đợi của cả database rồi chạy hết.
pub(crate) async fn run_job(app: &TestApp, kind: &'static str) {
    let queue = autoxide_kernel::job::JobQueue::new("orders");
    let options = autoxide_kernel::job::EnqueueOptions::default;
    let write = app.database().write();
    match kind {
        crate::jobs::MARK_ABANDONED_CARTS => {
            queue.enqueue_database(write, &MarkAbandonedNow {}, options()).await
        }
        crate::jobs::EXPIRE_UNPAID => {
            queue.enqueue_database(write, &ExpireUnpaidNow {}, options()).await
        }
        crate::jobs::RECOVER_CHECKOUTS => {
            queue.enqueue_database(write, &RecoverNow {}, options()).await
        }
        crate::jobs::COMPLETE_DELIVERED => {
            queue.enqueue_database(write, &CompleteNow {}, options()).await
        }
        crate::jobs::EXPIRE_UNACCEPTED => {
            queue.enqueue_database(write, &UnacceptedNow {}, options()).await
        }
        crate::jobs::EXPIRE_PICKUPS => {
            queue.enqueue_database(write, &PickupsNow {}, options()).await
        }
        other => panic!("no test job for `{other}`"),
    }
    .unwrap();
    drain(app).await;
}

/// Event vận đơn của `shipping` mà `orders` nghe.
pub(crate) const SHIPPING_EVENTS: [&str; 5] = [
    autoxide_shipping::api::events::SHIPMENT_STATUS_CHANGED,
    autoxide_shipping::api::events::SHIPMENT_DELIVERED,
    autoxide_shipping::api::events::SHIPMENT_DELIVERY_FAILED,
    autoxide_shipping::api::events::SHIPMENT_RETURNED,
    autoxide_shipping::api::events::COD_COLLECTED,
];

/// Event vận đơn do module giả `shipping` phát: mỗi tên một kiểu (`Event::NAME` là hằng).
macro_rules! shipping_event {
    ($kind:ident, $name:expr) => {
        #[derive(Debug, serde::Serialize)]
        #[serde(transparent)]
        pub(crate) struct $kind(pub Value);

        impl Event for $kind {
            const NAME: &'static str = $name;
        }
    };
}

shipping_event!(ShipmentDelivered, autoxide_shipping::api::events::SHIPMENT_DELIVERED);
shipping_event!(CodCollected, autoxide_shipping::api::events::COD_COLLECTED);
shipping_event!(PreorderAllocated, crate::preorders::PREORDER_ALLOCATED);
shipping_event!(PreorderDateChanged, crate::preorders::PREORDER_DATE_CHANGED);
shipping_event!(VariantUpdated, autoxide_catalog::api::events::VARIANT_UPDATED);

/// Thân `{"after": ShipmentEvent}` của event vận đơn cho đơn `order`.
pub(crate) fn shipment_event(order: uuid::Uuid, status: &str, cod: Option<i64>) -> Value {
    keyed_shipment_event(order, &format!("order:{order}:1"), status, cod)
}

/// Event vận đơn của lần giao mang khoá `request_key` (khoá `orders` đã gửi `shipping`).
pub(crate) fn keyed_shipment_event(
    order: uuid::Uuid,
    request_key: &str,
    status: &str,
    cod: Option<i64>,
) -> Value {
    json!({"after": {
        "shipment_id": uuid::Uuid::now_v7(), "order_id": order, "request_key": request_key,
        "direction": "outbound", "org_id": uuid::Uuid::nil(), "status": status,
        "tracking_number": "T1", "carrier_code": "fake", "cod_amount": cod.map(money),
        "at": Utc::now(),
    }})
}

/// `dyn OrgDirectory` giả: cài đặt theo nút đặt sẵn (không kế thừa), còn lại mặc định.
#[derive(Default)]
pub(crate) struct FakeOrgs {
    settings: Mutex<HashMap<(OrgId, String), Value>>,
}

impl FakeOrgs {
    pub(crate) fn set(&self, org: OrgId, key: &str, value: Value) {
        self.settings.lock().unwrap().insert((org, key.to_owned()), value);
    }
}

#[async_trait::async_trait]
impl autoxide_organizations::api::OrgDirectory for FakeOrgs {
    async fn unit(
        &self,
        _: &TenantContext,
        _: OrgId,
    ) -> Result<Option<autoxide_organizations::api::Unit>, Error> {
        Ok(None)
    }
    async fn root(
        &self,
        tenant: &TenantContext,
    ) -> Result<autoxide_organizations::api::Unit, Error> {
        let root = autoxide_kernel::tenancy::root_of(tenant.id());
        Ok(autoxide_organizations::api::Unit::new(root, None, "root", "Root"))
    }
    async fn ancestors(
        &self,
        _: &TenantContext,
        _: OrgId,
    ) -> Result<Vec<autoxide_organizations::api::Unit>, Error> {
        Ok(Vec::new())
    }
    async fn is_within(&self, _: &TenantContext, _: OrgId, _: OrgId) -> Result<bool, Error> {
        Ok(true)
    }
    async fn setting(
        &self,
        _: &TenantContext,
        id: OrgId,
        key: &str,
    ) -> Result<Option<autoxide_organizations::api::EffectiveSetting>, Error> {
        let value = self.settings.lock().unwrap().get(&(id, key.to_owned())).cloned();
        Ok(value.map(|value| {
            autoxide_organizations::api::EffectiveSetting::new(key, Some(value), Some(id), false)
        }))
    }
}
