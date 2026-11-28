//! The tenant split / merge service ([`TenantRelocations`]): plan, start, run step by step, tenant
//! requests. The steps (`planned` → … → `done`) are in `steps.rs`.

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

/// A party holds a run for at most this long without extending; while running it extends every third of that.
pub(super) const LEASE: Duration = Duration::from_secs(120);

pub(super) fn now() -> Timestamp {
    Timestamp::now()
}

/// What to split or merge (CLI, REST).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelocationInput {
    pub kind: RelocationKind,
    /// The tenant whose data goes: split, the tenant that has the branch; merge, the tenant being merged (ID
    /// or slug).
    pub tenant: String,
    /// Split: the root node of the branch.
    pub org_id: Option<Uuid>,
    /// Merge: the receiving tenant (ID or slug).
    pub into: Option<String>,
    /// Merge: the node of the receiving tenant under which the merged tenant's root becomes a child.
    pub under_org: Option<Uuid>,
    /// Split: slug and name of the new tenant.
    pub slug: Option<String>,
    pub name: Option<String>,
    /// Split: placement (database) of the new tenant.
    pub placement: Option<String>,
    /// Split: the new tenant is a child of the source tenant.
    pub keep_parent: bool,
    /// Why the data moves between the two tenants (required).
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

/// Splits / merges tenants. Published by the `kernel` module (`context.require::<TenantRelocations>()`).
pub struct TenantRelocations {
    pub(super) registry: DatabaseRegistry,
    pub(super) setup: MoveSetup,
    pub(super) outbox: Outbox,
    pub(super) jobs: JobQueue,
    /// Waits this long (in milliseconds) after the tenant becomes `moving` (tenant cache TTL + the maximum
    /// time of a request). Atomic so tests can skip the wait ([`TenantRelocations::skip_settle`]).
    pub(super) settle: std::sync::atomic::AtomicU64,
    /// `AUTOXIDE_RELOCATION_DRAIN_SECONDS`.
    pub(super) drain: Duration,
    pub(super) translations: Translations,
    pub(super) key: SecretKey,
    pub(super) seeds: Seeds,
    /// Settings of the tenants (cache cleared once a run ends: the run moved their setting rows).
    pub(super) settings: Option<Arc<crate::tenancy::settings::SettingsService>>,
}

impl TenantRelocations {
    /// The wait after the tenant becomes `moving`.
    pub(super) fn settle(&self) -> Duration {
        Duration::from_millis(self.settle.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Skips the wait for other servers to see the tenant being moved (tests only, through
    /// `TestApp::skip_relocation_wait`): tests do not have to lower `AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS`
    /// to 1 second, a level that cuts a test's ordinary requests when the CI machine is slow.
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

    /// Tenant-level tables (except job queues) for which no module declares a split / merge policy, read at
    /// `database`; the binary's tests keep this list empty.
    pub async fn uncovered_tables(&self, database: &Database) -> Result<Vec<String>, Error> {
        let (modules, control_plane) = self.setup.tenant_tables()?;
        let tables =
            crate::transfer::TenantTables::read(database, database, &modules, &control_plane)
                .await
                .map_err(|error| Error::internal(error.to_string()))?;
        Ok(self.setup.policies()?.uncovered(&tables))
    }

    /// The plan from the check result.
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

    /// The plan for `input`, changing nothing: blocking reasons (sentences in `language`), conflicts, row
    /// counts.
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

    /// Starts run `input` (or carries out the tenant's request `request` with the values of `input`):
    /// checks, plans; the first blocking reason becomes an error; if there are conflicts, saves the run as
    /// `failed` with a report and returns `tenancy.relocation_conflict`. Nothing is copied yet: call
    /// [`run`](Self::run) (CLI); with `enqueue`, the job `kernel.relocate` runs it (REST).
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

    /// Runs run `id` to completion (`done` / `failed`). `runner` tells apart the running party (CLI, job).
    /// On an error: the run stops at its current state and the error is recorded in `error`; calling again
    /// continues it.
    pub async fn run(&self, id: Uuid, runner: &str) -> Result<TenantRelocation, Error> {
        loop {
            let relocation = self.step(id, runner).await?;
            if !store::UNFINISHED.contains(&relocation.state.as_str()) {
                // Lượt đã chuyển dòng cài đặt (cấp khách và theo cây) giữa hai khách: bỏ cache của cả hai trên máy chủ
                // này (D-20261128-settings-one-system); máy chủ khác thấy sau thời gian sống của cache.
                if let Some(settings) = &self.settings {
                    let tenants = std::iter::once(relocation.source_tenant_id)
                        .chain(relocation.target_tenant_id);
                    for tenant in tenants {
                        settings.forget(crate::tenancy::TenantId::from_uuid(tenant));
                    }
                }
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

    /// Run `id`.
    pub async fn get(&self, id: Uuid) -> Result<TenantRelocation, Error> {
        let row = self.find(id).await?;
        let placement = match row.placement_id {
            Some(placement) => self.placement_row(placement).await.ok().map(|row| row.name),
            None => None,
        };
        store::describe(row, placement)
    }

    /// Tenant `tenant` submits a split / merge request `input` (the source tenant is itself): saves a run
    /// `requested` with a trial plan, into the platform staff's work inbox. Changes no data.
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

    /// Request / run `id` of tenant `tenant` (the source tenant); one of another tenant → not found.
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

    /// Requests waiting for platform staff, oldest first.
    pub(crate) async fn requested(&self, limit: u64) -> Result<Vec<Model>, Error> {
        use sea_orm::{QueryOrder, QuerySelect};
        Ok(Entity::find()
            .filter(Column::State.eq(store::REQUESTED))
            .order_by_asc(Column::CreatedAt)
            .limit(limit)
            .all(self.control().read())
            .await?)
    }

    /// Records a new state, only while this party still holds the lease and the state has not changed.
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

    /// Takes the lease of run `id`. `None`: the run has ended. Another party holds it: `in_progress`.
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

    /// Does one step of run `id` then returns the run. An error is recorded in the run's `error`.
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

    /// Runs `work`, extending the lease of run `id` every third of the term until it finishes.
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

    /// Parameters of the saved run.
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

/// Job that runs run `relocation_id` to completion.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Relocate {
    pub(super) relocation_id: Uuid,
}

impl Job for Relocate {
    const KIND: &'static str = super::RELOCATE;
}

/// Runner of the job `kernel.relocate` (a control-database job).
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

/// The split / merge part of the `kernel` module: configuration, service, routes, jobs.
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
    /// Reads `AUTOXIDE_RELOCATION_DRAIN_SECONDS`.
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<u64, Invalid> {
        reader
            .get_with(&super::DRAIN_SECONDS, |raw| crate::config::parse_in_range(raw, 1u64, 86_400))
    }

    pub(crate) fn set(&mut self, drain: u64) {
        self.drain = drain;
    }

    /// Builds the service, publishes it and the work source "split / merge requests".
    pub(crate) fn start(
        &mut self,
        context: &mut StartContext<'_>,
        setup: MoveSetup,
        key: SecretKey,
        seeds: Seeds,
        request_timeout: Duration,
        settings: Option<Arc<crate::tenancy::settings::SettingsService>>,
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
            settings,
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
