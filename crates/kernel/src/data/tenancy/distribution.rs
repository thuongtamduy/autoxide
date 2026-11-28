//! Distributing data from a parent tenant to child tenants (step 38, D-20261118-step38-distribution): a parent
//! tenant (chain, head office) pushes one of its records (catalog, price list…) down to its child tenants,
//! including child tenants that have their own database.
//!
//! 1. The module adds [`distribution_migration`] to its migrations: the table `<module>_distributions` (the
//!    distribution: which record, which version) and `<module>_distribution_targets` (one row per child
//!    tenant: applied yet, result). The tables live in the parent tenant's database.
//! 2. Inside the WRITE transaction of the parent's record, the module calls [`Distributor::publish`]: it
//!    records the distribution, one target row per chosen child tenant, and one `<module>.distribute` job per
//!    target. Sending the same (record, version) again returns the old distribution and adds nothing.
//! 3. The module declares the runner with [`Distributor::job`] using its own [`DistributionApply`]. The job
//!    (running in the parent tenant's database) opens the child tenant's context through the database
//!    registry, checks the child is still a descendant and has the module enabled, calls `apply` (writes the
//!    child's database through the child's service), then updates the target row.
//!
//! The two databases share no transaction: `apply` MUST be idempotent (create with [`reference`], D-184;
//! update only when the new version is greater than the version stored at the child). If the process dies
//! after the child has written but before the parent updates the target row, the rerun finds the record
//! already at the child, gets [`Applied::Unchanged`], and the target becomes `applied`.

use crate::time::Timestamp;
use std::sync::Arc;

use crate::migration::prelude::*;
use crate::migration::schema;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, OnConflict, Order, Query};
use sea_orm::{ConnectionTrait, QueryResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::{DatabaseRegistry, TenantContext, TenantId};
use crate::entitlement::EntitlementsHandle;
use crate::error::{Error, ErrorKind};
use crate::event::Actor;
use crate::job::{EnqueueOptions, JobContext, JobHandler, JobQueue, JobRegistration};

/// A parent tenant record to apply to a child tenant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DistributionItem {
    /// The sending parent tenant.
    pub parent: TenantId,
    /// Kind of the module record, e.g. `catalog.category`.
    pub record_kind: String,
    /// Record ID at the parent tenant.
    pub record_id: Uuid,
    /// Version of the record at the parent tenant; increases on every change.
    pub version: i64,
    /// Content to apply (module JSON).
    pub payload: Value,
}

impl DistributionItem {
    pub fn new(
        parent: TenantId,
        record_kind: impl Into<String>,
        record_id: Uuid,
        version: i64,
        payload: Value,
    ) -> Self {
        DistributionItem { parent, record_kind: record_kind.into(), record_id, version, payload }
    }

    /// Reference code (D-184) of the record at the child tenant: the same record of the same parent always
    /// yields the same code, across versions too; `apply` finds the already received record by this code.
    pub fn reference(&self) -> String {
        format!("dist:{}:{}:{}", self.parent, self.record_kind, self.record_id)
    }
}

/// Result of applying a record to a child tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    Created(Uuid),
    Updated(Uuid),
    /// An equal or newer version already exists (a rerun, or a version that arrived late).
    Unchanged(Uuid),
    /// Not applied, with a short reason (e.g. the child tenant set its own value).
    Skipped {
        reason: String,
    },
}

/// Implemented by a module to apply a parent tenant record to a child tenant; calls the module's own
/// service, in the child's context (the child's events, audit).
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait DistributionApply: Send + Sync {
    /// Must be idempotent: create with `item.reference()`, update only when `item.version` is greater than the
    /// source version stored at the child (`UPDATE … WHERE source_version < ?`).
    async fn apply(
        &self,
        child: &TenantContext,
        actor: Actor,
        item: &DistributionItem,
    ) -> Result<Applied, Error>;
}

