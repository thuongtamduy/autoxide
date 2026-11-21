//! Service tách / gộp khách ([`TenantRelocations`]): kế hoạch, bắt đầu, chạy từng bước, yêu cầu
//! của khách. Các bước (`planned` → … → `done`) ở `steps.rs`.

use crate::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::sea_query::{Condition, Expr};
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use super::prepare::{Prepared, Spec};
use super::store::{self, Column, Entity, Model, Options};
use super::{RelocationBlocker, RelocationKind, RelocationPlan, TenantRelocation, errors};
use crate::config::{ConfigReader, Invalid};
use crate::database::Database;
use crate::error::Error;
use crate::event::{Actor, Outbox};
use crate::i18n::{Language, Translations};
use crate::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue, JobRegistration};
use crate::module::StartContext;
use crate::secret::SecretKey;
use crate::tenancy::admin::Seeds;
use crate::tenancy::moves::MoveSetup;
use crate::tenancy::{DatabaseRegistry, TenantContext};

/// Một bên giữ lượt tối đa chừng này mà không gia hạn; đang chạy thì gia hạn mỗi một phần ba.
pub(super) const LEASE: Duration = Duration::from_secs(120);

pub(super) fn now() -> Timestamp {
    Timestamp::now()
}

/// Tách hay gộp những gì (CLI, REST).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelocationInput {
    pub kind: RelocationKind,
    /// Khách có dữ liệu đi: tách — khách có nhánh; gộp — khách bị gộp (ID hay slug).
    pub tenant: String,
    /// Tách: nút gốc của nhánh.
    pub org_id: Option<Uuid>,
    /// Gộp: khách nhận (ID hay slug).
    pub into: Option<String>,
    /// Gộp: nút của khách nhận mà gốc của khách bị gộp thành con.
    pub under_org: Option<Uuid>,
    /// Tách: slug và tên của khách mới.
    pub slug: Option<String>,
    pub name: Option<String>,
    /// Tách: placement (database) của khách mới.
    pub placement: Option<String>,
    /// Tách: khách mới là con của khách nguồn.
    pub keep_parent: bool,
    /// Vì sao dữ liệu chuyển giữa hai khách (bắt buộc).
    pub reason: String,
}

impl RelocationInput {
    pub(super) fn spec(&self) -> Spec {
        Spec {
            kind: self.kind,
            tenant: self.tenant.clone(),
            org: self.org_id,
            into: self.into.clone(),
            under_org: self.under_org,
            options: Options {
                slug: self.slug.clone(),
                name: self.name.clone(),
                keep_parent: self.keep_parent,
                reason: self.reason.clone(),
                placement: self.placement.clone(),
                into: self.into.clone(),
                ..Options::default()
            },
        }
    }
}

/// Tách / gộp khách. Module `kernel` công bố (`context.require::<TenantRelocations>()`).
pub struct TenantRelocations {
    pub(super) registry: DatabaseRegistry,
    pub(super) setup: MoveSetup,
    pub(super) outbox: Outbox,
    pub(super) jobs: JobQueue,
    /// Chờ chừng này (mili giây) sau khi khách thành `moving` (TTL cache khách + thời gian tối đa của
    /// request). Atomic để test bỏ được thời gian chờ ([`TenantRelocations::skip_settle`]).
    pub(super) settle: std::sync::atomic::AtomicU64,
    /// `AUTOXIDE_RELOCATION_DRAIN_SECONDS`.
    pub(super) drain: Duration,
    pub(super) translations: Translations,
    pub(super) key: SecretKey,
    pub(super) seeds: Seeds,
}

impl TenantRelocations {
    /// Thời gian chờ sau khi khách thành `moving`.
    pub(super) fn settle(&self) -> Duration {
        Duration::from_millis(self.settle.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Bỏ thời gian chờ máy chủ khác thấy khách đang dời (chỉ cho test, qua
    /// `TestApp::skip_relocation_wait`): test không phải hạ `AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS`
    /// xuống 1 giây — mức đó cắt request bình thường của test khi máy CI chậm.
    pub fn skip_settle(&self) {
        self.settle.store(0, std::sync::atomic::Ordering::Relaxed);
    }
}

impl std::fmt::Debug for TenantRelocations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TenantRelocations").finish_non_exhaustive()
    }
}

impl TenantRelocations {
    pub(super) fn control(&self) -> &Database {
        self.registry.control()
    }

