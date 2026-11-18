//! Phân phối dữ liệu khách cha → khách con (bước 38, `tenancy::distribution`) với một module mẫu
//! khai trong test: khách cha ở database chính, khách con ở database riêng và database chung. Chạy
//! trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, OnceLock};

use autoxide_kernel::entitlement::{Entitlements, Limit, TenantEntitlements};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobRegistration, jobs_migration};
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::tenancy::distribution::{
    Applied, DistributionApply, DistributionItem, Distributor, Targets, distribution_migration,
};
use autoxide_kernel::tenancy::migration::adopt;
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use autoxide_testkit::{TestApp, TestDatabase};
use sea_orm::sea_query::{Alias, Expr, OnConflict, Query};
use serde_json::json;
use uuid::Uuid;

const KIND: &str = "samples.distribute";

/// Bảng của module mẫu: bản ghi nhận được giữ `reference` và `source_version` (cột của module).
struct CreateItems;

impl MigrationName for CreateItems {
    fn name(&self) -> &str {
        "m0001_items"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateItems {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("samples_items")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("name", 100))
                    .col(schema::string_len("reference", 200))
                    .col(schema::big_integer("source_version"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("samples_items_reference_idx")
                    .table("samples_items")
                    .unique()
                    .col("tenant_id")
                    .col("reference")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("samples_items").to_owned()).await
    }
}

/// Áp bản ghi vào khách con, idempotent theo `reference` và `source_version`.
struct ApplyItem;

fn col(name: &str) -> Alias {
    Alias::new(name)
}

async fn find(child: &TenantContext, reference: &str) -> Option<(Uuid, String, i64)> {
    let select = Query::select()
        .columns(["id", "name", "source_version"].map(col))
        .from(col("samples_items"))
        .and_where(Expr::col(col("tenant_id")).eq(child.id().as_uuid()))
        .and_where(Expr::col(col("reference")).eq(reference))
        .to_owned();
    let row = child.database().write().query_one(&select).await.unwrap()?;
    Some((
        row.try_get("", "id").unwrap(),
        row.try_get("", "name").unwrap(),
        row.try_get("", "source_version").unwrap(),
    ))
}

#[async_trait::async_trait]
impl DistributionApply for ApplyItem {
    async fn apply(
        &self,
        child: &TenantContext,
        _actor: Actor,
        item: &DistributionItem,
    ) -> Result<Applied, Error> {
        let reference = item.reference();
        let name = item.payload["name"].as_str().unwrap_or_default().to_owned();
        let write = child.database().write();
        let id = Uuid::now_v7();
        let insert = Query::insert()
            .into_table(col("samples_items"))
            .columns(["id", "tenant_id", "name", "reference", "source_version"].map(col))
            .values([
                id.into(),
                child.id().as_uuid().into(),
                name.clone().into(),
                reference.clone().into(),
                item.version.into(),
            ])
            .unwrap()
            .on_conflict(
                OnConflict::columns(["tenant_id", "reference"].map(col)).do_nothing().to_owned(),
            )
            .to_owned();
        if write.execute(&insert).await?.rows_affected() == 1 {
            return Ok(Applied::Created(id));
        }
        let update = Query::update()
            .table(col("samples_items"))
            .value(col("name"), name)
            .value(col("source_version"), item.version)
            .and_where(Expr::col(col("tenant_id")).eq(child.id().as_uuid()))
            .and_where(Expr::col(col("reference")).eq(reference.as_str()))
            .and_where(Expr::col(col("source_version")).lt(item.version))
            .to_owned();
        let changed = write.execute(&update).await?.rows_affected();
        let (id, ..) = find(child, &reference).await.unwrap();
        Ok(if changed == 1 { Applied::Updated(id) } else { Applied::Unchanged(id) })
    }
}

/// Module mẫu `samples`: dùng bộ phân phối của kernel.
#[derive(Clone, Default)]
struct Samples(Arc<OnceLock<Distributor>>);

impl Module for Samples {
    fn name(&self) -> &'static str {
        "samples"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(CreateItems),
            distribution_migration("m0002_distributions", "samples"),
            jobs_migration("m0003_jobs", "samples"),
            adopt("m0004_tenancy_jobs", vec![autoxide_kernel::job::tenant_table("samples")]),
        ]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let _ = self.0.set(context.distributor());
        Ok(())
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.0
            .get()
            .map_or_else(Vec::new, |distributor| vec![distributor.job(KIND, Arc::new(ApplyItem))])
    }
}

/// Quyền dùng giả: khách `blocked` chưa bật module nào.
struct Plans(OnceLock<TenantId>);

