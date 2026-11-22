//! Test của module `customers`, trên database gate đang chọn (gate chạy cả SQLite và Postgres).
//! `TestApp` dựng module như binary; module giả `orders` / `auth` phát event như module thật (thân
//! event giống `orders::lifecycle::payload`), `OrderExtensions` giả giữ `BuyerGroups` được cắm vào,
//! `AccountVerification` giả trả số đã xác thực.

mod engagement;
pub(crate) mod fake_shop;
mod groups;
mod loyalty;
mod profiles;
mod stored_value;
mod wishlist_alerts;

pub(crate) use autoxide_compliance::api::testing::FakeConsentLedger;
use autoxide_kernel::time::Timestamp;
pub(crate) use autoxide_notifications::api::testing::FakeNotifier;
use jiff::SignedDuration;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use autoxide_compliance::api::{AccountVerification, VerificationLevel};
use autoxide_kernel::access::{Principal, Scope};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Event, EventInfo, Outbox};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantId, root_of};
use autoxide_orders::api::{BuyerGroups, OrderExtensions, SellerSplit, Tender};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

/// Mọi permission của module, độ sâu `tenant`.
pub(crate) const ALL: [&str; 8] = [
    "customers.read",
    "customers.manage",
    "customers.contact.reveal",
    "customers.segments.manage",
    "customers.loyalty.manage",
    "customers.loyalty.adjust",
    "customers.engagement.manage",
    "customers.stored_value.manage",
];

pub(crate) const BUYER: u128 = 101;

/// `OrderExtensions` giả: giữ `BuyerGroups` và `Tender` mà `customers` cắm vào.
#[derive(Default)]
pub(crate) struct FakeExtensions(
    pub Mutex<Option<Arc<dyn BuyerGroups>>>,
    pub Mutex<Vec<Arc<dyn Tender>>>,
);

impl OrderExtensions for FakeExtensions {
    fn set_seller_split(&self, _split: Arc<dyn SellerSplit>) -> Result<(), Error> {
        Ok(())
    }
    fn set_buyer_groups(&self, groups: Arc<dyn BuyerGroups>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(groups);
        Ok(())
    }
    fn add_tender(&self, tender: Arc<dyn Tender>) -> Result<(), Error> {
        self.1.lock().unwrap().push(tender);
        Ok(())
    }
}

/// Số đã xác thực theo tài khoản.
#[derive(Default)]
pub(crate) struct FakeAccounts(pub Mutex<HashMap<Uuid, String>>);

#[async_trait::async_trait]
impl AccountVerification for FakeAccounts {
    async fn level(&self, _: TenantId, _: Uuid) -> Result<VerificationLevel, Error> {
        Ok(VerificationLevel::None)
    }
    async fn verified_phone(&self, _: TenantId, user: Uuid) -> Result<Option<PhoneNumber>, Error> {
        Ok(self.0.lock().unwrap().get(&user).and_then(|raw| PhoneNumber::parse(raw, "84")))
    }
}

