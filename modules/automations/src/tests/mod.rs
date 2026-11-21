//! Test của module `automations`, trên cả SQLite và Postgres (`TestDatabase`).
//!
//! Module mẫu `shop` cho test: phát `shop.order_placed` (kèm số liệu báo cáo `shop.revenue`), khai số
//! liệu tóm tắt `shop.pending` và các tool: `tag_order` (đếm số lần gọi), `place_order` (phát lại
//! `shop.order_placed` — để thử vòng lặp), `flaky` (hỏng lần đầu), `purge` (xoá: rủi ro, chờ duyệt).

mod metrics;
mod rules;
mod runs;

use autoxide_kernel::time::Timestamp;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use autoxide_kernel::access::{Authenticator, Grant, OrgTarget, Permission, Principal, Scope};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, Event, EventInfo, Outbox};
use autoxide_kernel::idempotency::{Begin, IdempotencyStore, StoredResponse};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::report::{Fact, FactKind, FactSource, Measure, money_field};
use autoxide_kernel::summary::{MetricInfo, MetricValue, SummaryMetric};
use autoxide_kernel::tenancy::{TenantContext, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::entity::{rule, run};
use crate::service::Automations;
use crate::service::rules::{NewRule, Rule};
use crate::service::validate::RawBody;

pub(crate) use fixtures::*;

/// Người dùng `id` với các permission `permissions` cho cả khách.
pub(crate) fn person(id: u128, permissions: &[&str]) -> Principal {
    let root = root_of(autoxide_kernel::tenancy::DEFAULT_TENANT);
    Principal::with_grants(
        Uuid::from_u128(id),
        permissions
            .iter()
            .map(|permission| ((*permission).to_owned(), Grant::new(root, Scope::Tenant))),
    )
}

/// App của test: `shop`, `idempotency`, `automations`; người 1 có đủ quyền (token `1`).
pub(crate) struct Harness {
    pub app: TestApp,
    pub shop: Shop,
    pub people: People,
    pub service: Arc<Automations>,
    pub tenant: TenantContext,
    pub owner: Principal,
}

impl Harness {
    pub(crate) async fn new(test: &TestDatabase) -> Harness {
        Harness::with(test, &[]).await
    }

    pub(crate) async fn with(test: &TestDatabase, settings: &[(&str, &str)]) -> Harness {
        let shop = Shop::default();
        let people = People::default();
        let owner = person(1, &OWNER);
        people.set(owner.clone());
        let (module, shared) = crate::module_with_service();
        let mut builder = TestApp::builder(test)
            .provide::<dyn Authenticator>(Arc::new(people.clone()))
            .provide::<dyn IdempotencyStore>(Arc::new(MemoryStore::default()))
            .module(shop.clone())
            .module(module);
        for (name, value) in settings {
            builder = builder.setting(name, value);
        }
        let app = builder.build().await.unwrap();
        let service = shared.get().unwrap().clone();
        let tenant = app.default_tenant();
        Harness { app, shop, people, service, tenant, owner }
    }

    /// Tạo luật (người 1) và trả nó; lỗi thì panic kèm lỗi.
    pub(crate) async fn rule(&self, trigger: Value, conditions: Value, actions: Value) -> Rule {
        self.rule_by(&self.owner, trigger, conditions, actions).await.unwrap()
    }

    pub(crate) async fn rule_by(
        &self,
        principal: &Principal,
        trigger: Value,
        conditions: Value,
        actions: Value,
    ) -> Result<Rule, Error> {
        let input = NewRule {
            name: "Test rule".to_owned(),
            description: None,
            org_id: None,
            body: RawBody { trigger, conditions, actions },
            enabled: true,
        };
        self.service.create_rule(&self.tenant, principal, input).await
    }

    /// Giao mọi event rồi chạy mọi job tới khi hết việc.
    pub(crate) async fn settle(&self) {
        let dispatcher = self.app.modules().dispatcher().unwrap();
        let worker = self.app.modules().worker(4).unwrap();
        for _ in 0..10 {
            let delivered = dispatcher.run_once().await.unwrap();
            let ran = worker.run_once().await.unwrap();
            if delivered == 0 && ran == 0 {
                break;
            }
        }
    }

    pub(crate) async fn runs(&self, rule_id: Uuid) -> Vec<run::Model> {
        use autoxide_kernel::tenancy::ScopeExt;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
        run::Entity::find()
            .filter(run::Column::RuleId.eq(rule_id))
            .scoped(&self.tenant)
            .order_by_asc(run::Column::Id)
            .all(self.tenant.database().read())
            .await
            .unwrap()
    }

    pub(crate) async fn reload(&self, rule_id: Uuid) -> Rule {
        use autoxide_kernel::tenancy::ScopeExt;
        use sea_orm::EntityTrait;
        rule::Entity::find_by_id(rule_id)
            .scoped(&self.tenant)
            .one(self.tenant.database().read())
            .await
            .unwrap()
            .unwrap()
    }

    pub(crate) fn count(&self, pick: impl Fn(&Counters) -> &AtomicU64) -> u64 {
        pick(&self.shop.counters).load(Ordering::SeqCst)
    }
}

/// Kích hoạt theo event `shop.order_placed`.
pub(crate) fn on_order() -> Value {
    json!({"type": "event", "event": "shop.order_placed"})
}

// Module mẫu và bộ xác thực của test nằm trong `#[cfg(test)] mod fixtures` để `cargo xtask check`
// bỏ qua mã lỗi của chúng (`shop.busy`, `auth.invalid_token` — không phải mã của module này).
#[cfg(test)]
mod fixtures {
    use super::*;

    pub(crate) const SHOP_WRITE: Permission = Permission::new("shop.write", "Change the shop.");
    pub(crate) const SHOP_READ: Permission = Permission::new("shop.read", "See the shop.");
    pub(crate) const REVENUE: FactKind =
        FactKind::new("shop.revenue", "Money from orders.", Measure::Amount, &[]);

    /// Quyền của người tạo luật trong test.
    pub(crate) const OWNER: [&str; 5] =
        ["automations.read", "automations.manage", "automations.run", "shop.write", "shop.read"];

    #[derive(Serialize)]
    struct OrderPlaced(Value);

    impl Event for OrderPlaced {
        const NAME: &'static str = "shop.order_placed";
    }

    /// Số lần gọi từng tool, và giá trị của số liệu tóm tắt.
    #[derive(Default)]
    pub(crate) struct Counters {
        pub tagged: AtomicU64,
        pub placed: AtomicU64,
        pub flaky: AtomicU64,
        pub purged: AtomicU64,
        pub pending: AtomicU64,
    }

    #[derive(Clone, Default)]
    pub(crate) struct Shop {
        pub outbox: Arc<OnceLock<Outbox>>,
        pub counters: Arc<Counters>,
    }

    /// An order to tag.
    #[derive(Debug, Deserialize, Validate, ToSchema)]
    #[serde(deny_unknown_fields)]
    pub(crate) struct TagInput {
        /// Id of the order.
        order_id: String,
        /// The tag.
        #[schema(min_length = 1, max_length = 20)]
        tag: String,
    }

    /// No arguments.
    #[derive(Debug, Default, Deserialize, Validate, ToSchema)]
    #[serde(deny_unknown_fields)]
    pub(crate) struct Nothing {}

    fn revenue(event: &Delivered) -> Result<Vec<Fact>, Error> {
        let amount =
            money_field(&event.payload, "total")?.ok_or_else(|| Error::internal("total"))?;
        Ok(vec![Fact::new(REVENUE.kind, event.occurred_at).amount(amount)])
    }

    struct Pending(Arc<Counters>);

    #[async_trait::async_trait]
    impl SummaryMetric for Pending {
        fn info(&self) -> MetricInfo {
            MetricInfo::new(
                "shop.pending",
                "Pending orders",
                "Orders not shipped yet, right now.",
                "sales",
            )
        }

        fn permission(&self) -> Permission {
            SHOP_READ
        }

        async fn value(
            &self,
            _: &TenantContext,
            _: &Principal,
            _: OrgTarget,
        ) -> Result<MetricValue, Error> {
            Ok(MetricValue::Count(self.0.pending.load(Ordering::SeqCst)))
        }
    }

    impl Module for Shop {
        fn name(&self) -> &'static str {
            "shop"
        }

        fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
            vec![
                autoxide_kernel::event::outbox_migration("m0001_events", "shop"),
                autoxide_kernel::event::outbox_actor_migration("m0002_actor", "shop"),
                autoxide_kernel::tenancy::migration::adopt(
                    "m0003_tenancy",
                    vec![autoxide_kernel::event::tenant_table("shop")],
                ),
                autoxide_kernel::event::outbox_access_migration("k0001_access", "shop"),
            ]
        }

        fn permissions(&self) -> Vec<Permission> {
            vec![SHOP_WRITE, SHOP_READ]
        }

        fn events(&self) -> Vec<EventInfo> {
            vec![EventInfo::new("shop.order_placed", "An order was placed (`total`, `code`).")]
        }

        fn facts(&self) -> Vec<FactSource> {
            vec![FactSource::new("shop.order_placed", vec![REVENUE], revenue)]
        }

        fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
            let _ = self.outbox.set(context.outbox()?);
            context.summary_metric(Arc::new(Pending(self.counters.clone())))
        }

        fn tools(&self) -> Vec<Tool> {
            let (tag, place, flaky, purge) =
                (self.clone(), self.clone(), self.clone(), self.clone());
            vec![
                Tool::new(
                    "tag_order",
                    "Tag an order.",
                    move |_: TenantContext, _: Principal, input: TagInput| {
                        let shop = tag.clone();
                        async move {
                            shop.counters.tagged.fetch_add(1, Ordering::SeqCst);
                            Ok::<_, Error>(json!({"order_id": input.order_id, "tag": input.tag}))
                        }
                    },
                )
                .requires(SHOP_WRITE),
                Tool::new(
                    "place_order",
                    "Place an order.",
                    move |tenant: TenantContext, principal: Principal, _: Nothing| {
                        let shop = place.clone();
                        async move {
                            shop.counters.placed.fetch_add(1, Ordering::SeqCst);
                            shop.place_as(&tenant, Actor::from(&principal), 100).await;
                            Ok::<_, Error>(json!({"placed": true}))
                        }
                    },
                )
                .requires(SHOP_WRITE),
                Tool::new(
                    "flaky",
                    "Fail the first time.",
                    move |_: TenantContext, _: Principal, _: Nothing| {
                        let shop = flaky.clone();
                        async move {
                            if shop.counters.flaky.fetch_add(1, Ordering::SeqCst) == 0 {
                                return Err(Error::conflict("shop.busy", "Busy; try again."));
                            }
                            Ok(json!({"ok": true}))
                        }
                    },
                )
                .requires(SHOP_WRITE),
                Tool::new(
                    "purge",
                    "Delete every order.",
                    move |_: TenantContext, _: Principal, _: Nothing| {
                        let shop = purge.clone();
                        async move {
                            shop.counters.purged.fetch_add(1, Ordering::SeqCst);
                            Ok::<_, Error>(json!({"purged": true}))
                        }
                    },
                )
                .requires(SHOP_WRITE)
                .destructive(),
            ]
        }
    }

    impl Shop {
        /// Phát `shop.order_placed` (trong transaction, như module thật) với tổng tiền `total` VND.
        pub(crate) async fn place_as(&self, tenant: &TenantContext, actor: Actor, total: i64) {
            let outbox = self.outbox.get().unwrap();
            let transaction = tenant.database().begin_write().await.unwrap();
            let payload =
                json!({"total": Money::new(total, Currency::VND), "code": "DH1", "tags": ["new"]});
            outbox.record(&transaction, tenant, actor, None, &OrderPlaced(payload)).await.unwrap();
            transaction.commit().await.unwrap();
        }

        pub(crate) async fn place(&self, tenant: &TenantContext, total: i64) {
            self.place_as(tenant, Actor::system(), total).await;
        }
    }

    /// Bộ xác thực của test: người dùng theo ID, đổi quyền được giữa chừng (mất quyền).
    #[derive(Clone, Default)]
    pub(crate) struct People(pub Arc<Mutex<HashMap<Uuid, Principal>>>);

    impl People {
        pub(crate) fn set(&self, principal: Principal) {
            self.0.lock().unwrap().insert(principal.user_id(), principal);
        }

        pub(crate) fn remove(&self, id: Uuid) {
            self.0.lock().unwrap().remove(&id);
        }
    }

    #[async_trait::async_trait]
    impl Authenticator for People {
        async fn authenticate(
            &self,
            authorization: Option<&str>,
            _: &autoxide_kernel::tenancy::TenantHint,
        ) -> Result<Principal, Error> {
            let id = authorization
                .and_then(|value| value.strip_prefix("Bearer "))
                .and_then(|token| token.parse::<u128>().ok())
                .map(Uuid::from_u128);
            id.and_then(|id| self.0.lock().unwrap().get(&id).cloned()).ok_or_else(|| {
                Error::unauthenticated("auth.invalid_token", "The token is not valid.")
            })
        }

        async fn principal_of(
            &self,
            _: &TenantContext,
            user_id: Uuid,
        ) -> Result<Option<Principal>, Error> {
            Ok(self.0.lock().unwrap().get(&user_id).cloned())
        }
    }

    /// Nơi lưu idempotency key trong bộ nhớ (như module `idempotency`), cho test chạy lại bước.
    #[derive(Default)]
    pub(crate) struct MemoryStore(Mutex<HashMap<String, (Uuid, Option<StoredResponse>)>>);

    #[async_trait::async_trait]
    impl IdempotencyStore for MemoryStore {
        async fn begin(
            &self,
            _: &TenantContext,
            scope: &str,
            key: &str,
            _: &str,
            _: Timestamp,
        ) -> Result<Begin, Error> {
            let mut keys = self.0.lock().unwrap();
            let entry = format!("{scope}|{key}");
            match keys.get(&entry) {
                Some((_, Some(stored))) => Ok(Begin::Replay(stored.clone())),
                Some((_, None)) => Ok(Begin::InProgress),
                None => {
                    let id = Uuid::now_v7();
                    keys.insert(entry, (id, None));
                    Ok(Begin::Started { id })
                }
            }
        }

        async fn complete(
            &self,
            _: &TenantContext,
            id: Uuid,
            response: StoredResponse,
        ) -> Result<(), Error> {
            for value in self.0.lock().unwrap().values_mut() {
                if value.0 == id {
                    value.1 = Some(response.clone());
                }
            }
            Ok(())
        }

        async fn abandon(&self, _: &TenantContext, id: Uuid) -> Result<(), Error> {
            self.0.lock().unwrap().retain(|_, value| value.0 != id);
            Ok(())
        }
    }
}