    /// Bảng cấp khách (trừ hàng đợi) mà không module nào khai chính sách tách / gộp, đọc ở
    /// `database` — test của binary giữ danh sách này rỗng.
    pub async fn uncovered_tables(&self, database: &Database) -> Result<Vec<String>, Error> {
        let (modules, control_plane) = self.setup.tables()?;
        let tables =
            crate::transfer::TenantTables::read(database, database, &modules, &control_plane)
                .await
                .map_err(|error| Error::internal(error.to_string()))?;
        Ok(self.setup.policies()?.uncovered(&tables))
    }

    /// Kế hoạch từ kết quả kiểm.
    async fn plan_of(
        &self,
        prepared: &Prepared,
        language: Language,
    ) -> Result<RelocationPlan, Error> {
        let report = self.report(prepared).await?;
        let blocking: Vec<RelocationBlocker> = prepared
            .blockers
            .iter()
            .map(|error| RelocationBlocker {
                code: error.code().to_owned(),
                message: self.translations.message(error, language),
            })
            .collect();
        let (tables, conflicts, warnings) = match report {
            Some(report) => (report.tables, report.conflicts, report.warnings),
            None => (Vec::new(), Vec::new(), Vec::new()),
        };
        Ok(RelocationPlan {
            kind: prepared.kind,
            source_tenant_id: prepared.source.id,
            source: prepared.source.slug.clone(),
            target_tenant_id: prepared.target_id,
            target: prepared.target_slug.clone(),
            org_id: prepared.org.as_uuid(),
            placement: prepared.placement.name.clone(),
            ready: blocking.is_empty() && conflicts.is_empty(),
            blocking,
            conflicts,
            warnings,
            rows: tables
                .iter()
                .filter(|table| table.action == "move" || table.action == "copy")
                .map(|table| table.rows)
                .sum(),
            tables,
        })
    }

    /// Kế hoạch của `input`, không đổi gì: lý do chặn (câu theo `language`), xung đột, số dòng.
    pub async fn plan(
        &self,
        input: &RelocationInput,
        language: Language,
    ) -> Result<RelocationPlan, Error> {
        let spec = input.spec();
        spec.check(true)?;
        let prepared = self.prepare(&spec, None).await?;
        self.plan_of(&prepared, language).await
    }

    /// Bắt đầu lượt `input` (hay thực hiện yêu cầu `request` của khách với các giá trị của
    /// `input`): kiểm, lập kế hoạch; lý do chặn đầu tiên thành lỗi; có xung đột thì lưu lượt
    /// `failed` kèm báo cáo và trả `tenancy.relocation_conflict`. Chưa chép gì: gọi
    /// [`run`](Self::run) (CLI) — `enqueue` thì job `kernel.relocate` chạy (REST).
    pub async fn start(
        &self,
        actor: Actor,
        input: &RelocationInput,
        request: Option<Uuid>,
        enqueue: bool,
    ) -> Result<TenantRelocation, Error> {
        let spec = input.spec();
        spec.check(true)?;
        let mut prepared = self.prepare(&spec, None).await?;
        if !prepared.blockers.is_empty() {
            return Err(prepared.blockers.swap_remove(0));
        }
        let plan = self.plan_of(&prepared, Language::En).await?;
        let at = now();
        let id = match request {
            Some(id) => {
                let row = self.find(id).await?;
                if row.state != store::REQUESTED {
                    return Err(errors::in_progress(&id.to_string()));
                }
                id
            }
            None => store::new_id(),
        };
        let state = if plan.conflicts.is_empty() { store::PLANNED } else { store::FAILED };
        let model = store::entity::ActiveModel {
            id: Set(id),
            kind: Set(spec.kind.as_str().to_owned()),
            state: Set(state.to_owned()),
            source_tenant_id: Set(prepared.source.id),
            target_tenant_id: Set(Some(prepared.target_id)),
            org_id: Set(Some(prepared.org.as_uuid())),
            placement_id: Set(Some(prepared.placement.id)),
            options: Set(spec.options.json()?),
            plan: Set(Some(serde_json::to_string(&plan).map_err(Error::internal)?)),
            tables: Set("[]".to_owned()),
            error: Set(None),
            runner: Set(None),
            runner_until: Set(None),
            requested_by: sea_orm::ActiveValue::NotSet,
            created_by: Set(actor.user_id),
            created_at: Set(at),
            updated_at: Set(at),
            finished_at: Set((state == store::FAILED).then_some(at)),
        };
        let transaction = self.control().begin_write().await?;
        if request.is_some() {
            model.update(&transaction).await?;
        } else {
            let mut model = model;
            model.requested_by = Set(None);
            model.insert(&transaction).await?;
        }
        if state == store::PLANNED && enqueue {
            self.jobs
                .enqueue_database(
                    &transaction,
                    &Relocate { relocation_id: id },
                    EnqueueOptions::default(),
                )
                .await?;
        }
        transaction.commit().await?;
        if state == store::FAILED {
            return Err(errors::conflict(plan.conflicts.len(), &id.to_string()));
        }
        tracing::info!(relocation_id = %id, kind = spec.kind.as_str(), source = %prepared.source.slug, "tenant relocation planned");
        self.get(id).await
    }