/// Module giả phát event thay module thật: khai tên event, giữ `Outbox` sau `start`.
pub(crate) struct Emitter {
    name: &'static str,
    events: Vec<&'static str>,
    outbox: Arc<Mutex<Option<Outbox>>>,
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

macro_rules! fake_event {
    ($kind:ident, $name:expr) => {
        #[derive(Debug, serde::Serialize)]
        #[serde(transparent)]
        pub(crate) struct $kind(pub Value);
        impl Event for $kind {
            const NAME: &'static str = $name;
        }
    };
}

fake_event!(Placed, autoxide_orders::api::events::ORDER_PLACED);
fake_event!(StatusChanged, autoxide_orders::api::events::ORDER_STATUS_CHANGED);
fake_event!(Refunded, autoxide_orders::api::events::REFUND_COMPLETED);
fake_event!(Confirmed, autoxide_orders::api::events::ORDER_CONFIRMED);
fake_event!(Cancelled, autoxide_orders::api::events::ORDER_CANCELLED);
fake_event!(UserDeleted, autoxide_kernel::privacy::USER_DELETED);
fake_event!(Abandoned, autoxide_orders::api::events::CART_ABANDONED);
fake_event!(ConsentWithdrawn, autoxide_compliance::api::CONSENT_WITHDRAWN);
fake_event!(DisplayPriceChanged, autoxide_catalog::api::events::DISPLAY_PRICE_CHANGED);
fake_event!(LevelChanged, autoxide_inventory::api::events::LEVEL_CHANGED);

/// Cây gốc → chuỗi → cửa hàng A, B.
pub(crate) struct Tree {
    pub a: OrgId,
    pub b: OrgId,
}

pub(crate) struct Shop {
    pub app: TestApp,
    pub tree: Tree,
    pub extensions: Arc<FakeExtensions>,
    pub accounts: Arc<FakeAccounts>,
    pub promotions: Arc<FakePromotions>,
    pub notifier: Arc<FakeNotifier>,
    pub consents: Arc<FakeConsentLedger>,
    pub catalog: Arc<fake_shop::FakeCatalog>,
    pub inventory: Arc<fake_shop::FakeInventory>,
    pub rules: Arc<FakeRules>,
    catalog_events: Arc<Mutex<Option<Outbox>>>,
    inventory_events: Arc<Mutex<Option<Outbox>>>,
    compliance: Arc<Mutex<Option<Outbox>>>,
    orders: Arc<Mutex<Option<Outbox>>>,
    auth: Arc<Mutex<Option<Outbox>>>,
}

impl std::ops::Deref for Shop {
    type Target = TestApp;
    fn deref(&self) -> &TestApp {
        &self.app
    }
}

/// Người dùng: `boss` (mọi quyền, cả khách), `reader` (chỉ `customers.read`), `staff_a` (đọc / sửa ở
/// cửa hàng A), người mua `buyer` (101), `buyer2` (102), `buyer3` (103).
pub(crate) async fn shop(test: &TestDatabase) -> Shop {
    shop_with(test, &[]).await
}

/// Như [`shop`], kèm biến cấu hình `settings`.
pub(crate) async fn shop_with(test: &TestDatabase, settings: &[(&str, &str)]) -> Shop {
    let org = || OrgId::from_uuid(Uuid::now_v7());
    let root = root_of(DEFAULT_TENANT);
    let tree = Tree { a: org(), b: org() };
    let chain = org();
    let extensions = Arc::new(FakeExtensions::default());
    let accounts = Arc::new(FakeAccounts::default());
    let promotions = Arc::new(FakePromotions::default());
    let notifier = Arc::new(FakeNotifier::default());
    let consents = Arc::new(FakeConsentLedger::new());
    let catalog = Arc::new(fake_shop::FakeCatalog::default());
    let inventory = Arc::new(fake_shop::FakeInventory::default());
    let rules = Arc::new(FakeRules::default());
    let catalog_events = Arc::new(Mutex::new(None));
    let inventory_events = Arc::new(Mutex::new(None));
    let compliance = Arc::new(Mutex::new(None));
    let orders = Arc::new(Mutex::new(None));
    let auth = Arc::new(Mutex::new(None));
    let all: Vec<(&str, Scope)> = ALL.iter().map(|name| (*name, Scope::Tenant)).collect();
    let mut builder = TestApp::builder(test);
    for (name, value) in settings {
        builder = builder.setting(name, value);
    }
    let app = builder
        .member("boss", 1, root, &all)
        .member("reader", 2, root, &[("customers.read", Scope::Tenant)])
        .member(
            "staff_a",
            3,
            tree.a,
            &[("customers.read", Scope::Unit), ("customers.manage", Scope::Unit)],
        )
        .principal("buyer", Principal::customer(Uuid::from_u128(BUYER)))
        .principal("buyer2", Principal::customer(Uuid::from_u128(102)))
        .principal("buyer3", Principal::customer(Uuid::from_u128(103)))
        .provide::<dyn OrderExtensions>(extensions.clone())
        .provide::<dyn AccountVerification>(accounts.clone())
        .provide::<dyn autoxide_promotions::api::Promotions>(promotions.clone())
        .provide::<dyn autoxide_notifications::api::Notifier>(notifier.clone())
        .provide::<dyn autoxide_compliance::api::ConsentLedger>(consents.clone())
        .provide::<dyn autoxide_catalog::api::Catalog>(catalog.clone())
        .provide::<dyn autoxide_inventory::api::Inventory>(inventory.clone())
        .provide::<dyn autoxide_compliance::api::ComplianceRules>(rules.clone())
        .module(Emitter {
            name: "catalog",
            events: vec![autoxide_catalog::api::events::DISPLAY_PRICE_CHANGED],
            outbox: catalog_events.clone(),
        })
        .module(Emitter {
            name: "inventory",
            events: vec![autoxide_inventory::api::events::LEVEL_CHANGED],
            outbox: inventory_events.clone(),
        })
        .module(Emitter {
            name: "compliance",
            events: vec![autoxide_compliance::api::CONSENT_WITHDRAWN],
            outbox: compliance.clone(),
        })
        .provide::<dyn autoxide_ledger::api::Ledger>(Arc::new(
            autoxide_ledger::api::testing::FakeLedger::new(),
        ))
        .module(Emitter {
            name: "orders",
            events: vec![
                autoxide_orders::api::events::ORDER_PLACED,
                autoxide_orders::api::events::ORDER_STATUS_CHANGED,
                autoxide_orders::api::events::REFUND_COMPLETED,
                autoxide_orders::api::events::ORDER_CONFIRMED,
                autoxide_orders::api::events::ORDER_CANCELLED,
                autoxide_orders::api::events::CART_ABANDONED,
            ],
            outbox: orders.clone(),
        })
        .module(Emitter {
            name: "auth",
            events: vec![autoxide_kernel::privacy::USER_DELETED],
            outbox: auth.clone(),
        })
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let write = app.database().begin_write().await.unwrap();
    OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, chain, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.a, chain, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.b, chain, 8).await.unwrap();
    write.commit().await.unwrap();
    Shop {
        app,
        tree,
        extensions,
        accounts,
        promotions,
        notifier,
        consents,
        catalog,
        inventory,
        rules,
        catalog_events,
        inventory_events,
        compliance,
        orders,
        auth,
    }
}