/// Child tenants that receive a distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Targets {
    /// Direct children of the parent tenant.
    AllChildren,
    /// Every descendant tenant (child, grandchild…).
    Descendants,
    /// Exactly these tenants; a tenant that is not a descendant → `tenancy.distribution_target_invalid`.
    Only(Vec<TenantId>),
}

/// One child tenant in a distribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct TargetStatus {
    pub tenant_id: TenantId,
    /// `pending`, `applied`, `skipped` hay `failed`.
    pub status: String,
    /// `created`, `updated`, `unchanged` or `skipped`; `None` when not yet applied.
    pub outcome: Option<String>,
    pub applied_record_id: Option<Uuid>,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub applied_at: Option<Timestamp>,
}

/// A distribution and its state at each child tenant.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct DistributionStatus {
    pub id: Uuid,
    pub record_kind: String,
    pub record_id: Uuid,
    pub version: i64,
    pub created_at: Timestamp,
    pub targets: Vec<TargetStatus>,
}

/// Payload of the `<module>.distribute` job.
#[derive(Debug, Serialize, Deserialize)]
struct DistributeJob {
    distribution_id: Uuid,
    target_tenant_id: Uuid,
}

/// The distributor of a module. Obtained with `StartContext::distributor()`; cheap to clone.
#[derive(Clone)]
pub struct Distributor {
    module: &'static str,
    registry: DatabaseRegistry,
    entitlements: EntitlementsHandle,
}

impl std::fmt::Debug for Distributor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Distributor").field("module", &self.module).finish_non_exhaustive()
    }
}

fn now() -> Timestamp {
    Timestamp::now()
}

fn col(name: &str) -> Alias {
    Alias::new(name)
}

impl Distributor {
    /// The distributor of `module` (inside the program, obtained with `StartContext::distributor()`).
    pub fn new(
        module: &'static str,
        registry: DatabaseRegistry,
        entitlements: EntitlementsHandle,
    ) -> Self {
        Distributor { module, registry, entitlements }
    }

    fn distributions(&self) -> String {
        format!("{}_distributions", self.module)
    }

    fn targets(&self) -> String {
        format!("{}_distribution_targets", self.module)
    }

    /// Job kind that applies a distribution: `<module>.distribute`.
    pub fn job_kind(&self) -> String {
        format!("{}.distribute", self.module)
    }

    /// Records the distribution `item` of parent tenant `parent` to `targets`, on `connection`: the parent
    /// tenant's WRITE transaction (the same transaction as the record change). Returns the distribution ID;
    /// if the same (kind, record, version) already exists, returns the old ID and adds no target or job.
    pub async fn publish<C: crate::db::Executor>(
        &self,
        connection: &C,
        parent: &TenantContext,
        actor: Actor,
        item: DistributionItem,
        targets: Targets,
    ) -> Result<Uuid, Error> {
        let connection = &crate::db::Via::of(connection);
        if item.parent != parent.id() {
            return Err(Error::internal("a distribution item must name the publishing tenant"));
        }
        let chosen = self.resolve_targets(parent.id(), targets).await?;
        let id = Uuid::now_v7();
        let at = now();
        let mut insert = Query::insert();
        insert
            .into_table(col(&self.distributions()))
            .columns(
                [
                    "tenant_id",
                    "id",
                    "record_kind",
                    "record_id",
                    "version",
                    "payload",
                    "created_by",
                    "created_at",
                ]
                .map(col),
            )
            .values([
                parent.id().as_uuid().into(),
                id.into(),
                item.record_kind.clone().into(),
                item.record_id.into(),
                item.version.into(),
                item.payload.to_string().into(),
                actor.user_id.into(),
                at.into(),
            ])
            .map_err(|error| Error::internal(format!("distribution insert: {error}")))?
            .on_conflict(
                OnConflict::columns(["tenant_id", "record_kind", "record_id", "version"].map(col))
                    .do_nothing()
                    .to_owned(),
            );
        if connection.execute(&insert).await?.rows_affected() == 0 {
            let select = Query::select()
                .column(col("id"))
                .from(col(&self.distributions()))
                .and_where(Expr::col(col("tenant_id")).eq(parent.id().as_uuid()))
                .and_where(Expr::col(col("record_kind")).eq(item.record_kind.as_str()))
                .and_where(Expr::col(col("record_id")).eq(item.record_id))
                .and_where(Expr::col(col("version")).eq(item.version))
                .to_owned();
            let row = connection.query_one(&select).await?.ok_or_else(|| {
                Error::internal("a distribution vanished while it was being published")
            })?;
            return Ok(row.try_get::<Uuid>("", "id")?);
        }
        let queue = JobQueue::new(self.module);
        for child in chosen {
            let mut insert = Query::insert();
            insert
                .into_table(col(&self.targets()))
                .columns(
                    [
                        "tenant_id",
                        "id",
                        "distribution_id",
                        "target_tenant_id",
                        "status",
                        "attempts",
                    ]
                    .map(col),
                )
                .values([
                    parent.id().as_uuid().into(),
                    Uuid::now_v7().into(),
                    id.into(),
                    child.as_uuid().into(),
                    "pending".into(),
                    0i32.into(),
                ])
                .map_err(|error| Error::internal(format!("distribution target: {error}")))?;
            connection.execute(&insert).await?;
            self.enqueue(&queue, connection, parent.id(), id, child, None).await?;
        }
        Ok(id)
    }

