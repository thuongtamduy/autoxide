//! Test đường đi của số liệu (rút → đẩy → kho của cây), trên cả SQLite và Postgres: cha ở database
//! chính, con ở database khác (testkit cấp hai database trống cùng loại).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Delivered;
use autoxide_kernel::module::Module;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::report::Fact;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_organizations::api::{EffectiveSetting, OrgDirectory, Unit};
use autoxide_testkit::{TestApp, TestDatabase};
use chrono::{DateTime, NaiveDate, Utc};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Statement};
use serde_json::json;

use crate::config::ReportingConfig;
use crate::entity::{daily, fact, local_fact};
use crate::service::{Reporting, SummaryQuery};
use crate::test_support::{SALES_AMOUNT, Sales, service, settle, tree};

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::VND)
}

/// Tổng tiền (tiền `currency`) của `sales.amount` mà người `principal` ở khách `viewer` thấy.
async fn total(service: &Reporting, viewer: &TenantContext, principal: &Principal) -> i64 {
    let mut query = SummaryQuery::default();
    query.filter.kind = Some(SALES_AMOUNT.kind.to_owned());
    let summary = service.summary(viewer, principal, &query).await.unwrap();
    summary.totals.by_currency.iter().map(|total| total.amount_minor).sum()
}

fn boss() -> Principal {
    Principal::new(uuid::Uuid::now_v7(), ["reporting.read".to_owned()])
}

async fn local_count(tenant: &TenantContext) -> u64 {
    local_fact::Entity::find()
        .filter(local_fact::Column::TenantId.eq(tenant.id().as_uuid()))
        .count(tenant.database().read())
        .await
        .unwrap()
}