    /// Chạy lượt `id` tới xong (`done` / `failed`). `runner` phân biệt bên chạy (CLI, job). Lỗi:
    /// lượt dừng ở trạng thái đang có, lỗi ghi vào `error`; gọi lại thì chạy tiếp.
    pub async fn run(&self, id: Uuid, runner: &str) -> Result<TenantRelocation, Error> {
        loop {
            let relocation = self.step(id, runner).await?;
            if !store::UNFINISHED.contains(&relocation.state.as_str()) {
                return Ok(relocation);
            }
        }
    }

    pub(super) async fn find(&self, id: Uuid) -> Result<Model, Error> {
        Entity::find_by_id(id)
            .one(self.control().write())
            .await?
            .ok_or_else(|| errors::not_found(&id.to_string()))
    }

    /// Lượt `id`.
    pub async fn get(&self, id: Uuid) -> Result<TenantRelocation, Error> {
        let row = self.find(id).await?;
        let placement = match row.placement_id {
            Some(placement) => self.placement_row(placement).await.ok().map(|row| row.name),
            None => None,
        };
        store::describe(row, placement)
    }

    /// Khách `tenant` gửi yêu cầu tách / gộp `input` (khách nguồn là chính nó): lưu lượt
    /// `requested` kèm kế hoạch thử, vào hộp việc của nhân sự nền tảng. Không đổi dữ liệu nào.
    pub async fn request(
        &self,
        actor: Actor,
        tenant: &TenantContext,
        mut input: RelocationInput,
    ) -> Result<TenantRelocation, Error> {
        input.tenant = tenant.id().to_string();
        let spec = input.spec();
        spec.check(false)?;
        let at = now();
        let id = store::new_id();
        let model = store::entity::ActiveModel {
            id: Set(id),
            kind: Set(spec.kind.as_str().to_owned()),
            state: Set(store::REQUESTED.to_owned()),
            source_tenant_id: Set(tenant.id().as_uuid()),
            target_tenant_id: Set(None),
            org_id: Set(spec.org.or(spec.under_org)),
            placement_id: Set(None),
            options: Set(spec.options.json()?),
            plan: Set(None),
            tables: Set("[]".to_owned()),
            error: Set(None),
            runner: Set(None),
            runner_until: Set(None),
            requested_by: Set(actor.user_id),
            created_by: Set(None),
            created_at: Set(at),
            updated_at: Set(at),
            finished_at: Set(None),
        };
        model.insert(self.control().write()).await?;
        tracing::info!(relocation_id = %id, tenant_id = %tenant.id(), "tenant relocation requested");
        self.get(id).await
    }