impl Shop {
    async fn emit<E: Event>(&self, outbox: &Arc<Mutex<Option<Outbox>>>, event: &E) {
        let outbox = outbox.lock().unwrap().clone().unwrap();
        let tenant = self.default_tenant();
        let transaction = self.database().begin_write().await.unwrap();
        outbox.record(&transaction, &tenant, Actor::system(), None, event).await.unwrap();
        transaction.commit().await.unwrap();
    }

    /// Ghi event của `orders` giả.
    pub(crate) async fn order_event<E: Event>(&self, event: &E) {
        self.emit(&self.orders, event).await;
    }

    /// Giá công khai của sản phẩm `product` đổi (event của `catalog` giả).
    pub(crate) async fn price_changed(&self, product: Uuid) {
        let event = DisplayPriceChanged(json!({"after": {"product_id": product}}));
        self.emit(&self.catalog_events, &event).await;
    }

    /// Tồn bán được của biến thể `variant` đi từ `before` lên `after` (event của `inventory` giả).
    pub(crate) async fn stock_changed(&self, variant: Uuid, before: i64, after: i64) {
        self.inventory.available.lock().unwrap().insert(variant, after);
        let event = LevelChanged(json!({
            "before": {"available": before},
            "after": {"variant_id": variant, "available": after},
        }));
        self.emit(&self.inventory_events, &event).await;
    }

    pub(crate) async fn compliance_event<E: Event>(&self, event: &E) {
        self.emit(&self.compliance, event).await;
    }

    pub(crate) async fn user_deleted(&self, user: Uuid) {
        self.emit(&self.auth, &UserDeleted(json!({"before": {"id": user}}))).await;
    }

    /// `Tender` loại `points` mà `customers` đã cắm vào `orders`.
    pub(crate) fn points_tender(&self) -> Arc<dyn Tender> {
        self.tender("points").unwrap()
    }

    /// `Tender` loại `kind` mà `customers` đã cắm vào `orders` (nếu có).
    pub(crate) fn tender(&self, kind: &str) -> Option<Arc<dyn Tender>> {
        self.extensions.1.lock().unwrap().iter().find(|tender| tender.kind() == kind).cloned()
    }