#[tokio::test]
async fn fact_extracted_once_on_redelivery() {
    TestDatabase::run(async |test| {
        let sales = Sales::default();
        let app = tree(TestApp::builder(test), &sales).build().await.unwrap();
        let parent = app.tenant("parent").await.unwrap();
        sales.sell(&parent, None, vnd(1000)).await;
        let dispatcher = app.modules().dispatcher().unwrap();
        while dispatcher.run_once().await.unwrap() > 0 {}
        // Một event, hai số liệu (tiền, số lần).
        assert_eq!(local_count(&parent).await, 2);
        // Giao lại đúng event đó (bộ phát giao ít nhất một lần): không thêm dòng nào.
        let row = local_fact::Entity::find().one(parent.database().read()).await.unwrap().unwrap();
        let event = Delivered::new(
            parent.clone(),
            "sales",
            "sales.sold",
            json!({"amount": vnd(1000), "channel": "web"}),
        )
        .id(row.source_event_id);
        let source = Sales::default().facts().remove(0);
        let reporting = service(&app);
        reporting.record_local(&parent, event.id, source.extract(&event).unwrap()).await.unwrap();
        assert_eq!(local_count(&parent).await, 2);
        // Số liệu không nêu nút thì thuộc nút gốc của khách.
        assert_eq!(row.org_id, parent.id().as_uuid());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn forward_is_idempotent_after_crash() {
    TestDatabase::run(async |test| {
        let sales = Sales::default();
        let app = tree(TestApp::builder(test), &sales).build().await.unwrap();
        let (parent, shop_a) =
            (app.tenant("parent").await.unwrap(), app.tenant("shop-a").await.unwrap());
        let reporting = service(&app);
        sales.sell(&shop_a, None, vnd(700)).await;
        sales.sell(&parent, None, vnd(300)).await;
        settle(&app, &reporting).await;
        assert_eq!(total(&reporting, &parent, &boss()).await, 1000);
        // "Chết" sau khi kho đã nhận mà trước khi nguồn đặt `forwarded_at`: như đẩy lại mọi dòng.
        let manager = Principal::new(uuid::Uuid::now_v7(), ["reporting.manage".to_owned()]);
        let requeued =
            reporting.rebuild(&parent, &manager, Some(shop_a.id().as_uuid())).await.unwrap();
        assert_eq!(requeued, 2);
        settle(&app, &reporting).await;
        assert_eq!(total(&reporting, &parent, &boss()).await, 1000, "counted once");
        let facts = fact::Entity::find().count(parent.database().read()).await.unwrap();
        assert_eq!(facts, 4);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn forward_survives_root_maintenance() {
    TestDatabase::run(async |test| {
        let sales = Sales::default();
        let app = tree(TestApp::builder(test), &sales).build().await.unwrap();
        let (parent, shop_a) =
            (app.tenant("parent").await.unwrap(), app.tenant("shop-a").await.unwrap());
        let reporting = service(&app);
        let status = |status: &'static str| {
            let control = app.database().clone();
            let id = parent.id();
            let registry = app.registry().clone();
            async move {
                let backend = control.write().get_database_backend();
                control
                    .write()
                    .execute_raw(Statement::from_sql_and_values(
                        backend,
                        "UPDATE kernel_tenants SET status = $1 WHERE id = $2",
                        [status.into(), id.as_uuid().into()],
                    ))
                    .await
                    .unwrap();
                registry.invalidate(id);
            }
        };
        status("maintenance").await;
        sales.sell(&shop_a, None, vnd(500)).await;
        settle(&app, &reporting).await;
        // Kho của cây đang bảo trì: số liệu chờ ở database của con, không mất.
        let pending = local_fact::Entity::find()
            .filter(local_fact::Column::ForwardedAt.is_null())
            .count(shop_a.database().read())
            .await
            .unwrap();
        assert_eq!(pending, 2);
        let mut query = SummaryQuery::default();
        query.filter.kind = Some(SALES_AMOUNT.kind.to_owned());
        let error = reporting.summary(&parent, &boss(), &query).await.unwrap_err();
        assert_eq!(error.code(), "reporting.store_unavailable");
        status("active").await;
        let parent = app.tenant("parent").await.unwrap();
        settle(&app, &reporting).await;
        assert_eq!(total(&reporting, &parent, &boss()).await, 500);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn child_on_other_database_appears_within_forward_interval() {
    TestDatabase::run(async |test| {
        let sales = Sales::default();
        let app = tree(TestApp::builder(test), &sales)
            .user_in("parent", "boss", 1, &["reporting.read"])
            .build()
            .await
            .unwrap();
        let (parent, shop_a) =
            (app.tenant("parent").await.unwrap(), app.tenant("shop-a").await.unwrap());
        assert_ne!(parent.placement(), shop_a.placement(), "the child has its own database");
        sales.sell(&shop_a, None, vnd(1200)).await;
        let dispatcher = app.modules().dispatcher().unwrap();
        while dispatcher.run_once().await.unwrap() > 0 {}
        // Lịch `reporting.forward` của worker (mỗi database một job) đưa số liệu lên kho của cha.
        let worker = app.modules().worker(4).unwrap();
        worker.run_once().await.unwrap();
        let response = app
            .get(&format!(
                "/api/v1/reporting/summary?kind=sales.amount&group_by=tenant&tenant_ids={}",
                shop_a.id()
            ))
            .token("boss")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{}", response.text());
        let body = response.json();
        assert_eq!(body["data"]["groups"][0]["key"], shop_a.id().to_string());
        assert_eq!(body["data"]["totals"]["by_currency"][0]["amount"], "1200");
        assert_eq!(body["data"]["totals"]["by_currency"][0]["currency"], "VND");
        // Kho nằm ở database của khách gốc, không ở database của con.
        let at_child = fact::Entity::find().count(shop_a.database().read()).await.unwrap();
        assert_eq!(at_child, 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn daily_rollup_equals_sum_of_facts() {
    TestDatabase::run(async |test| {
        let sales = Sales::default();
        let app = tree(TestApp::builder(test), &sales)
            .child_tenant("shop-c", "parent", true)
            .child_tenant("shop-d", "shop-a", false)
            .build()
            .await
            .unwrap();
        let parent = app.tenant("parent").await.unwrap();
        let mut shops = Vec::new();
        for slug in ["shop-a", "shop-b", "shop-c", "shop-d"] {
            shops.push(app.tenant(slug).await.unwrap());
        }
        let reporting = Arc::new(service(&app));
        // 200 số liệu ghi đồng thời từ 4 khách con (hai database riêng, hai chung với cha).
        let writes = (0..200i64).map(|index| {
            let reporting = reporting.clone();
            let shop = shops[usize::try_from(index % 4).unwrap()].clone();
            async move {
                let fact = Fact::new(SALES_AMOUNT.kind, Utc::now())
                    .amount(vnd(index + 1))
                    .dimension("channel", "web");
                reporting.record_local(&shop, uuid::Uuid::now_v7(), vec![fact]).await
            }
        });
        for result in futures_util::future::join_all(writes).await {
            result.unwrap();
        }
        // Hai lần đẩy chạy cùng lúc trên mỗi database vẫn không đếm hai lần.
        for (_, _, database) in app.registry().databases().await.unwrap() {
            let (a, b) = tokio::join!(
                reporting.forward_database(&database),
                reporting.forward_database(&database)
            );
            a.unwrap();
            b.unwrap();
        }
        let expected: i64 = (1..=200).sum();
        assert_eq!(total(&reporting, &parent, &boss()).await, expected);
        let rows = daily::Entity::find().all(parent.database().read()).await.unwrap();
        assert_eq!(rows.iter().map(|row| row.amount_minor).sum::<i64>(), expected);
        assert_eq!(rows.iter().map(|row| row.fact_count).sum::<i64>(), 200);
        let facts = fact::Entity::find().all(parent.database().read()).await.unwrap();
        assert_eq!(facts.iter().filter_map(|row| row.amount_minor).sum::<i64>(), expected);
    })
    .await
    .unwrap();
}

/// Cây tổ chức giả: mọi nút đặt múi giờ Việt Nam.
struct VietnamTime;

#[async_trait::async_trait]
impl OrgDirectory for VietnamTime {
    async fn unit(&self, _: &TenantContext, _: OrgId) -> Result<Option<Unit>, Error> {
        Ok(None)
    }
    async fn root(&self, _: &TenantContext) -> Result<Unit, Error> {
        Err(Error::internal("not used"))
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
        Ok(Some(EffectiveSetting::new(key, Some(json!("Asia/Ho_Chi_Minh")), Some(id), false)))
    }
}

#[tokio::test]
async fn local_day_uses_org_timezone() {
    TestDatabase::run(async |test| {
        let sales = Sales::default();
        let app = tree(TestApp::builder(test), &sales).build().await.unwrap();
        let shop = app.tenant("shop-a").await.unwrap();
        let sources = vec![("sales", Sales::default().facts().remove(0))];
        let reporting = Reporting::new(
            app.registry().clone(),
            Some(Arc::new(VietnamTime)),
            None,
            &sources,
            ReportingConfig::default(),
        );
        let at = |text: &str| DateTime::parse_from_rfc3339(text).unwrap().to_utc();
        // 23:30 ngày 31/10 và 00:30 ngày 1/11 giờ Việt Nam (UTC+7).
        for (moment, amount) in [("2026-10-31T16:30:00Z", 1), ("2026-10-31T17:30:00Z", 2)] {
            let fact = Fact::new(SALES_AMOUNT.kind, at(moment)).amount(vnd(amount));
            reporting.record_local(&shop, uuid::Uuid::now_v7(), vec![fact]).await.unwrap();
        }
        let rows = local_fact::Entity::find().all(shop.database().read()).await.unwrap();
        let day = |amount: i64| {
            rows.iter().find(|row| row.amount_minor == Some(amount)).unwrap().local_day
        };
        assert_eq!(day(1), NaiveDate::from_ymd_opt(2026, 10, 31).unwrap());
        assert_eq!(day(2), NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
    })
    .await
    .unwrap();
}