    /// Yêu cầu / lượt `id` của khách `tenant` (khách nguồn); của khách khác → không tìm thấy.
    pub async fn get_own(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<TenantRelocation, Error> {
        let row = self.find(id).await?;
        if row.source_tenant_id != tenant.id().as_uuid() {
            return Err(errors::not_found(&id.to_string()));
        }
        self.get(id).await
    }

    /// Yêu cầu đang chờ nhân sự nền tảng, cũ nhất trước.
    pub(crate) async fn requested(&self, limit: u64) -> Result<Vec<Model>, Error> {
        use sea_orm::{QueryOrder, QuerySelect};
        Ok(Entity::find()
            .filter(Column::State.eq(store::REQUESTED))
            .order_by_asc(Column::CreatedAt)
            .limit(limit)
            .all(self.control().read())
            .await?)
    }

    /// Ghi trạng thái mới — chỉ khi bên này còn giữ lease và trạng thái chưa đổi.
    pub(super) async fn set_state<C: ConnectionTrait>(
        &self,
        connection: &C,
        row: &Model,
        runner: &str,
        state: &str,
        update: impl FnOnce(sea_orm::UpdateMany<Entity>) -> Result<sea_orm::UpdateMany<Entity>, Error>,
    ) -> Result<(), Error> {
        let at = now();
        let mut query = Entity::update_many()
            .col_expr(Column::State, Expr::value(state))
            .col_expr(Column::Error, Expr::value(Option::<String>::None))
            .col_expr(Column::UpdatedAt, Expr::value(at))
            .filter(Column::Id.eq(row.id))
            .filter(Column::State.eq(row.state.as_str()))
            .filter(Column::Runner.eq(runner));
        if state == store::DONE || state == store::FAILED {
            query = query
                .col_expr(Column::FinishedAt, Expr::value(at))
                .col_expr(Column::RunnerUntil, Expr::value(Option::<Timestamp>::None));
        }
        query = update(query)?;
        if query.exec(connection).await?.rows_affected != 1 {
            return Err(errors::in_progress(&row.id.to_string()));
        }
        Ok(())
    }

    /// Nhận lease của lượt `id`. `None`: lượt đã kết thúc. Bên khác đang giữ: `in_progress`.
    async fn acquire(&self, id: Uuid, runner: &str) -> Result<Option<Model>, Error> {
        let at = now();
        let until = at + SignedDuration::try_from(LEASE).map_err(Error::internal)?;
        let taken = Entity::update_many()
            .col_expr(Column::Runner, Expr::value(runner))
            .col_expr(Column::RunnerUntil, Expr::value(until))
            .filter(Column::Id.eq(id))
            .filter(Column::State.is_in(store::UNFINISHED))
            .filter(
                Condition::any()
                    .add(Column::RunnerUntil.is_null())
                    .add(Column::RunnerUntil.lt(at))
                    .add(Column::Runner.eq(runner)),
            )
            .exec(self.control().write())
            .await?;
        let row = self.find(id).await?;
        if taken.rows_affected == 1 {
            return Ok(Some(row));
        }
        if store::UNFINISHED.contains(&row.state.as_str()) {
            return Err(errors::in_progress(&id.to_string()));
        }
        Ok(None)
    }

    async fn release(&self, id: Uuid, runner: &str) -> Result<(), Error> {
        Entity::update_many()
            .col_expr(Column::RunnerUntil, Expr::value(Option::<Timestamp>::None))
            .filter(Column::Id.eq(id))
            .filter(Column::Runner.eq(runner))
            .exec(self.control().write())
            .await?;
        Ok(())
    }

    /// Làm một bước của lượt `id` rồi trả lượt. Lỗi được ghi vào `error` của lượt.
    pub async fn step(&self, id: Uuid, runner: &str) -> Result<TenantRelocation, Error> {
        let Some(row) = self.acquire(id, runner).await? else {
            return self.get(id).await;
        };
        let advanced = self.holding(id, runner, self.advance(&row, runner)).await;
        let released = self.release(id, runner).await;
        match advanced {
            Ok(()) => {
                released?;
                self.get(id).await
            }
            Err(error) => {
                let message = crate::tenancy::moves::describe(&error);
                tracing::warn!(relocation_id = %id, error = %message, "tenant relocation stopped");
                let _ = Entity::update_many()
                    .col_expr(Column::Error, Expr::value(message))
                    .col_expr(Column::UpdatedAt, Expr::value(now()))
                    .filter(Column::Id.eq(id))
                    .exec(self.control().write())
                    .await;
                Err(error)
            }
        }
    }

    /// Chạy `work`, gia hạn lease của lượt `id` mỗi một phần ba thời hạn cho tới khi xong.
    async fn holding<T>(
        &self,
        id: Uuid,
        runner: &str,
        work: impl Future<Output = Result<T, Error>>,
    ) -> Result<T, Error> {
        let control = self.control().clone();
        let runner = runner.to_owned();
        let renew = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(LEASE / 3);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let until = now() + SignedDuration::from_secs(120);
                let renewed = Entity::update_many()
                    .col_expr(Column::RunnerUntil, Expr::value(until))
                    .filter(Column::Id.eq(id))
                    .filter(Column::Runner.eq(runner.as_str()))
                    .exec(control.write())
                    .await;
                if let Err(error) = renewed {
                    tracing::warn!(relocation_id = %id, error = %error, "cannot extend the relocation lease");
                }
            }
        });
        let outcome = work.await;
        renew.abort();
        outcome
    }

    async fn advance(&self, row: &Model, runner: &str) -> Result<(), Error> {
        // `Box::pin`: mỗi bước là một future lớn; cấp phát trên heap giữ kiểu của `step` nhỏ.
        match row.state.as_str() {
            store::PLANNED => Box::pin(self.freeze(row, runner)).await,
            store::FROZEN => Box::pin(self.copy(row, runner)).await,
            store::COPIED => Box::pin(self.verify(row, runner)).await,
            store::VERIFIED => Box::pin(self.switch(row, runner)).await,
            store::SWITCHED => Box::pin(self.clean(row, runner)).await,
            store::CLEANED => {
                self.set_state(self.control().write(), row, runner, store::DONE, Ok).await
            }
            _ => Ok(()),
        }
    }

    /// Tham số của lượt đã lưu.
    pub(super) fn spec_of(row: &Model) -> Result<Spec, Error> {
        let kind = RelocationKind::parse(&row.kind).ok_or_else(|| {
            Error::internal(format!("relocation {} has kind `{}`", row.id, row.kind))
        })?;
        let options = Options::parse(&row.options)?;
        Ok(Spec {
            kind,
            tenant: row.source_tenant_id.to_string(),
            org: (kind == RelocationKind::Split).then_some(row.org_id).flatten(),
            into: row.target_tenant_id.map(|id| id.to_string()).or_else(|| options.into.clone()),
            under_org: (kind == RelocationKind::Merge).then_some(row.org_id).flatten(),
            options,
        })
    }
}