    /// Chạy job định kỳ `kind` ngay (như lịch của database).
    pub(crate) async fn run_job(&self, kind: &'static str) {
        let queue = autoxide_kernel::job::JobQueue::new("customers");
        let options = autoxide_kernel::job::EnqueueOptions::default;
        let write = self.database().write();
        match kind {
            crate::jobs::EXPIRE_POINTS => {
                queue.enqueue_database(write, &ExpirePointsNow {}, options()).await
            }
            crate::jobs::RELEASE_EXPIRED_HOLDS => {
                queue.enqueue_database(write, &ReleaseHoldsNow {}, options()).await
            }
            crate::jobs::BIRTHDAY_VOUCHERS => {
                queue.enqueue_database(write, &BirthdayNow {}, options()).await
            }
            crate::jobs::EVALUATE_TIERS => {
                queue.enqueue_database(write, &TiersNow {}, options()).await
            }
            crate::jobs::SEND_WISHLIST_ALERTS => {
                queue.enqueue_database(write, &WishlistAlertsNow {}, options()).await
            }
            other => panic!("no test job for `{other}`"),
        }
        .unwrap();
        drain(self).await;
    }

    /// `BuyerGroups` mà `customers` đã cắm vào `orders`.
    pub(crate) fn buyer_groups(&self) -> Arc<dyn BuyerGroups> {
        self.extensions.0.lock().unwrap().clone().expect("customers plugged buyer groups")
    }
}

/// Một đơn giả.
pub(crate) struct Order {
    pub id: Uuid,
    pub org: OrgId,
    pub buyer: Option<Uuid>,
    pub buyer_key: Option<String>,
    pub goods: i64,
}

impl Order {
    pub(crate) fn of(buyer: u128, org: OrgId, goods: i64) -> Self {
        Order {
            id: Uuid::now_v7(),
            org,
            buyer: Some(Uuid::from_u128(buyer)),
            buyer_key: None,
            goods,
        }
    }

    /// Thân event như `orders::lifecycle::payload`.
    pub(crate) fn payload(&self, status: &str) -> Value {
        let money = |minor: i64| json!({"amount": minor.to_string(), "currency": "VND"});
        json!({"after": {
            "order_id": self.id, "tenant_id": DEFAULT_TENANT, "org_id": self.org, "channel": "web",
            "number": format!("DH{}", &self.id.simple().to_string()[24..]), "status": status,
            "buyer_user_id": self.buyer, "buyer_key": self.buyer_key,
            "hold_reference": format!("orders:{}", self.id),
            "placed_at": Timestamp::now(), "total": money(self.goods + 30_000),
            "goods_paid": money(self.goods), "lines": [],
        }})
    }

    pub(crate) fn placed(&self) -> Placed {
        Placed(self.payload("pending_payment"))
    }

    pub(crate) fn status(&self, status: &str) -> StatusChanged {
        StatusChanged(self.payload(status))
    }

    pub(crate) fn confirmed(&self) -> Confirmed {
        Confirmed(self.payload("confirmed"))
    }

    pub(crate) fn cancelled(&self) -> Cancelled {
        Cancelled(self.payload("cancelled"))
    }

    pub(crate) fn refunded(&self, refund_id: Uuid, amount: i64) -> Refunded {
        Refunded(json!({"after": {
            "order_id": self.id, "tenant_id": DEFAULT_TENANT, "org_id": self.org, "channel": "web",
            "refund_id": refund_id, "amount": {"amount": amount.to_string(), "currency": "VND"},
            "manual": false, "reason": "test",
        }}))
    }
}

/// Chạy hết event và job đang chờ.
pub(crate) async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..20 {
        let events = dispatcher.run_once().await.unwrap();
        if worker.run_once().await.unwrap() + events == 0 {
            break;
        }
    }
}

/// Gọi `/api/v1/customers{uri}`; trả trạng thái và thân JSON (`null` khi rỗng).
pub(crate) async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/customers{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

/// Số event `name` trong outbox của `customers`.
pub(crate) async fn events_named(app: &TestApp, name: &str) -> Vec<Value> {
    use sea_orm::ConnectionTrait;
    let read = app.database().read();
    let statement = autoxide_kernel::database::statement(
        read.get_database_backend(),
        match read.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres => {
                "SELECT payload FROM customers_events WHERE name = $1 ORDER BY id"
            }
            _ => "SELECT payload FROM customers_events WHERE name = ? ORDER BY id",
        },
        [name.into()],
    );
    read.query_all_raw(statement)
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            let text: String = row.try_get("", "payload").unwrap();
            serde_json::from_str(&text).unwrap()
        })
        .collect()
}

pub(crate) fn minor(value: &Value) -> i64 {
    value["amount"].as_str().unwrap().parse().unwrap()
}

