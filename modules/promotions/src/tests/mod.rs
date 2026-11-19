//! Test của `promotions` trên database gate đang chọn (gate chạy cả SQLite và Postgres). Dựng app
//! như binary bằng `TestApp` của testkit, với `dyn Catalog` giả (biến thể, giá tham chiếu, sàn giá
//! đặt sẵn); gọi qua REST, MCP và `dyn Promotions`.

mod evaluate;
mod insights;
mod participations;
mod programs;
mod redemption;
mod vouchers;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use autoxide_catalog::api::{
    BundleComponent, Catalog, FloorAction, PriceFloor, PriceQuery, PriceSnapshot, ProductKind,
    ProductStatus, SaleMode, VariantSnapshot,
};
use autoxide_kernel::access::Scope;
use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantContext, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{Cart, CartLine, Promotions};

pub(crate) const STAFF: [&str; 8] = [
    "promotions.read",
    "promotions.write",
    "promotions.publish",
    "promotions.codes.manage",
    "promotions.compliance",
    "promotions.vouchers.grant",
    "promotions.funding.manage",
    "promotions.participate",
];

pub(crate) fn vnd() -> Currency {
    Currency::parse("VND").unwrap()
}

pub(crate) fn money(minor: i64) -> Money {
    Money::new(minor, vnd())
}

/// (sản phẩm, nút sở hữu, danh mục) của biến thể giả.
pub(crate) type FakeVariant = (Uuid, OrgId, Vec<Uuid>);
/// Lịch sử giá: (từ lúc, giá).
pub(crate) type PriceHistory = Vec<(DateTime<Utc>, Money)>;

/// `dyn Catalog` giả: biến thể và lịch sử giá đặt sẵn.
#[derive(Default)]
pub(crate) struct FakeCatalog {
    pub variants: Mutex<HashMap<Uuid, FakeVariant>>,
    /// Lịch sử giá mỗi biến thể: (từ lúc, giá).
    pub prices: Mutex<HashMap<Uuid, PriceHistory>>,
    pub floors: Mutex<HashMap<Uuid, (Money, FloorAction)>>,
    /// Nguồn giá giảm tự động promotions cắm vào (`CatalogExtensions`).
    pub sale_source: Mutex<Option<Arc<dyn autoxide_catalog::api::SalePriceSource>>>,
}

impl autoxide_catalog::api::CatalogExtensions for FakeCatalog {
    fn set_sale_price_source(
        &self,
        source: Arc<dyn autoxide_catalog::api::SalePriceSource>,
    ) -> bool {
        let mut slot = self.sale_source.lock().unwrap();
        if slot.is_some() {
            return false;
        }
        *slot = Some(source);
        true
    }
}

impl FakeCatalog {
    /// Thêm biến thể của nút `org` trong các danh mục `categories`, giá `price` từ 30 ngày trước.
    pub(crate) fn add(&self, org: OrgId, price: i64, categories: &[Uuid]) -> Uuid {
        let variant = Uuid::now_v7();
        self.variants.lock().unwrap().insert(variant, (Uuid::now_v7(), org, categories.to_vec()));
        self.prices
            .lock()
            .unwrap()
            .insert(variant, vec![(Utc::now() - Duration::days(30), money(price))]);
        variant
    }

    fn snapshot(&self, id: Uuid) -> Option<VariantSnapshot> {
        let variants = self.variants.lock().unwrap();
        let (product, org, categories) = variants.get(&id)?.clone();
        // `VariantSnapshot` là `#[non_exhaustive]` ở crate khác: dựng qua JSON.
        serde_json::from_value(json!({
            "variant_id": id, "product_id": product, "org_id": org, "kind": ProductKind::Physical,
            "status": ProductStatus::Active, "variant_active": true, "sku": id.to_string(),
            "barcode": null, "name": "Item", "variant_label": "", "locale": "vi",
            "tax_category": "vat.standard", "category_ids": categories, "weight_grams": null,
            "length_mm": null, "width_mm": null, "height_mm": null, "requires_shipping": true,
            "track_inventory": true, "sale_mode": SaleMode::InStock, "preorder_available_on": null,
            "preorder_limit": null, "preorder_ends_at": null, "allow_backorder": false,
            "lot_tracked": false, "serial_tracked": false, "warranty_months": null,
            "image_media_id": null, "updated_at": Utc::now(),
        }))
        .ok()
    }