/// Job chạy lượt `relocation_id` tới xong.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Relocate {
    pub(super) relocation_id: Uuid,
}

impl Job for Relocate {
    const KIND: &'static str = super::RELOCATE;
}

/// Bộ chạy job `kernel.relocate` (job của database điều khiển).
pub(crate) struct RelocateRunner(pub(crate) Arc<TenantRelocations>);

#[async_trait::async_trait]
impl JobHandler for RelocateRunner {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let payload: Relocate =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        let done = self.0.run(payload.relocation_id, &format!("job-{}", job.id)).await?;
        tracing::info!(relocation_id = %done.id, state = %done.state, "tenant relocation job finished");
        Ok(())
    }
}

/// Phần tách / gộp trong module `kernel`: cấu hình, service, route, job.
pub(crate) struct Wiring {
    drain: u64,
    service: Option<Arc<TenantRelocations>>,
}

impl Default for Wiring {
    fn default() -> Self {
        Wiring { drain: 300, service: None }
    }
}

impl Wiring {
    /// Đọc `AUTOXIDE_RELOCATION_DRAIN_SECONDS`.
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<u64, Invalid> {
        reader
            .get_with(&super::DRAIN_SECONDS, |raw| crate::config::parse_in_range(raw, 1u64, 86_400))
    }

    pub(crate) fn set(&mut self, drain: u64) {
        self.drain = drain;
    }

    /// Dựng service, công bố nó và nguồn việc "yêu cầu tách / gộp".
    pub(crate) fn start(
        &mut self,
        context: &mut StartContext<'_>,
        setup: MoveSetup,
        key: SecretKey,
        seeds: Seeds,
        request_timeout: Duration,
    ) -> Result<(), Error> {
        let registry = context.tenancy().clone();
        let settle = Duration::from_secs(registry.config().cache_seconds) + request_timeout;
        let service = Arc::new(TenantRelocations {
            registry,
            setup,
            outbox: context.outbox()?,
            jobs: context.jobs(),
            settle: std::sync::atomic::AtomicU64::new(
                u64::try_from(settle.as_millis()).unwrap_or(u64::MAX),
            ),
            drain: Duration::from_secs(self.drain),
            translations: context.translations().clone(),
            key,
            seeds,
        });
        context.work_source(Arc::new(super::RelocationRequestsWork(service.clone())))?;
        context.provide::<TenantRelocations>(service.clone())?;
        self.service = Some(service);
        Ok(())
    }

    pub(crate) fn routes(&self) -> utoipa_axum::router::OpenApiRouter {
        self.service.clone().map_or_else(utoipa_axum::router::OpenApiRouter::new, super::routes)
    }

    pub(crate) fn jobs(&self) -> Vec<JobRegistration> {
        self.service
            .iter()
            .map(|service| {
                JobRegistration::new(super::RELOCATE, Arc::new(RelocateRunner(service.clone())))
            })
            .collect()
    }
}