    async fn enqueue<C: ConnectionTrait>(
        &self,
        queue: &JobQueue,
        connection: &C,
        parent: TenantId,
        distribution: Uuid,
        child: TenantId,
        retry: Option<i64>,
    ) -> Result<(), Error> {
        let payload = serde_json::to_value(DistributeJob {
            distribution_id: distribution,
            target_tenant_id: child.as_uuid(),
        })
        .map_err(Error::internal)?;
        let key = match retry {
            None => format!("dist:{distribution}:{child}"),
            Some(round) => format!("dist:{distribution}:{child}:retry:{round}"),
        };
        queue
            .enqueue_value(
                connection,
                parent,
                &self.job_kind(),
                payload,
                EnqueueOptions::default().unique_key(key),
            )
            .await?;
        Ok(())
    }

    /// Descendant tenants chosen; an unknown tenant in `Only` → `tenancy.distribution_target_invalid`.
    async fn resolve_targets(
        &self,
        parent: TenantId,
        targets: Targets,
    ) -> Result<Vec<TenantId>, Error> {
        let below: Vec<_> = self
            .registry
            .tenant_descendants(parent)
            .await?
            .into_iter()
            .filter(|child| child.status != "deleted")
            .collect();
        Ok(match targets {
            Targets::AllChildren => {
                below.iter().filter(|child| child.parent_id == parent).map(|c| c.id).collect()
            }
            Targets::Descendants => below.iter().map(|child| child.id).collect(),
            Targets::Only(list) => {
                let mut chosen = Vec::new();
                for tenant in list {
                    if !below.iter().any(|child| child.id == tenant) {
                        return Err(target_invalid(tenant));
                    }
                    if !chosen.contains(&tenant) {
                        chosen.push(tenant);
                    }
                }
                chosen
            }
        })
    }