#[async_trait::async_trait]
impl Entitlements for Plans {
    async fn snapshot(&self, tenant: TenantId) -> Result<Arc<TenantEntitlements>, Error> {
        if self.0.get() == Some(&tenant) {
            Ok(Arc::new(TenantEntitlements::default()))
        } else {
            Ok(Arc::new(TenantEntitlements::unrestricted()))
        }
    }
    async fn reserve(&self, _: TenantId, _: &Limit, _: i64, _: &str) -> Result<(), Error> {
        Ok(())
    }
    async fn confirm(&self, _: TenantId, _: &Limit, _: &str) -> Result<(), Error> {
        Ok(())
    }
    async fn release(&self, _: TenantId, _: &Limit, _: &str) -> Result<(), Error> {
        Ok(())
    }
    async fn record_usage(&self, _: TenantId, _: &Limit, _: i64, _: &str) -> Result<(), Error> {
        Ok(())
    }
}

struct Chain {
    app: TestApp,
    samples: Samples,
    parent: TenantContext,
    /// Database riêng.
    far: TenantContext,
    /// Database chung với khách cha.
    near: TenantContext,
    stranger: TenantContext,
    plans: Arc<Plans>,
}

async fn chain(test: &TestDatabase) -> Chain {
    let samples = Samples::default();
    let plans = Arc::new(Plans(OnceLock::new()));
    let app = TestApp::builder(test)
        .tenant("parent")
        .child_tenant("far", "parent", true)
        .child_tenant("near", "parent", false)
        .tenant("stranger")
        .provide::<dyn Entitlements>(plans.clone())
        .module(samples.clone())
        .build()
        .await
        .unwrap();
    let tenant = async |slug: &str| app.tenant(slug).await.unwrap();
    Chain {
        parent: tenant("parent").await,
        far: tenant("far").await,
        near: tenant("near").await,
        stranger: tenant("stranger").await,
        app,
        samples,
        plans,
    }
}

impl Chain {
    fn distributor(&self) -> &Distributor {
        self.samples.0.get().unwrap()
    }

    fn item(&self, record: Uuid, version: i64, name: &str) -> DistributionItem {
        DistributionItem::new(
            self.parent.id(),
            "samples.item",
            record,
            version,
            json!({"name": name}),
        )
    }

    async fn publish(&self, item: DistributionItem, targets: Targets) -> Result<Uuid, Error> {
        let transaction = self.parent.database().begin_write().await?;
        let id = self
            .distributor()
            .publish(&transaction, &self.parent, Actor::system(), item, targets)
            .await?;
        transaction.commit().await?;
        Ok(id)
    }

    async fn run_jobs(&self) {
        let worker = self.app.modules().worker(4).unwrap();
        for _ in 0..5 {
            if worker.run_once().await.unwrap() == 0 {
                break;
            }
        }
    }

    async fn count(&self, child: &TenantContext) -> i64 {
        let select = Query::select()
            .expr(Expr::col(col("id")).count())
            .from(col("samples_items"))
            .and_where(Expr::col(col("tenant_id")).eq(child.id().as_uuid()))
            .to_owned();
        let row = child.database().read().query_one(&select).await.unwrap().unwrap();
        row.try_get_by_index::<i64>(0).unwrap()
    }
}

