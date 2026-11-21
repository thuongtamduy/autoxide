//! Phân phối dữ liệu khách cha → khách con (bước 38, D-20261118-step38-distribution): khách cha
//! (chuỗi, trụ sở) đẩy một bản ghi của mình (danh mục, bảng giá…) xuống các khách con — kể cả khách
//! con có database riêng.
//!
//! 1. Module thêm [`distribution_migration`] vào migration của mình: bảng `<module>_distributions`
//!    (bản phân phối: bản ghi nào, phiên bản mấy) và `<module>_distribution_targets` (mỗi khách
//!    con một dòng: đã áp chưa, kết quả). Bảng nằm ở database của khách cha.
//! 2. Trong transaction GHI bản ghi của khách cha, module gọi [`Distributor::publish`]: ghi bản phân
//!    phối, một dòng đích cho mỗi khách con được chọn, và một job `<module>.distribute` cho mỗi
//!    đích. Cùng (bản ghi, phiên bản) gửi lại → trả bản cũ, không thêm gì.
//! 3. Module khai bộ chạy bằng [`Distributor::job`] với [`DistributionApply`] của mình. Job (chạy ở
//!    database của khách cha) mở ngữ cảnh khách con qua sổ database, kiểm khách con còn là con
//!    cháu và đã bật module, gọi `apply` (ghi database của con qua service của con), rồi cập nhật
//!    dòng đích.
//!
//! Hai database không có transaction chung: `apply` PHẢI idempotent (tạo bằng
//! [`reference`] — D-184; chỉ cập nhật khi phiên bản mới lớn hơn phiên bản đang lưu ở con). Chết
//! sau khi con đã ghi mà trước khi cha cập nhật dòng đích thì lần chạy lại gặp bản đã có ở con →
//! [`Applied::Unchanged`] → đích thành `applied`.

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

/// Một bản ghi của khách cha cần áp vào khách con.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DistributionItem {
    /// Khách cha gửi.
    pub parent: TenantId,
    /// Loại bản ghi của module, vd `catalog.category`.
    pub record_kind: String,
    /// ID bản ghi ở khách cha.
    pub record_id: Uuid,
    /// Phiên bản của bản ghi ở khách cha; tăng mỗi lần đổi.
    pub version: i64,
    /// Nội dung để áp (JSON của module).
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

    /// Mã tham chiếu (D-184) của bản ghi ở khách con: cùng bản ghi của cùng khách cha luôn ra cùng
    /// mã, kể cả qua nhiều phiên bản — `apply` tìm bản đã nhận bằng mã này.
    pub fn reference(&self) -> String {
        format!("dist:{}:{}:{}", self.parent, self.record_kind, self.record_id)
    }
}

/// Kết quả áp một bản ghi vào khách con.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    Created(Uuid),
    Updated(Uuid),
    /// Đã có bản bằng hay mới hơn (chạy lại, hay phiên bản đến muộn).
    Unchanged(Uuid),
    /// Không áp, kèm lý do ngắn (vd khách con đã tự đặt giá trị riêng).
    Skipped {
        reason: String,
    },
}

/// Module cài để áp một bản ghi của khách cha vào khách con — gọi service của chính module, trong
/// ngữ cảnh con (event, audit của con).
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait DistributionApply: Send + Sync {
    /// Phải idempotent: tạo bằng `item.reference()`, cập nhật chỉ khi `item.version` lớn hơn phiên
    /// bản nguồn đang lưu ở con (`UPDATE … WHERE source_version < ?`).
    async fn apply(
        &self,
        child: &TenantContext,
        actor: Actor,
        item: &DistributionItem,
    ) -> Result<Applied, Error>;
}

/// Khách con nhận một bản phân phối.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Targets {
    /// Con trực tiếp của khách cha.
    AllChildren,
    /// Mọi khách cấp dưới (con, cháu…).
    Descendants,
    /// Đúng các khách này; khách nào không phải cấp dưới → `tenancy.distribution_target_invalid`.
    Only(Vec<TenantId>),
}

/// Một khách con trong bản phân phối.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct TargetStatus {
    pub tenant_id: TenantId,
    /// `pending`, `applied`, `skipped` hay `failed`.
    pub status: String,
    /// `created`, `updated`, `unchanged` hay `skipped`; `None` khi chưa áp.
    pub outcome: Option<String>,
    pub applied_record_id: Option<Uuid>,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub applied_at: Option<Timestamp>,
}

/// Một bản phân phối và tình trạng ở từng khách con.
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

/// Payload của job `<module>.distribute`.
#[derive(Debug, Serialize, Deserialize)]
struct DistributeJob {
    distribution_id: Uuid,
    target_tenant_id: Uuid,
}

/// Bộ phân phối của một module. Lấy bằng `StartContext::distributor()`; clone rẻ.
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
    /// Bộ phân phối của `module` (trong chương trình, lấy bằng `StartContext::distributor()`).
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

    /// Loại job áp bản phân phối: `<module>.distribute`.
    pub fn job_kind(&self) -> String {
        format!("{}.distribute", self.module)
    }

    /// Ghi bản phân phối `item` của khách cha `parent` tới `targets`, trong `connection` —
    /// transaction GHI của khách cha (cùng transaction với thay đổi bản ghi). Trả ID bản phân
    /// phối; cùng (loại, bản ghi, phiên bản) đã có thì trả ID cũ và không thêm đích hay job nào.
    pub async fn publish<C: ConnectionTrait>(
        &self,
        connection: &C,
        parent: &TenantContext,
        actor: Actor,
        item: DistributionItem,
        targets: Targets,
    ) -> Result<Uuid, Error> {
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

    /// Khách cấp dưới được chọn; khách lạ trong `Only` → `tenancy.distribution_target_invalid`.
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

    /// Bản phân phối `distribution` của khách cha `parent`; `None` khi khách không có bản này.
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

    /// Đưa các đích `failed` của bản phân phối về `pending` và chạy lại; trả số đích chạy lại.
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

    /// Bộ chạy job `<module>.distribute` cho `Module::jobs`: `kind` phải là
    /// [`job_kind`](Self::job_kind) (hằng `&'static str` của module, vd `"catalog.distribute"`).
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

    /// Ghi kết quả của một đích (ở database của khách cha).
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

/// Chuỗi lỗi lưu vào cột: cắt 1000 ký tự.
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

/// `tenancy.distribution_target_invalid` (422): khách đích không phải khách cấp dưới của khách gửi.
pub fn target_invalid(tenant: TenantId) -> Error {
    Error::new(
        ErrorKind::Validation,
        "tenancy.distribution_target_invalid",
        "The tenant `{tenant}` is not a child of the sending tenant.",
    )
    .with_param("tenant", tenant.to_string())
}

/// Migration tạo bảng phân phối `<module>_distributions` và `<module>_distribution_targets` của
/// `module`, như [`outbox_migration`](crate::event::outbox_migration).
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