/// `Promotions` giả: chỉ `grant_voucher` (ghi lại, chống trùng theo `source_ref`; `fail` → lỗi).
#[derive(Default)]
pub(crate) struct FakePromotions {
    pub granted: Mutex<Vec<autoxide_promotions::api::GrantVoucher>>,
    pub fail: Mutex<bool>,
}

#[async_trait::async_trait]
impl autoxide_promotions::api::Promotions for FakePromotions {
    async fn evaluate(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        _: &autoxide_promotions::api::Cart,
    ) -> Result<autoxide_promotions::api::Evaluation, Error> {
        Err(Error::internal("not used"))
    }
    async fn eligible(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        _: &autoxide_promotions::api::Cart,
    ) -> Result<autoxide_promotions::api::Eligibility, Error> {
        Err(Error::internal("not used"))
    }
    async fn reserve(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        _: &autoxide_promotions::api::ReserveRequest,
    ) -> Result<autoxide_kernel::reference::Submitted<autoxide_promotions::api::Redemption>, Error>
    {
        Err(Error::internal("not used"))
    }
    async fn confirm(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        _: Uuid,
    ) -> Result<autoxide_promotions::api::Redemption, Error> {
        Err(Error::internal("not used"))
    }
    async fn release(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        _: Uuid,
    ) -> Result<Option<autoxide_promotions::api::Redemption>, Error> {
        Ok(None)
    }
    async fn redemption(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        _: Uuid,
    ) -> Result<Option<autoxide_promotions::api::Redemption>, Error> {
        Ok(None)
    }
    async fn grant_voucher(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        request: &autoxide_promotions::api::GrantVoucher,
    ) -> Result<autoxide_kernel::reference::Submitted<autoxide_promotions::api::Voucher>, Error>
    {
        if *self.fail.lock().unwrap() {
            return Err(Error::internal("promotions is down"));
        }
        let mut granted = self.granted.lock().unwrap();
        let replay = granted
            .iter()
            .any(|old| old.source_ref == request.source_ref && old.user_id == request.user_id);
        if !replay {
            granted.push(request.clone());
        }
        let voucher: autoxide_promotions::api::Voucher = serde_json::from_value(json!({
            "id": Uuid::now_v7(), "promotion_id": request.promotion_id, "holder_user_id": request.user_id,
            "status": "available", "source": request.source, "source_ref": request.source_ref,
            "valid_from": Timestamp::now(), "expires_at": Timestamp::now() + SignedDuration::from_hours(24 * 30),
            "issued_at": Timestamp::now(), "used_order_id": null,
        }))
        .unwrap();
        Ok(if replay {
            autoxide_kernel::reference::Submitted::Replayed(voucher)
        } else {
            autoxide_kernel::reference::Submitted::Created(voucher)
        })
    }
    async fn vouchers_of(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        _: Uuid,
        _: Option<autoxide_promotions::api::VoucherStatus>,
    ) -> Result<Vec<autoxide_promotions::api::Voucher>, Error> {
        Ok(Vec::new())
    }
    async fn automatic_prices(
        &self,
        _: &autoxide_kernel::tenancy::TenantContext,
        _: OrgId,
        _: Timestamp,
        _: &[(Uuid, autoxide_kernel::money::Money)],
    ) -> Result<Vec<autoxide_promotions::api::AutomaticPrice>, Error> {
        Ok(Vec::new())
    }
}

/// Đẩy mốc hết hạn của mọi lô điểm và chỗ giữ về quá khứ (giả lập thời gian trôi).
pub(crate) async fn age_points(app: &TestApp) {
    use sea_orm::ConnectionTrait;
    let write = app.database().write();
    let past = Timestamp::now() - SignedDuration::from_hours(24);
    for table in ["customers_points_lots", "customers_point_holds"] {
        let statement = sea_orm::sea_query::Query::update()
            .table(sea_orm::sea_query::Alias::new(table))
            .value(sea_orm::sea_query::Alias::new("expires_at"), sea_orm::Value::from(past))
            .to_owned();
        write.execute(&statement).await.unwrap();
    }
}

/// Job định kỳ chạy ngay: mỗi loại một kiểu (`Job::KIND` là hằng).
macro_rules! now_job {
    ($kind:ident, $name:expr) => {
        #[derive(Debug, serde::Serialize)]
        pub(crate) struct $kind {}

        impl autoxide_kernel::job::Job for $kind {
            const KIND: &'static str = $name;
        }
    };
}

