//! Test của `inventory`, trên database gate đang chọn (gate chạy cả SQLite và Postgres). Dựng app
//! như binary bằng `TestApp` của testkit; `dyn Catalog`, `dyn OrgDirectory` là bộ giả (module khác
//! chỉ dùng qua `api`).

mod documents;
mod levels;
mod lots;
mod orders;
mod preorders;
mod public;
mod purchasing;
mod reservations;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use autoxide_catalog::api::{
    BundleComponent, Catalog, PriceFloor, PriceQuery, PriceSnapshot, ProductKind, VariantSnapshot,
};
use autoxide_kernel::access::Scope;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, EventInfo, Outbox};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::money::Currency;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantContext, root_of};
use autoxide_organizations::api::{EffectiveSetting, OrgDirectory, Unit};
use autoxide_testkit::{TestApp, TestAppBuilder, TestDatabase};
use axum::http::{Method, StatusCode};
use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

/// Mọi permission của module.
pub(crate) const ALL: [&str; 9] = [
    "inventory.read",
    "inventory.adjust",
    "inventory.receive",
    "inventory.transfer",
    "inventory.count",
    "inventory.reservations.read",
    "inventory.settings.manage",
    "inventory.purchasing.read",
    "inventory.purchasing.manage",
];

/// Danh mục giả: biến thể khai bằng [`FakeCatalog::add`].
#[derive(Default)]
pub(crate) struct FakeCatalog {
    variants: Mutex<HashMap<Uuid, VariantSnapshot>>,
    bundles: Mutex<HashMap<Uuid, Vec<BundleComponent>>>,
}

/// Biến thể vật lý đang bán, theo dõi tồn.
pub(crate) fn physical(id: Uuid) -> VariantSnapshot {
    let json = json!({
        "variant_id": id, "product_id": Uuid::now_v7(), "org_id": root_of(DEFAULT_TENANT),
        "kind": "physical", "status": "active", "variant_active": true,
        "sku": format!("SKU-{}", id.simple()), "barcode": null, "name": "Item", "variant_label": "",
        "locale": "vi", "tax_category": "vat.standard", "category_ids": [],
        "weight_grams": 100, "length_mm": null, "width_mm": null, "height_mm": null,
        "requires_shipping": true, "track_inventory": true, "sale_mode": "in_stock",
        "preorder_available_on": null, "preorder_limit": null, "preorder_ends_at": null,
        "allow_backorder": false, "lot_tracked": false, "serial_tracked": false,
        "warranty_months": null, "image_media_id": null, "updated_at": Utc::now(),
    });
    // `VariantSnapshot` là `#[non_exhaustive]`: crate khác dựng qua serde.
    serde_json::from_value(json).unwrap()
}

impl FakeCatalog {
    /// Thêm biến thể vật lý mới (sửa bằng `edit`); trả ID.
    pub(crate) fn add(&self, edit: impl FnOnce(&mut VariantSnapshot)) -> Uuid {
        let id = Uuid::now_v7();
        let mut variant = physical(id);
        edit(&mut variant);
        self.variants.lock().unwrap().insert(id, variant);
        id
    }

    /// Biến thể vật lý thường.
    pub(crate) fn item(&self) -> Uuid {
        self.add(|_| {})
    }