    /// The distribution `distribution` of parent tenant `parent`; `None` when the tenant has no such distribution.
    pub async fn status(
        &self,
        parent: &TenantContext,
        distribution: Uuid,
    ) -> Result<Option<DistributionStatus>, Error> {
        let read = parent.database().read();
        let select = Query::select()
            .columns(["id", "record_kind", "record_id", "version", "created_at"].map(col))
            .from(col(&self.distributions()))
            .and_where(Expr::col(col("tenant_id")).eq(parent.id().as_uuid()))
            .and_where(Expr::col(col("id")).eq(distribution))
            .to_owned();
        let Some(row) = read.query_one(&select).await? else { return Ok(None) };
        let select = Query::select()
            .columns(
                [
                    "target_tenant_id",
                    "status",
                    "outcome",
                    "applied_record_id",
                    "attempts",
                    "last_error",
                    "applied_at",
                ]
                .map(col),
            )
            .from(col(&self.targets()))
            .and_where(Expr::col(col("tenant_id")).eq(parent.id().as_uuid()))
            .and_where(Expr::col(col("distribution_id")).eq(distribution))
            .order_by(col("target_tenant_id"), Order::Asc)
            .to_owned();
        let targets = read
            .query_all(&select)
            .await?
            .iter()
            .map(target_status)
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(Some(DistributionStatus {
            id: row.try_get("", "id")?,
            record_kind: row.try_get("", "record_kind")?,
            record_id: row.try_get("", "record_id")?,
            version: row.try_get("", "version")?,
            created_at: row.try_get("", "created_at")?,
            targets,
        }))
    }

    /// Moves the `failed` targets of the distribution back to `pending` and reruns them; returns the number rerun.
    pub async fn retry_failed(
        &self,
        parent: &TenantContext,
        distribution: Uuid,
    ) -> Result<u64, Error> {
        let transaction = parent.database().begin_write().await?;
        let select = Query::select()
            .column(col("target_tenant_id"))
            .from(col(&self.targets()))
            .and_where(Expr::col(col("tenant_id")).eq(parent.id().as_uuid()))
            .and_where(Expr::col(col("distribution_id")).eq(distribution))
            .and_where(Expr::col(col("status")).eq("failed"))
            .to_owned();
        let failed = transaction.query_all(&select).await?;
        let queue = JobQueue::new(self.module);
        let round = Timestamp::now().as_unix_millis();
        for row in &failed {
            let child = TenantId::from_uuid(row.try_get("", "target_tenant_id")?);
            let update = Query::update()
                .table(col(&self.targets()))
                .value(col("status"), "pending")
                .and_where(Expr::col(col("tenant_id")).eq(parent.id().as_uuid()))
                .and_where(Expr::col(col("distribution_id")).eq(distribution))
                .and_where(Expr::col(col("target_tenant_id")).eq(child.as_uuid()))
                .to_owned();
            transaction.execute(&update).await?;
            self.enqueue(&queue, &transaction, parent.id(), distribution, child, Some(round))
                .await?;
        }
        transaction.commit().await?;
        Ok(failed.len() as u64)
    }

    /// Runner of the `<module>.distribute` job for `Module::jobs`: `kind` must be
    /// [`job_kind`](Self::job_kind) (a `&'static str` constant of the module, e.g. `"catalog.distribute"`).
    pub fn job(&self, kind: &'static str, apply: Arc<dyn DistributionApply>) -> JobRegistration {
        JobRegistration::new(kind, Arc::new(DistributeRunner { distributor: self.clone(), apply }))
    }

    async fn target_row(
        &self,
        parent: &TenantContext,
        job: &DistributeJob,
    ) -> Result<Option<(String, i32)>, Error> {
        let select = Query::select()
            .columns(["status", "attempts"].map(col))
            .from(col(&self.targets()))
            .and_where(Expr::col(col("tenant_id")).eq(parent.id().as_uuid()))
            .and_where(Expr::col(col("distribution_id")).eq(job.distribution_id))
            .and_where(Expr::col(col("target_tenant_id")).eq(job.target_tenant_id))
            .to_owned();
        let row = parent.database().read().query_one(&select).await?;
        row.map(|row| Ok((row.try_get("", "status")?, row.try_get("", "attempts")?))).transpose()
    }

