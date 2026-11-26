//! Test của module `analytics`, chạy trên SQLite và Postgres (theo `TEST_DATABASE_URL`).
//!
//! Một module giả `shop` khai loại sự kiện (xem sản phẩm, thêm giỏ — từ trình duyệt; đã mua — từ
//! event `shop.order_paid` của nó) và phễu `shop.checkout`, như `catalog` / `orders` sẽ làm.

mod dormant;
mod ingest;
mod pages;
mod privacy;
mod reports;

use autoxide_compliance::api::testing::FakeConsentLedger;
use autoxide_kernel::time::Timestamp;
use std::net::IpAddr;
use std::sync::Arc;

use autoxide_compliance::api::{ConsentKey, ConsentLedger, ConsentSubject};
use autoxide_kernel::access::Scope;
use autoxide_kernel::analytics::{
    AnalyticsCatalog, AnalyticsEvent, Funnel, Prop, PropKind, ServerHit,
};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventCatalog, EventInfo};
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::module::Module;
use autoxide_kernel::report::org_field;
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, Site, TenantContext, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::config::AnalyticsConfig;
use crate::ingest::{Ingested, Visitor};
use crate::service::AnalyticsService;

pub(super) const CHROME: &str = "Mozilla/5.0 (Linux; Android 14; SM-S918B) AppleWebKit/537.36 \
                                 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36";
pub(super) const SAFARI: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_6) \
                                 AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 \
                                 Safari/605.1.15";
pub(super) const HOST: &str = "shop.example";

const ORDER_PAID: EventInfo = EventInfo::new("shop.order_paid", "An order was paid.");

fn paid(event: &Delivered) -> Result<Option<ServerHit>, Error> {
    let product = event.payload.get("product_id").and_then(Value::as_str);
    let user = event.payload.get("user_id").and_then(Value::as_str).and_then(|id| id.parse().ok());
    let mut hit = ServerHit::new(event.occurred_at).org(org_field(&event.payload)).user(user);
    if let Some(product) = product {
        hit = hit.object(format!("shop.product:{product}"));
    }
    Ok(Some(hit))
}

const VARIANT: &[Prop] = &[Prop::new("variant", PropKind::Text)];
const QUANTITY: &[Prop] = &[Prop::new("quantity", PropKind::Number)];

pub(super) fn shop_events() -> Vec<AnalyticsEvent> {
    vec![
        AnalyticsEvent::browser("shop.product_viewed", "A product page was shown.")
            .object_kind("shop.product")
            .props(VARIANT),
        AnalyticsEvent::browser("shop.cart_added", "A product went into the cart.")
            .object_kind("shop.product")
            .props(QUANTITY),
        AnalyticsEvent::server("shop.purchased", "An order was paid.", "shop.order_paid", paid)
            .object_kind("shop.product"),
    ]
}

pub(super) fn shop_funnels() -> Vec<Funnel> {
    vec![Funnel::new(
        "shop.checkout",
        "From product page to paid order.",
        &["shop.product_viewed", "shop.cart_added", "shop.purchased"],
    )]
}

/// Module giả khai loại sự kiện và phễu.
struct Shop;

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn events(&self) -> Vec<EventInfo> {
        vec![ORDER_PAID]
    }

    fn analytics_events(&self) -> Vec<AnalyticsEvent> {
        shop_events()
    }

    fn funnels(&self) -> Vec<Funnel> {
        shop_funnels()
    }
}

/// Người `user` đồng ý `analytics.personal` (dựng dữ liệu, không qua module).
pub(super) fn grant_personal(ledger: &FakeConsentLedger, user: Uuid) {
    ledger.allow(ConsentSubject::User(user), ConsentKey::new(crate::api::PERSONAL_PURPOSE, "", ""));
}

/// Người `user` rút đồng ý `analytics.personal`.
pub(super) fn withdraw_personal(ledger: &FakeConsentLedger, user: Uuid) {
    ledger.deny(ConsentSubject::User(user), ConsentKey::new(crate::api::PERSONAL_PURPOSE, "", ""));
}

/// Cây gốc → A, B (hai website của hai cửa hàng).
pub(super) struct Tree {
    pub a: OrgId,
    pub b: OrgId,
}