    fn price(&self, id: Uuid, at: DateTime<Utc>, before: bool) -> Option<PriceSnapshot> {
        let prices = self.prices.lock().unwrap();
        let (from, amount) = prices
            .get(&id)?
            .iter()
            .filter(|(from, _)| if before { *from < at } else { *from <= at })
            .max_by_key(|(from, _)| *from)?
            .to_owned();
        serde_json::from_value(json!({
            "variant_id": id, "price_list_id": Uuid::nil(), "price_id": Uuid::nil(),
            "amount": amount, "tax_inclusive": true, "compare_at": null, "valid_from": from,
            "valid_to": null,
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
        Ok(ids.iter().filter_map(|id| self.snapshot(*id)).collect())
    }

    async fn variant_by_code(
        &self,
        _tenant: &TenantContext,
        _code: &str,
        _locale: Option<&str>,
    ) -> Result<Option<VariantSnapshot>, Error> {
        Ok(None)
    }

    async fn prices(
        &self,
        _tenant: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error> {
        Ok(query.variant_ids.iter().filter_map(|id| self.price(*id, query.at, false)).collect())
    }

    async fn reference_prices(
        &self,
        _tenant: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error> {
        Ok(query.variant_ids.iter().filter_map(|id| self.price(*id, query.at, true)).collect())
    }

    async fn price_floors(
        &self,
        _tenant: &TenantContext,
        _org_id: OrgId,
        _currency: Currency,
        variant_ids: &[Uuid],
    ) -> Result<Vec<PriceFloor>, Error> {
        let floors = self.floors.lock().unwrap();
        Ok(variant_ids
            .iter()
            .filter_map(|id| {
                let (floor, action) = floors.get(id)?;
                serde_json::from_value(json!({
                    "variant_id": id, "guard_id": Uuid::nil(), "min_price": floor,
                    "min_margin_bp": null, "unit_cost": null, "floor": floor, "action": action,
                }))
                .ok()
            })
            .collect())
    }

    async fn bundle_components(
        &self,
        _tenant: &TenantContext,
        _variant_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<BundleComponent>>, Error> {
        Ok(HashMap::new())
    }
}

/// Cây gốc → chuỗi → cửa hàng A, B.
pub(crate) struct Tree {
    pub chain: OrgId,
    pub a: OrgId,
    pub b: OrgId,
}

pub(crate) struct Setup {
    pub app: TestApp,
    pub tree: Tree,
    pub catalog: Arc<FakeCatalog>,
    pub notifier: Arc<FakeNotifier>,
    pub consents: Arc<FakeLedger>,
}

/// `dyn Notifier` giả: giữ mọi thông báo đã gửi; cùng `dedupe_key` không giữ hai lần.
#[derive(Default)]
pub(crate) struct FakeNotifier(pub Mutex<Vec<autoxide_notifications::api::Notification>>);

#[async_trait::async_trait]
impl autoxide_notifications::api::Notifier for FakeNotifier {
    fn declare(&self, _kind: autoxide_notifications::api::NotificationKind) -> Result<(), Error> {
        Ok(())
    }

    async fn notify(
        &self,
        _tenant: autoxide_kernel::tenancy::TenantId,
        notification: autoxide_notifications::api::Notification,
    ) -> Result<autoxide_notifications::api::Outcome, Error> {
        let mut sent = self.0.lock().unwrap();
        if !sent.iter().any(|found| found.dedupe_key == notification.dedupe_key) {
            sent.push(notification);
        }
        Ok(autoxide_notifications::api::Outcome::default())
    }
}

/// Sổ đồng ý giả: (khoá, `source_ref`, còn hiệu lực).
#[derive(Default)]
pub(crate) struct FakeLedger(pub Mutex<Vec<(autoxide_compliance::api::ConsentKey, String, bool)>>);

impl FakeLedger {
    fn consent(
        key: &autoxide_compliance::api::ConsentKey,
        granted: bool,
    ) -> autoxide_compliance::api::Consent {
        autoxide_compliance::api::Consent {
            id: Uuid::now_v7(),
            purpose: key.purpose.clone(),
            scope: key.scope.clone(),
            channel: key.channel.clone(),
            state: if granted { "granted" } else { "withdrawn" }.to_owned(),
            granted_at: Some(Utc::now()),
            withdrawn_at: (!granted).then(Utc::now),
            subject_value: None,
        }
    }
}

#[async_trait::async_trait]
impl autoxide_compliance::api::ConsentLedger for FakeLedger {
    async fn grant(
        &self,
        _tenant: autoxide_kernel::tenancy::TenantId,
        _subject: autoxide_compliance::api::ConsentSubject,
        key: autoxide_compliance::api::ConsentKey,
        _evidence: autoxide_compliance::api::ConsentEvidence,
        source_ref: &str,
    ) -> Result<autoxide_compliance::api::Consent, Error> {
        self.0.lock().unwrap().push((key.clone(), source_ref.to_owned(), true));
        Ok(Self::consent(&key, true))
    }

    async fn withdraw(
        &self,
        _tenant: autoxide_kernel::tenancy::TenantId,
        _subject: autoxide_compliance::api::ConsentSubject,
        key: autoxide_compliance::api::ConsentKey,
        _source: &str,
        source_ref: &str,
    ) -> Result<autoxide_compliance::api::Consent, Error> {
        for row in self.0.lock().unwrap().iter_mut() {
            if row.0 == key && row.1 == source_ref {
                row.2 = false;
            }
        }
        Ok(Self::consent(&key, false))
    }

    async fn is_granted(
        &self,
        _tenant: autoxide_kernel::tenancy::TenantId,
        _subject: &autoxide_compliance::api::ConsentSubject,
        key: &autoxide_compliance::api::ConsentKey,
    ) -> Result<bool, Error> {
        Ok(self.0.lock().unwrap().iter().any(|row| &row.0 == key && row.2))
    }

    async fn granted_many(
        &self,
        _tenant: autoxide_kernel::tenancy::TenantId,
        subjects: &[autoxide_compliance::api::ConsentSubject],
        _key: &autoxide_compliance::api::ConsentKey,
    ) -> Result<Vec<bool>, Error> {
        Ok(vec![false; subjects.len()])
    }

    async fn get(
        &self,
        _tenant: autoxide_kernel::tenancy::TenantId,
        _consent_id: Uuid,
    ) -> Result<Option<autoxide_compliance::api::Consent>, Error> {
        Ok(None)
    }
}

impl Setup {
    pub(crate) fn promotions(&self) -> Arc<dyn Promotions> {
        self.app.api::<dyn Promotions>().unwrap()
    }

    pub(crate) fn tenant(&self) -> TenantContext {
        self.app.default_tenant()
    }

    /// Giỏ ở cửa hàng A với các dòng (biến thể, số lượng), giá lấy từ catalog giả.
    pub(crate) fn cart(&self, lines: &[(Uuid, i64)]) -> Cart {
        let mut cart = Cart::new(self.tree.a, vnd(), Utc::now());
        for (index, (variant, quantity)) in lines.iter().enumerate() {
            let price = self.catalog.price(*variant, Utc::now(), false).unwrap().amount;
            cart.lines.push(CartLine::new(format!("l{index}"), *variant, *quantity, price));
        }
        cart
    }
}

/// Dựng app; `legal`: nạp gói luật "vn".
pub(crate) async fn setup(test: &TestDatabase, legal: bool) -> Setup {
    let org = || OrgId::from_uuid(Uuid::now_v7());
    let tree = Tree { chain: org(), a: org(), b: org() };
    let scoped = |scope: Scope| STAFF.map(|name| (name, scope));
    let catalog = Arc::new(FakeCatalog::default());
    let notifier = Arc::new(FakeNotifier::default());
    let consents = Arc::new(FakeLedger::default());
    let mut builder = TestApp::builder(test)
        .member("boss", 1, root_of(DEFAULT_TENANT), &scoped(Scope::Tenant))
        .member("store_a", 2, tree.a, &scoped(Scope::Unit))
        .member("store_b", 3, tree.b, &scoped(Scope::Unit))
        .provide::<dyn Catalog>(catalog.clone())
        .provide::<dyn autoxide_catalog::api::CatalogExtensions>(catalog.clone())
        .provide::<dyn autoxide_notifications::api::Notifier>(notifier.clone())
        .provide::<dyn autoxide_compliance::api::ConsentLedger>(consents.clone());
    for buyer in 0..4u128 {
        builder = builder.user(&format!("buyer{buyer}"), 100 + buyer, &["promotions.use"]);
    }
    let app = builder.module(crate::module()).build().await.unwrap();
    let write = app.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.chain, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.a, tree.chain, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.b, tree.chain, 8).await.unwrap();
    write.commit().await.unwrap();
    if legal {
        load_legal(&app).await;
    }
    Setup { app, tree, catalog, notifier, consents }
}

/// `autoxide task promotions.load_legal_data`.
pub(crate) async fn load_legal(app: &TestApp) -> String {
    let tasks = app.modules().tasks().unwrap();
    let (_, task) =
        tasks.into_iter().find(|(name, _)| name == "promotions.load_legal_data").unwrap();
    task.run(&task.parse(&[]).unwrap()).await.unwrap()
}

/// Gọi `/api/v1/promotions{uri}`; trả trạng thái và thân JSON (`null` khi rỗng).
pub(crate) async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/promotions{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    let body = if answer.body().is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(answer.body()).unwrap_or_else(|_| Value::String(answer.text()))
    };
    (status, body)
}

/// Thân tạo chương trình ở nút `org`; `extra` ghi đè trường.
pub(crate) fn body(org: OrgId, extra: Value) -> Value {
    let mut base = json!({
        "org_id": org.as_uuid(),
        "kind": "percent_off",
        "currency": "VND",
        "value_bp": 1000,
        "starts_at": Utc::now() - Duration::hours(1),
        "ends_at": Utc::now() + Duration::days(7),
        "locale": "vi",
        "title": "Giảm giá",
        "terms": "Áp dụng toàn cửa hàng",
    });
    if let (Value::Object(base), Value::Object(extra)) = (&mut base, extra) {
        base.extend(extra);
    }
    base
}

/// Tạo chương trình bằng `boss`; trả ID.
pub(crate) async fn create(setup: &Setup, body: Value) -> String {
    let (status, created) = call(&setup.app, "POST", "/promotions", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["data"]["id"].as_str().unwrap().to_owned()
}

/// Ghi nhận mọi thông báo cần và bật chương trình; trả thân trả lời của `activate`.
pub(crate) async fn activate(setup: &Setup, id: &str) -> (StatusCode, Value) {
    let (_, checks) =
        call(&setup.app, "POST", &format!("/promotions/{id}/activation-check"), "boss", None).await;
    for check in checks["data"]["checks"].as_array().cloned().unwrap_or_default() {
        if check["code"] == "promotions.notice_required" {
            let authority = check["params"]["authority"].as_str().unwrap();
            let (status, recorded) = call(
                &setup.app,
                "PATCH",
                &format!("/promotions/{id}/notices/{authority}"),
                "boss",
                Some(json!({"status": "submitted", "reference": "TB-1"})),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{recorded}");
        }
    }
    call(&setup.app, "POST", &format!("/promotions/{id}/activate"), "boss", None).await
}

/// Tạo và bật một chương trình; trả ID.
pub(crate) async fn running(setup: &Setup, body: Value) -> Uuid {
    let id = create(setup, body).await;
    let (status, activated) = activate(setup, &id).await;
    assert_eq!(status, StatusCode::OK, "{activated}");
    id.parse().unwrap()
}

/// Job theo lịch chạy ngay: đưa vào hàng đợi của cả database rồi chạy.
#[derive(Debug, serde::Serialize)]
struct ExpireHoldsNow {}

impl autoxide_kernel::job::Job for ExpireHoldsNow {
    const KIND: &'static str = crate::jobs::EXPIRE_HOLDS;
}

/// Chạy hết job đang chờ (và event).
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

/// Cho lượt giữ của đơn `order` quá hạn rồi chạy job `promotions.expire_holds`; `true` khi job đã
/// đổi nó sang `expired`.
pub(crate) async fn expire_now(setup: &Setup, order: Uuid) -> bool {
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    crate::entity::redemption::Entity::update_many()
        .col_expr(
            crate::entity::redemption::Column::ExpiresAt,
            Expr::value(Some(Utc::now() - Duration::seconds(1))),
        )
        .filter(crate::entity::redemption::Column::OrderId.eq(order))
        .filter(crate::entity::redemption::Column::Status.eq("held"))
        .exec(setup.app.database().write())
        .await
        .unwrap();
    autoxide_kernel::job::JobQueue::new("promotions")
        .enqueue_database(
            setup.app.database().write(),
            &ExpireHoldsNow {},
            autoxide_kernel::job::EnqueueOptions::default(),
        )
        .await
        .unwrap();
    drain(&setup.app).await;
    let row = crate::entity::redemption::Entity::find()
        .filter(crate::entity::redemption::Column::OrderId.eq(order))
        .one(setup.app.database().read())
        .await
        .unwrap()
        .unwrap();
    row.status == "expired"
}