    async fn item(
        &self,
        parent: &TenantContext,
        distribution: Uuid,
    ) -> Result<Option<(DistributionItem, Option<Uuid>)>, Error> {
        let select = Query::select()
            .columns(["record_kind", "record_id", "version", "payload", "created_by"].map(col))
            .from(col(&self.distributions()))
            .and_where(Expr::col(col("tenant_id")).eq(parent.id().as_uuid()))
            .and_where(Expr::col(col("id")).eq(distribution))
            .to_owned();
        let Some(row) = parent.database().read().query_one(&select).await? else {
            return Ok(None);
        };
        let payload: String = row.try_get("", "payload")?;
        let item = DistributionItem {
            parent: parent.id(),
            record_kind: row.try_get("", "record_kind")?,
            record_id: row.try_get("", "record_id")?,
            version: row.try_get("", "version")?,
            payload: serde_json::from_str(&payload).map_err(Error::internal)?,
        };
        Ok(Some((item, row.try_get("", "created_by")?)))
    }

    /// Records the result of one target (in the parent tenant's database).
    async fn finish(
        &self,
        parent: &TenantContext,
        job: &DistributeJob,
        outcome: Result<Applied, (String, bool)>,
    ) -> Result<(), Error> {
        let mut update = Query::update();
        update
            .table(col(&self.targets()))
            .value(col("attempts"), Expr::col(col("attempts")).add(1));
        match outcome {
            Ok(applied) => {
                let (status, outcome, record) = match applied {
                    Applied::Created(id) => ("applied", "created", Some(id)),
                    Applied::Updated(id) => ("applied", "updated", Some(id)),
                    Applied::Unchanged(id) => ("applied", "unchanged", Some(id)),
                    Applied::Skipped { reason } => {
                        update.value(col("last_error"), short(&reason));
                        ("skipped", "skipped", None)
                    }
                };
                update
                    .value(col("status"), status)
                    .value(col("outcome"), outcome)
                    .value(col("applied_record_id"), record)
                    .value(col("applied_at"), now());
            }
            Err((error, last)) => {
                update.value(col("last_error"), short(&error));
                if last {
                    update.value(col("status"), "failed");
                }
            }
        }
        update
            .and_where(Expr::col(col("tenant_id")).eq(parent.id().as_uuid()))
            .and_where(Expr::col(col("distribution_id")).eq(job.distribution_id))
            .and_where(Expr::col(col("target_tenant_id")).eq(job.target_tenant_id));
        parent.database().write().execute(&update).await?;
        Ok(())
    }

    async fn run(&self, apply: &dyn DistributionApply, job: &JobContext) -> Result<(), Error> {
        let parent = job
            .tenant
            .clone()
            .ok_or_else(|| Error::internal("a distribution job must belong to a tenant"))?;
        let input: DistributeJob =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        let Some((status, _)) = self.target_row(&parent, &input).await? else { return Ok(()) };
        if status == "applied" || status == "skipped" {
            return Ok(());
        }
        let Some((item, created_by)) = self.item(&parent, input.distribution_id).await? else {
            return Ok(());
        };
        let child_id = TenantId::from_uuid(input.target_tenant_id);
        let skipped = |reason: &str| Ok(Applied::Skipped { reason: reason.to_owned() });
        let outcome = match self.registry.tenant(child_id).await {
            Err(error) if error.code() == "tenant_not_found" => skipped("tenant_not_found"),
            Err(error) => Err(error),
            Ok(child) => {
                let ancestors = self.registry.tenant_ancestors(child_id).await?;
                if !ancestors.contains(&parent.id()) {
                    skipped("not_descendant")
                } else if !self.entitlements.module_enabled(child_id, self.module).await? {
                    skipped("module_not_enabled")
                } else {
                    super::ensure_serving(&child)?;
                    let actor = created_by.map_or_else(Actor::system, Actor::user);
                    apply.apply(&child, actor, &item).await
                }
            }
        };
        match outcome {
            Ok(applied) => self.finish(&parent, &input, Ok(applied)).await,
            Err(error) => {
                let last = job.attempt >= crate::job::DEFAULT_MAX_ATTEMPTS;
                self.finish(&parent, &input, Err((error.to_string(), last))).await?;
                Err(error)
            }
        }
    }
}