#[tokio::test]
async fn distribute_creates_once_on_rerun() {
    TestDatabase::run(async |test| {
        let chain = chain(test).await;
        let record = Uuid::now_v7();
        let id = chain.publish(chain.item(record, 1, "Shoes"), Targets::AllChildren).await.unwrap();
        chain.run_jobs().await;
        assert_eq!((chain.count(&chain.far).await, chain.count(&chain.near).await), (1, 1));
        let status = chain.distributor().status(&chain.parent, id).await.unwrap().unwrap();
        assert_eq!(status.targets.len(), 2);
        assert!(status.targets.iter().all(
            |target| target.status == "applied" && target.outcome.as_deref() == Some("created")
        ));
        // Gửi lại cùng phiên bản: bản cũ, không thêm đích hay job; chạy lại job: không bản trùng.
        let again =
            chain.publish(chain.item(record, 1, "Shoes"), Targets::AllChildren).await.unwrap();
        assert_eq!(again, id);
        assert_eq!(chain.distributor().retry_failed(&chain.parent, id).await.unwrap(), 0);
        chain.run_jobs().await;
        assert_eq!((chain.count(&chain.far).await, chain.count(&chain.near).await), (1, 1));
        // Phiên bản mới: cập nhật, vẫn một bản.
        let v2 = chain.publish(chain.item(record, 2, "Boots"), Targets::Descendants).await.unwrap();
        chain.run_jobs().await;
        let status = chain.distributor().status(&chain.parent, v2).await.unwrap().unwrap();
        assert!(status.targets.iter().all(|target| target.outcome.as_deref() == Some("updated")));
        let reference = chain.item(record, 2, "").reference();
        assert_eq!(find(&chain.far, &reference).await.unwrap().1, "Boots");
        assert_eq!(chain.count(&chain.far).await, 1);
        assert!(chain.distributor().status(&chain.stranger, id).await.unwrap().is_none());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn crash_after_child_write_marks_applied_on_retry() {
    TestDatabase::run(async |test| {
        let chain = chain(test).await;
        let item = chain.item(Uuid::now_v7(), 1, "Hats");
        let id = chain.publish(item.clone(), Targets::Only(vec![chain.far.id()])).await.unwrap();
        // Con đã ghi (job chết trước khi cha cập nhật dòng đích).
        let Applied::Created(record) =
            ApplyItem.apply(&chain.far, Actor::system(), &item).await.unwrap()
        else {
            panic!("the first apply creates");
        };
        chain.run_jobs().await;
        let status = chain.distributor().status(&chain.parent, id).await.unwrap().unwrap();
        assert_eq!(status.targets[0].status, "applied");
        assert_eq!(status.targets[0].outcome.as_deref(), Some("unchanged"));
        assert_eq!(status.targets[0].applied_record_id, Some(record));
        assert_eq!(chain.count(&chain.far).await, 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn out_of_order_versions_keep_latest() {
    TestDatabase::run(async |test| {
        let chain = chain(test).await;
        let record = Uuid::now_v7();
        let (v1, v2) = (chain.item(record, 1, "Old"), chain.item(record, 2, "New"));
        // v2 tới trước v1.
        ApplyItem.apply(&chain.near, Actor::system(), &v2).await.unwrap();
        let late = ApplyItem.apply(&chain.near, Actor::system(), &v1).await.unwrap();
        assert!(matches!(late, Applied::Unchanged(_)), "{late:?}");
        // Qua bộ phân phối, cả hai phiên bản cùng chờ: còn lại bản mới nhất.
        chain.publish(v2.clone(), Targets::AllChildren).await.unwrap();
        chain.publish(v1, Targets::AllChildren).await.unwrap();
        chain.run_jobs().await;
        for child in [&chain.far, &chain.near] {
            let (_, name, version) = find(child, &v2.reference()).await.unwrap();
            assert_eq!((name.as_str(), version), ("New", 2));
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn child_without_module_is_skipped() {
    TestDatabase::run(async |test| {
        let chain = chain(test).await;
        chain.plans.0.set(chain.far.id()).unwrap();
        let id = chain
            .publish(chain.item(Uuid::now_v7(), 1, "Socks"), Targets::AllChildren)
            .await
            .unwrap();
        chain.run_jobs().await;
        let status = chain.distributor().status(&chain.parent, id).await.unwrap().unwrap();
        let far = status.targets.iter().find(|target| target.tenant_id == chain.far.id()).unwrap();
        assert_eq!(
            (far.status.as_str(), far.last_error.as_deref()),
            ("skipped", Some("module_not_enabled"))
        );
        assert_eq!(chain.count(&chain.far).await, 0);
        let near =
            status.targets.iter().find(|target| target.tenant_id == chain.near.id()).unwrap();
        assert_eq!(near.status, "applied");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn concurrent_publish_same_version_once() {
    TestDatabase::run(async |test| {
        let chain = chain(test).await;
        let item = chain.item(Uuid::now_v7(), 1, "Belts");
        let (a, b) = tokio::join!(
            chain.publish(item.clone(), Targets::AllChildren),
            chain.publish(item.clone(), Targets::AllChildren)
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a, b);
        let status = chain.distributor().status(&chain.parent, a).await.unwrap().unwrap();
        assert_eq!(status.targets.len(), 2);
        chain.run_jobs().await;
        assert_eq!((chain.count(&chain.far).await, chain.count(&chain.near).await), (1, 1));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn non_descendant_target_rejected() {
    TestDatabase::run(async |test| {
        let chain = chain(test).await;
        let item = chain.item(Uuid::now_v7(), 1, "Gloves");
        let error = chain
            .publish(item, Targets::Only(vec![chain.far.id(), chain.stranger.id()]))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "tenancy.distribution_target_invalid");
        assert_eq!(chain.count(&chain.far).await, 0);
    })
    .await
    .unwrap();
}