pub(super) struct Harness {
    pub app: TestApp,
    pub service: Arc<AnalyticsService>,
    pub ledger: Arc<FakeConsentLedger>,
    pub tenant: TenantContext,
    pub tree: Tree,
}

pub(super) const READ: &str = "analytics.read";

/// App có module `analytics` và `shop`, sổ đồng ý giả, cây gốc → A, B; người dùng:
/// `boss` (đọc mọi website), `north` (chỉ nhánh A), `buyer` (người mua, không permission).
pub(super) async fn harness(test: &TestDatabase) -> Harness {
    let org = || OrgId::from_uuid(Uuid::now_v7());
    let tree = Tree { a: org(), b: org() };
    let ledger = Arc::new(FakeConsentLedger::new());
    let consents: Arc<dyn ConsentLedger> = ledger.clone();
    let app = TestApp::builder(test)
        .provide::<dyn ConsentLedger>(consents.clone())
        .member("boss", 1, root_of(DEFAULT_TENANT), &[(READ, Scope::Tenant)])
        .member("north", 2, tree.a, &[(READ, Scope::Subtree)])
        .user("buyer", 9, &[])
        .module(crate::module())
        .module(Shop)
        .build()
        .await
        .unwrap();
    let write = app.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.a, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, tree.b, root, 8).await.unwrap();
    write.commit().await.unwrap();
    let events = EventCatalog::new(vec![("shop", vec![ORDER_PAID])]).unwrap();
    let catalog = AnalyticsCatalog::new(
        vec![
            ("analytics", vec![crate::PAGE_VIEW], vec![]),
            ("shop", shop_events(), shop_funnels()),
        ],
        &events,
    )
    .unwrap();
    let service = Arc::new(AnalyticsService {
        config: AnalyticsConfig::default(),
        catalog,
        secret: SecretKey::new(
            app.database().clone(),
            Some(b"analytics-test-secret-0123456789".to_vec()),
        ),
        consents: Some(consents),
        registry: app.registry().clone(),
        settings: app.api::<dyn autoxide_kernel::tenancy::TenantSettings>(),
    });
    let tenant = app.default_tenant();
    Harness { app, service, ledger, tenant, tree }
}

/// Người dùng `n` của testkit (`user` / `member`).
pub(super) fn user_id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// Trình duyệt từ `ip` với user agent `agent`, trên trang của `HOST`.
pub(super) fn browser(ip: &str, agent: &str) -> ClientInfo {
    let mut client = ClientInfo::from_ip(ip.parse::<IpAddr>().unwrap());
    client.user_agent = Some(agent.to_owned());
    client.origin = Some(format!("https://{HOST}"));
    client
}

pub(super) fn at(day: i8, hour: i8) -> Timestamp {
    Timestamp::from_utc(2026, 11, day, hour, 0, 0).unwrap()
}

pub(super) fn page(path: &str) -> Value {
    json!({"type": "analytics.page_view", "path": path})
}

impl Harness {
    pub async fn site(&self, org: Option<OrgId>) -> Site {
        match org {
            Some(org) => Site::of(&self.tenant, org).await.unwrap(),
            None => Site::whole(&self.tenant),
        }
    }

    /// Gửi `events` như trình duyệt `client` lúc `now` lên website `org` (`None`: cả khách).
    pub async fn send(
        &self,
        org: Option<OrgId>,
        client: &ClientInfo,
        user: Option<Uuid>,
        events: Vec<Value>,
        now: Timestamp,
    ) -> Ingested {
        let site = self.site(org).await;
        let visitor = Visitor { client, host: Some(HOST), user_id: user };
        self.service.ingest(&self.tenant, &site, visitor, events, now).await.unwrap()
    }

    /// Mọi dòng của `analytics_events`.
    pub async fn raw(&self) -> Vec<crate::entity::event::Model> {
        use sea_orm::{EntityTrait, QueryOrder};
        crate::entity::event::Entity::find()
            .order_by_asc(crate::entity::event::Column::Id)
            .all(self.app.database().read())
            .await
            .unwrap()
    }
}
