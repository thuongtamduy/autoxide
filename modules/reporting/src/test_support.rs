//! Đồ dùng chung cho test của `reporting`: module mẫu `sales` (một event, hai loại số liệu) và cây
//! khách hàng mẫu — cha ở database chính, con ở database khác.

use std::sync::{Arc, OnceLock};

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, Event, EventInfo, Outbox};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::money::Money;
use autoxide_kernel::report::{Fact, FactKind, FactSource, Measure, money_field, org_field};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_testkit::{TestApp, TestAppBuilder};
use serde::Serialize;
use serde_json::json;

use crate::config::ReportingConfig;
use crate::service::Reporting;

pub(crate) const SALES_AMOUNT: FactKind =
    FactKind::new("sales.amount", "Amount sold.", Measure::Amount, &["channel"]);
pub(crate) const SALES_COUNT: FactKind =
    FactKind::new("sales.count", "Number of sales.", Measure::Quantity, &[]);

#[derive(Serialize)]
struct Sold(serde_json::Value);

impl Event for Sold {
    const NAME: &'static str = "sales.sold";
}

fn sold(event: &Delivered) -> Result<Vec<Fact>, Error> {
    let org = org_field(&event.payload);
    let amount = money_field(&event.payload, "amount")?.ok_or_else(|| Error::internal("amount"))?;
    let channel = event.payload["channel"].as_str().unwrap_or("web").to_owned();
    Ok(vec![
        Fact::new(SALES_AMOUNT.kind, event.occurred_at)
            .org(org)
            .amount(amount)
            .dimension("channel", channel),
        Fact::new(SALES_COUNT.kind, event.occurred_at).org(org).quantity(1),
    ])
}

/// Module mẫu `sales`: phát `sales.sold`, khai số liệu cho nó.
#[derive(Clone, Default)]
pub(crate) struct Sales(pub Arc<OnceLock<Outbox>>);

impl Module for Sales {
    fn name(&self) -> &'static str {
        "sales"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            autoxide_kernel::event::outbox_migration("m0001_events", "sales"),
            autoxide_kernel::event::outbox_actor_migration("m0002_actor", "sales"),
            autoxide_kernel::tenancy::migration::adopt(
                "m0003_tenancy",
                vec![autoxide_kernel::event::tenant_table("sales")],
            ),
            autoxide_kernel::event::outbox_access_migration("k0001_access", "sales"),
        ]
    }

    fn events(&self) -> Vec<EventInfo> {
        vec![EventInfo::new("sales.sold", "A sale (`org_id`, `amount`, `channel`).")]
    }

    fn facts(&self) -> Vec<FactSource> {
        vec![FactSource::new("sales.sold", vec![SALES_AMOUNT, SALES_COUNT], sold)]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let _ = self.0.set(context.outbox()?);
        Ok(())
    }
}

impl Sales {
    /// Ghi một lần bán ở khách `tenant` (event trong transaction, như module thật).
    pub(crate) async fn sell(&self, tenant: &TenantContext, org: Option<OrgId>, amount: Money) {
        let outbox = self.0.get().unwrap();
        let transaction = tenant.database().begin_write().await.unwrap();
        let payload = json!({"org_id": org, "amount": amount, "channel": "web"});
        outbox.record(&transaction, tenant, Actor::system(), None, &Sold(payload)).await.unwrap();
        transaction.commit().await.unwrap();
    }
}

/// Cây mẫu: `parent` (database chính) có hai con `shop_a` (database riêng) và `shop_b` (database
/// chung); `stranger` là khách ngoài cây.
pub(crate) fn tree<'a>(builder: TestAppBuilder<'a>, sales: &Sales) -> TestAppBuilder<'a> {
    builder
        .tenant("parent")
        .child_tenant("shop-a", "parent", true)
        .child_tenant("shop-b", "parent", false)
        .tenant("stranger")
        .module(sales.clone())
        .module(crate::module())
}

/// Service dựng trên sổ database của app (như module thật dựng lúc khởi động).
pub(crate) fn service(app: &TestApp) -> Reporting {
    let sources = vec![("sales", Sales::default().facts().remove(0))];
    Reporting::new(app.registry().clone(), None, None, &sources, ReportingConfig::default())
}

/// Giao event và đẩy số liệu trên mọi database cho tới khi hết việc.
pub(crate) async fn settle(app: &TestApp, service: &Reporting) {
    let dispatcher = app.modules().dispatcher().unwrap();
    while dispatcher.run_once().await.unwrap() > 0 {}
    for (_, _, database) in app.registry().databases().await.unwrap() {
        service.forward_database(&database).await.unwrap();
    }
}
