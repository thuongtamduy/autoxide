//! Test của module `customers`, trên database gate đang chọn (gate chạy cả SQLite và Postgres).
//! `TestApp` dựng module như binary; module giả `orders` / `auth` phát event như module thật (thân
//! event giống `orders::lifecycle::payload`), `OrderExtensions` giả giữ `BuyerGroups` được cắm vào,
//! `AccountVerification` giả trả số đã xác thực.

mod groups;
mod profiles;

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
pub(crate) const ALL: [&str; 4] =
    ["customers.read", "customers.manage", "customers.contact.reveal", "customers.segments.manage"];

pub(crate) const BUYER: u128 = 101;

/// `OrderExtensions` giả: giữ `BuyerGroups` mà `customers` cắm vào.
#[derive(Default)]
pub(crate) struct FakeExtensions(pub Mutex<Option<Arc<dyn BuyerGroups>>>);

impl OrderExtensions for FakeExtensions {
    fn set_seller_split(&self, _split: Arc<dyn SellerSplit>) -> Result<(), Error> {
        Ok(())
    }
    fn set_buyer_groups(&self, groups: Arc<dyn BuyerGroups>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(groups);
        Ok(())
    }
    fn add_tender(&self, _tender: Arc<dyn Tender>) -> Result<(), Error> {
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
fake_event!(UserDeleted, autoxide_kernel::privacy::USER_DELETED);

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
    let org = || OrgId::from_uuid(Uuid::now_v7());
    let root = root_of(DEFAULT_TENANT);
    let tree = Tree { a: org(), b: org() };
    let chain = org();
    let extensions = Arc::new(FakeExtensions::default());
    let accounts = Arc::new(FakeAccounts::default());
    let orders = Arc::new(Mutex::new(None));
    let auth = Arc::new(Mutex::new(None));
    let all: Vec<(&str, Scope)> = ALL.iter().map(|name| (*name, Scope::Tenant)).collect();
    let app = TestApp::builder(test)
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
        .module(Emitter {
            name: "orders",
            events: vec![
                autoxide_orders::api::events::ORDER_PLACED,
                autoxide_orders::api::events::ORDER_STATUS_CHANGED,
                autoxide_orders::api::events::REFUND_COMPLETED,
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
    Shop { app, tree, extensions, accounts, orders, auth }
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

    pub(crate) async fn user_deleted(&self, user: Uuid) {
        self.emit(&self.auth, &UserDeleted(json!({"before": {"id": user}}))).await;
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
            "placed_at": chrono::Utc::now(), "total": money(self.goods + 30_000),
            "goods_paid": money(self.goods), "lines": [],
        }})
    }

    pub(crate) fn placed(&self) -> Placed {
        Placed(self.payload("pending_payment"))
    }

    pub(crate) fn status(&self, status: &str) -> StatusChanged {
        StatusChanged(self.payload(status))
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
    use sea_orm::{ConnectionTrait, Statement};
    let read = app.database().read();
    let statement = Statement::from_sql_and_values(
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
