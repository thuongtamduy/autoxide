//! Test của `promotions` trên database gate đang chọn (gate chạy cả SQLite và Postgres). Dựng app
//! như binary bằng `TestApp` của testkit, với `dyn Catalog` giả (biến thể, giá tham chiếu, sàn giá
//! đặt sẵn); gọi qua REST, MCP và `dyn Promotions`.

mod blocks;
mod evaluate;
mod insights;
mod participations;
mod programs;
mod redemption;
mod vouchers;

pub(crate) use autoxide_compliance::api::testing::FakeConsentLedger;
use autoxide_kernel::time::Timestamp;
pub(crate) use autoxide_notifications::api::testing::FakeNotifier;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_catalog::api::Catalog;
use autoxide_kernel::access::Scope;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantContext, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
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

/// Cây gốc → chuỗi → cửa hàng A, B.
pub(crate) struct Tree {
    pub chain: OrgId,
    pub a: OrgId,
    pub b: OrgId,
}

pub(crate) use autoxide_catalog::api::testing::FakeCatalog;

/// Cách `promotions` thêm hàng vào catalog giả dùng chung.
pub(crate) trait CatalogExt {
    /// Thêm biến thể của nút `org` trong các danh mục `categories`, giá `price` từ 30 ngày trước.
    fn add(&self, org: OrgId, price: i64, categories: &[Uuid]) -> Uuid;
}

impl CatalogExt for FakeCatalog {
    fn add(&self, org: OrgId, price: i64, categories: &[Uuid]) -> Uuid {
        let id = Uuid::now_v7();
        let mut variant = autoxide_catalog::api::testing::variant(id, org);
        variant.category_ids = categories.to_vec();
        self.insert_variant(variant);
        self.set_price(id, money(price));
        id
    }
}

pub(crate) struct Setup {
    pub app: TestApp,
    pub tree: Tree,
    pub catalog: Arc<FakeCatalog>,
    pub notifier: Arc<FakeNotifier>,
    pub consents: Arc<FakeConsentLedger>,
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
        let mut cart = Cart::new(self.tree.a, vnd(), Timestamp::now());
        for (index, (variant, quantity)) in lines.iter().enumerate() {
            let price = self.catalog.price_of(*variant, Timestamp::now()).unwrap();
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
    let consents = Arc::new(FakeConsentLedger::new());
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
        "starts_at": Timestamp::now() - SignedDuration::from_hours(1),
        "ends_at": Timestamp::now() + SignedDuration::from_hours(24 * 7),
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
            Expr::value(Some(Timestamp::now() - SignedDuration::from_secs(1))),
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