/// Error string stored in the column: truncated to 1000 characters.
fn short(text: &str) -> String {
    text.chars().take(1000).collect()
}

fn target_status(row: &QueryResult) -> Result<TargetStatus, Error> {
    Ok(TargetStatus {
        tenant_id: TenantId::from_uuid(row.try_get("", "target_tenant_id")?),
        status: row.try_get("", "status")?,
        outcome: row.try_get("", "outcome")?,
        applied_record_id: row.try_get("", "applied_record_id")?,
        attempts: row.try_get("", "attempts")?,
        last_error: row.try_get("", "last_error")?,
        applied_at: row.try_get("", "applied_at")?,
    })
}

struct DistributeRunner {
    distributor: Distributor,
    apply: Arc<dyn DistributionApply>,
}

#[async_trait::async_trait]
impl JobHandler for DistributeRunner {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.distributor.run(self.apply.as_ref(), job).await
    }
}

/// `tenancy.distribution_target_invalid` (422): the target tenant is not a descendant of the sending tenant.
pub fn target_invalid(tenant: TenantId) -> Error {
    Error::new(
        ErrorKind::Validation,
        "tenancy.distribution_target_invalid",
        "The tenant `{tenant}` is not a child of the sending tenant.",
    )
    .with_param("tenant", tenant.to_string())
}

/// Migration that creates the distribution tables `<module>_distributions` and `<module>_distribution_targets`
/// of `module`, like [`outbox_migration`](crate::event::outbox_migration).
pub fn distribution_migration(name: &'static str, module: &'static str) -> Box<dyn MigrationTrait> {
    Box::new(DistributionMigration { name, module })
}

struct DistributionMigration {
    name: &'static str,
    module: &'static str,
}

impl MigrationName for DistributionMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for DistributionMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let distributions = format!("{}_distributions", self.module);
        let targets = format!("{}_distribution_targets", self.module);
        manager
            .create_table(
                Table::create()
                    .table(col(&distributions))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("record_kind", 64))
                    .col(schema::uuid("record_id"))
                    .col(schema::big_integer("version"))
                    // JSON dạng chữ: chạy giống nhau trên SQLite và Postgres.
                    .col(schema::text("payload"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name(format!("{distributions}_record_idx"))
                    .table(col(&distributions))
                    .unique()
                    .col("tenant_id")
                    .col("record_kind")
                    .col("record_id")
                    .col("version")
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(col(&targets))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("distribution_id"))
                    .col(schema::uuid("target_tenant_id"))
                    .col(schema::string_len("status", 16))
                    .col(schema::uuid_null("applied_record_id"))
                    .col(schema::string_len_null("outcome", 16))
                    .col(schema::integer("attempts"))
                    .col(schema::text_null("last_error"))
                    .col(schema::timestamp_with_time_zone_null("applied_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name(format!("{targets}_target_idx"))
                    .table(col(&targets))
                    .unique()
                    .col("tenant_id")
                    .col("distribution_id")
                    .col("target_tenant_id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            format!("{}_distribution_targets", self.module),
            format!("{}_distributions", self.module),
        ] {
            manager.drop_table(Table::drop().table(col(&table)).to_owned()).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ma_tham_chieu_on_dinh_qua_phien_ban() {
        let parent = TenantId::from_uuid(Uuid::now_v7());
        let record = Uuid::now_v7();
        let v1 = DistributionItem::new(parent, "sample.item", record, 1, Value::Null);
        let v2 = DistributionItem::new(parent, "sample.item", record, 2, Value::Null);
        assert_eq!(v1.reference(), v2.reference());
        assert_eq!(v1.reference(), format!("dist:{parent}:sample.item:{record}"));
    }
}