now_job!(ExpirePointsNow, crate::jobs::EXPIRE_POINTS);
now_job!(ReleaseHoldsNow, crate::jobs::RELEASE_EXPIRED_HOLDS);
now_job!(BirthdayNow, crate::jobs::BIRTHDAY_VOUCHERS);
now_job!(TiersNow, crate::jobs::EVALUATE_TIERS);
now_job!(WishlistAlertsNow, crate::jobs::SEND_WISHLIST_ALERTS);

/// Quy tắc `notifications.send_windows` giả: mặc định khung đang ĐÓNG (lịch hằng giờ của job tự chạy
/// trong `drain` không gửi gì); `open()` đặt một khung quanh giờ hiện tại.
#[derive(Default)]
pub(crate) struct FakeRules(pub Mutex<bool>);

impl FakeRules {
    pub(crate) fn close(&self) {
        *self.0.lock().unwrap() = false;
    }
    pub(crate) fn open(&self) {
        *self.0.lock().unwrap() = true;
    }
}

#[async_trait::async_trait]
impl autoxide_compliance::api::ComplianceRules for FakeRules {
    async fn effective(
        &self,
        _: TenantId,
        key: &str,
        at: Timestamp,
    ) -> Result<Option<autoxide_compliance::api::Rule>, Error> {
        if key != "notifications.send_windows" {
            return Ok(None);
        }
        // Giờ UTC (lệch 0): khung mở = 2 giờ trước tới 2 giờ sau; khung đóng = 3 tới 5 giờ sau.
        let hhmm =
            |hours: i64| (at + SignedDuration::from_hours(hours)).format("%H:%M").to_string();
        let window = if *self.0.lock().unwrap() {
            json!({"from": hhmm(-2), "to": hhmm(2)})
        } else {
            json!({"from": hhmm(3), "to": hhmm(5)})
        };
        let body = json!({"utc_offset_minutes": 0, "marketing": {"email": window, "sms": null}});
        Ok(Some(serde_json::from_value(json!({
            "jurisdiction": "vn", "key": key, "version": 1, "effective_from": at,
            "effective_to": null, "body": body, "legal_basis": "test", "needs_legal_review": false,
        }))
        .map_err(Error::internal)?))
    }
}

/// Người mua `user` đồng ý nhận quảng cáo (`marketing`, mọi kênh).
pub(crate) fn allow_marketing(consents: &FakeConsentLedger, user: u128) {
    consents.allow(
        autoxide_compliance::api::ConsentSubject::User(Uuid::from_u128(user)),
        autoxide_compliance::api::ConsentKey::new("marketing", "", ""),
    );
}

/// Người mua `user` rút đồng ý nhận quảng cáo.
pub(crate) fn deny_marketing(consents: &FakeConsentLedger, user: u128) {
    consents.deny(
        autoxide_compliance::api::ConsentSubject::User(Uuid::from_u128(user)),
        autoxide_compliance::api::ConsentKey::new("marketing", "", ""),
    );
}

/// Người đang đồng ý nhận báo yêu thích (ghi qua module bằng `grant` / `withdraw`).
pub(crate) fn alert_subscribers(consents: &FakeConsentLedger) -> Vec<Uuid> {
    consents
        .records()
        .into_iter()
        .filter(|record| {
            record.consent.purpose == crate::wishlist_alerts::PURPOSE
                && record.consent.state == "granted"
        })
        .filter_map(|record| match record.subject {
            autoxide_compliance::api::ConsentSubject::User(user) => Some(user),
            _ => None,
        })
        .collect()
}

/// Cho mọi job đang chờ của `customers` tới hạn ngay (giả lập thời gian trôi).
pub(crate) async fn jobs_due_now(app: &TestApp) {
    use sea_orm::ConnectionTrait;
    use sea_orm::sea_query::ExprTrait as _;
    let write = app.database().write();
    let past = Timestamp::now() - SignedDuration::from_mins(1);
    let statement = sea_orm::sea_query::Query::update()
        .table(sea_orm::sea_query::Alias::new("customers_jobs"))
        .value(sea_orm::sea_query::Alias::new("run_at"), sea_orm::Value::from(past))
        .and_where(
            sea_orm::sea_query::Expr::col(sea_orm::sea_query::Alias::new("completed_at")).is_null(),
        )
        .to_owned();
    write.execute(&statement).await.unwrap();
}