    /// Combo gồm các thành phần `(biến thể, số lượng)`.
    #[allow(dead_code)]
    pub(crate) fn bundle(&self, components: &[(Uuid, i64)]) -> Uuid {
        let id = self.add(|variant| {
            variant.kind = ProductKind::Bundle;
            variant.track_inventory = false;
        });
        let parts = components
            .iter()
            .map(|(variant_id, quantity)| BundleComponent {
                variant_id: *variant_id,
                quantity: *quantity,
            })
            .collect();
        self.bundles.lock().unwrap().insert(id, parts);
        id
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
        let variants = self.variants.lock().unwrap();
        Ok(ids.iter().filter_map(|id| variants.get(id).cloned()).collect())
    }
    async fn variant_by_code(
        &self,
        _: &TenantContext,
        code: &str,
        _: Option<&str>,
    ) -> Result<Option<VariantSnapshot>, Error> {
        Ok(self.variants.lock().unwrap().values().find(|variant| variant.sku == code).cloned())
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
    async fn bundle_components(
        &self,
        _: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<BundleComponent>>, Error> {
        let bundles = self.bundles.lock().unwrap();
        Ok(ids.iter().filter_map(|id| bundles.get(id).map(|parts| (*id, parts.clone()))).collect())
    }
}

/// Cây tổ chức giả: loại của từng nút (mặc định `warehouse`).
#[derive(Default)]
pub(crate) struct FakeOrgs {
    types: Mutex<HashMap<OrgId, String>>,
}

impl FakeOrgs {
    pub(crate) fn set_type(&self, org: OrgId, unit_type: &str) {
        self.types.lock().unwrap().insert(org, unit_type.to_owned());
    }
}

#[async_trait::async_trait]
impl OrgDirectory for FakeOrgs {
    async fn unit(&self, tenant: &TenantContext, id: OrgId) -> Result<Option<Unit>, Error> {
        if id == root_of(tenant.id()) {
            return Ok(Some(Unit::new(id, None, "root", "Root")));
        }
        let unit_type =
            self.types.lock().unwrap().get(&id).cloned().unwrap_or_else(|| "warehouse".to_owned());
        Ok(Some(Unit::new(id, Some(root_of(tenant.id())), unit_type, "Unit")))
    }
    async fn root(&self, tenant: &TenantContext) -> Result<Unit, Error> {
        Ok(Unit::new(root_of(tenant.id()), None, "root", "Root"))
    }
    async fn ancestors(&self, _: &TenantContext, _: OrgId) -> Result<Vec<Unit>, Error> {
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
    ) -> Result<Option<EffectiveSetting>, Error> {
        Ok(Some(EffectiveSetting::new(key, None, Some(id), false)))
    }
}

/// Cây gốc → chuỗi; chuỗi → kho A, kho B, cửa hàng S.
pub(crate) struct Tree {
    pub chain: OrgId,
    pub a: OrgId,
    pub b: OrgId,
    pub store: OrgId,
}

/// App đã dựng cùng bộ giả.
pub(crate) struct Shop {
    pub app: TestApp,
    pub catalog: Arc<FakeCatalog>,
    pub orgs: Arc<FakeOrgs>,
    pub tree: Tree,
}

impl std::ops::Deref for Shop {
    type Target = TestApp;
    fn deref(&self) -> &TestApp {
        &self.app
    }
}

/// App với `boss` (mọi quyền cả khách), `keeper_a` / `keeper_b` (mọi quyền ở kho A / B, độ sâu
/// `unit`); `extra` thêm người dùng / module.
pub(crate) async fn shop_with<'a>(
    test: &'a TestDatabase,
    extra: impl FnOnce(TestAppBuilder<'a>, &Tree) -> TestAppBuilder<'a>,
) -> Shop {
    let org = || OrgId::from_uuid(Uuid::now_v7());
    let tree = Tree { chain: org(), a: org(), b: org(), store: org() };
    let catalog = Arc::new(FakeCatalog::default());
    let orgs = Arc::new(FakeOrgs::default());
    orgs.set_type(tree.chain, "region");
    orgs.set_type(tree.store, "store");
    let scoped = |scope: Scope| ALL.map(|name| (name, scope));
    let builder = TestApp::builder(test)
        .member("boss", 1, root_of(DEFAULT_TENANT), &scoped(Scope::Tenant))
        .member("keeper_a", 2, tree.a, &scoped(Scope::Unit))
        .member("keeper_b", 3, tree.b, &scoped(Scope::Unit))
        .provide::<dyn Catalog>(catalog.clone())
        .provide::<dyn OrgDirectory>(orgs.clone());
    let app = extra(builder, &tree).module(crate::module()).build().await.unwrap();
    let write = app.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.chain, root, 8).await.unwrap();
    for unit in [tree.a, tree.b, tree.store] {
        OrgTree::insert(&write, DEFAULT_TENANT, unit, tree.chain, 8).await.unwrap();
    }
    write.commit().await.unwrap();
    Shop { app, catalog, orgs, tree }
}

pub(crate) async fn shop(test: &TestDatabase) -> Shop {
    shop_with(test, |builder, _| builder).await
}

/// Gọi `/api/v1/inventory{uri}`; trả trạng thái và thân JSON (`null` khi rỗng).
pub(crate) async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/inventory{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    (status, if answer.body().is_empty() { Value::Null } else { answer.json() })
}

/// Nhập `quantity` biến thể `variant` vào kho `org` bằng lệnh điều chỉnh `found` (giá `cost` VND
/// khi có); trả thân trả lời.
pub(crate) async fn stock_in(
    app: &TestApp,
    org: OrgId,
    variant: Uuid,
    quantity: i64,
    cost: Option<i64>,
) -> Value {
    let mut line = json!({"variant_id": variant, "quantity": quantity, "reason_code": "found"});
    if let Some(cost) = cost {
        line["unit_cost"] = json!({"amount": cost.to_string(), "currency": "VND"});
    }
    let body = json!({
        "org_id": org.as_uuid(),
        "reference": format!("in-{}", Uuid::now_v7().simple()),
        "lines": [line],
    });
    let (status, answer) = call(app, "POST", "/adjustments", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    answer
}

/// Dòng tồn của `variant` ở kho `org` (đọc bằng `boss`).
pub(crate) async fn level(app: &TestApp, org: OrgId, variant: Uuid) -> Value {
    let uri = format!("/levels?org_id={}&variant_id={variant}", org.as_uuid());
    let (status, found) = call(app, "GET", &uri, "boss", None).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    found["data"][0].clone()
}

/// Module giả phát event thay một module khác (`organizations`, `orders`…): khai tên event, giữ
/// `Outbox` sau `start` để test ghi event như module thật.
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

/// Event của module giả: tên đặt lúc chạy qua `Raw::NAME` không được (hằng), nên mỗi tên một kiểu.
macro_rules! raw_event {
    ($kind:ident, $name:expr) => {
        #[derive(Debug, serde::Serialize)]
        #[serde(transparent)]
        pub(crate) struct $kind(pub serde_json::Value);

        impl autoxide_kernel::event::Event for $kind {
            const NAME: &'static str = $name;
        }
    };
}
pub(crate) use raw_event;

/// Ghi `event` vào outbox của module giả (một transaction riêng).
pub(crate) async fn emit<E: autoxide_kernel::event::Event>(
    app: &TestApp,
    outbox: &Arc<Mutex<Option<Outbox>>>,
    subject: Option<String>,
    event: &E,
) {
    let outbox = outbox.lock().unwrap().clone().unwrap();
    let tenant = app.default_tenant();
    let transaction = app.database().begin_write().await.unwrap();
    outbox.record(&transaction, &tenant, Actor::system(), subject, event).await.unwrap();
    transaction.commit().await.unwrap();
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

/// Kiểm mọi dòng tồn khớp sổ, qua lệnh `inventory.verify_levels` như người vận hành chạy.
pub(crate) async fn assert_consistent(app: &TestApp) {
    let tasks = app.modules().tasks().unwrap();
    let (_, task) = tasks.iter().find(|(name, _)| name == "inventory.verify_levels").unwrap();
    let report = task.run(&task.parse(&[]).unwrap()).await.unwrap();
    assert!(report.contains("every stock level matches"), "{report}");
}

/// Tiêu chí xong của bước 42: không có đường ghi `on_hand` / `available` nào ngoài `rows::adjust`
/// (trong `stock.rs`), không có `UPDATE` / `DELETE` trên `inventory_movements`.
#[test]
fn so_chi_them_va_ton_chi_doi_bang_adjust() {
    fn sources(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if !path.ends_with("tests") {
                    sources(&path, out);
                }
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push((path.display().to_string(), std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let mut files = Vec::new();
    sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    assert!(files.len() > 5);
    for (path, text) in &files {
        for banned in [
            "movement::Entity::update",
            "movement::Entity::delete",
            "movement::ActiveModel::update",
        ] {
            assert!(
                !text.contains(banned),
                "{path}: `{banned}` — the movement ledger is append-only"
            );
        }
        if path.ends_with("stock.rs") {
            continue;
        }
        for banned in ["Column::OnHand,", "Column::Available,", "on_hand: Set(", "available: Set("]
        {
            // Dòng mới bắt đầu từ 0 là được; mọi giá trị khác phải đi qua `rows::adjust`.
            let bad = text.match_indices(banned).any(|(at, _)| {
                !banned.ends_with("Set(") || !text[at + banned.len()..].starts_with("0)")
            });
            assert!(
                !bad,
                "{path}: `{banned}` — change on hand / available only through `stock.rs` (`rows::adjust`)"
            );
        }
    }
}
