//! Migrate placement bằng API (D-334): nhà vận hành chạy migration của các database khách (placement)
//! từ giao diện, không phải vào máy chủ gõ `autoxide migrate up --all`.
//!
//! - `dry_run: true`: trả ngay migration còn chờ của từng placement, không đổi gì.
//! - Ngược lại: một dòng `kernel_migration_runs` (mặt điều khiển) và một job `kernel.migrate_placements`
//!   của database điều khiển; job chạy như phần placement của `migrate up --all` (từng placement theo
//!   tên, `migrating` → `ready`, hỏng thì `behind` và dừng). Chỉ một lượt chạy một lúc.
//! - Database điều khiển (`main`) không migrate qua API: `serve` không chạy khi `main` còn migration
//!   chờ, nên máy chủ đang trả lời API thì `main` đã xong.

use std::sync::Arc;

use axum::extract::{Path, State};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::entity::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use sea_orm_migration::prelude::{DbErr, MigrationName, MigrationTrait, SchemaManager, Table};
use sea_orm_migration::schema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;

use super::DatabaseRegistry;
use crate::access::Principal;
use crate::error::Error;
use crate::event::Actor;
use crate::http::ValidJson;
use crate::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use crate::response::{Created, Data, Problem};

/// Job chạy một lượt migrate (job của database điều khiển).
pub(crate) const MIGRATE_PLACEMENTS: &str = "kernel.migrate_placements";

mod entity {
    //! Bảng `kernel_migration_runs` (mặt điều khiển).
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_migration_runs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Tên placement; `None`: mọi placement.
        pub placement: Option<String>,
        /// `pending`, `running`, `done`, `failed`.
        pub status: String,
        pub requested_by: Option<Uuid>,
        /// JSON `[{"placement": …, "applied": ["module/m0001_…", …]}]`.
        pub result: String,
        pub error: Option<String>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

use entity::{Column, Entity, Model};

/// Bảng `kernel_migration_runs`.
pub(crate) struct CreateMigrationRuns;

impl MigrationName for CreateMigrationRuns {
    fn name(&self) -> &str {
        "m0017_create_migration_runs"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateMigrationRuns {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("kernel_migration_runs")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len_null("placement", 64))
                    .col(schema::string_len("status", 16))
                    .col(schema::uuid_null("requested_by"))
                    .col(schema::text("result"))
                    .col(schema::text_null("error"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("kernel_migration_runs").to_owned()).await
    }
}

/// Migrations of one placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PlacementMigrationList {
    #[schema(example = "eu-1")]
    pub placement: String,
    /// `<module>/<migration>`: pending ones for a dry run, applied ones for a run.
    #[schema(example = json!(["notes/m0002_add_tags"]))]
    pub migrations: Vec<String>,
}

fn lists(found: Vec<super::placements::PlacementMigrations>) -> Vec<PlacementMigrationList> {
    found
        .into_iter()
        .map(|entry| PlacementMigrationList {
            placement: entry.name,
            migrations: entry
                .migrations
                .into_iter()
                .map(|state| format!("{}/{}", state.module, state.name))
                .collect(),
        })
        .collect()
}

/// A run of the migrations of the tenant databases (placements).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MigrationRun {
    pub id: Uuid,
    /// The placement it runs on; `null`: every placement, by name.
    pub placement: Option<String>,
    /// `pending`, `running`, `done` or `failed`.
    #[schema(example = "done")]
    pub status: String,
    pub requested_by: Option<Uuid>,
    /// Migrations applied, per placement done (also when a later placement failed).
    pub applied: Vec<PlacementMigrationList>,
    /// Why it failed: the placement and its error. Fix it and start a new run; what is already
    /// applied is skipped.
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Model> for MigrationRun {
    fn from(row: Model) -> Self {
        MigrationRun {
            id: row.id,
            placement: row.placement,
            status: row.status,
            requested_by: row.requested_by,
            applied: serde_json::from_str(&row.result).unwrap_or_default(),
            error: row.error,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct MigratePlacements {
    run_id: Uuid,
}

impl Job for MigratePlacements {
    const KIND: &'static str = MIGRATE_PLACEMENTS;
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Migrate placement bằng API.
pub(crate) struct MigrationRuns {
    registry: DatabaseRegistry,
    setup: super::moves::MoveSetup,
    jobs: JobQueue,
}

impl MigrationRuns {
    pub(crate) fn new(
        registry: DatabaseRegistry,
        setup: super::moves::MoveSetup,
        jobs: JobQueue,
    ) -> Self {
        MigrationRuns { registry, setup, jobs }
    }

    fn control(&self) -> &crate::database::Database {
        self.registry.control()
    }

    /// Migration còn chờ (chạy thử).
    async fn pending(&self, placement: Option<&str>) -> Result<Vec<PlacementMigrationList>, Error> {
        let placements = self.setup.placements()?;
        Ok(lists(placements.pending(placement).await?))
    }

    /// Xếp một lượt chạy. Đang có lượt chưa xong → `kernel.migration_run_busy` (409).
    async fn start(&self, actor: Actor, placement: Option<String>) -> Result<MigrationRun, Error> {
        // Placement phải có (báo ngay thay vì để job hỏng).
        self.setup.placements()?.pending(placement.as_deref()).await?;
        let transaction = self.control().begin_write().await?;
        let busy = Entity::find()
            .filter(Column::Status.is_in(["pending", "running"]))
            .one(&transaction)
            .await?;
        if let Some(busy) = busy {
            return Err(Error::conflict(
                "kernel.migration_run_busy",
                "A migration run is already in progress (`{run}`); wait for it to finish.",
            )
            .with_param("run", busy.id));
        }
        let at = now();
        let row = entity::ActiveModel {
            id: Set(Uuid::now_v7()),
            placement: Set(placement),
            status: Set("pending".to_owned()),
            requested_by: Set(actor.user_id),
            result: Set("[]".to_owned()),
            error: Set(None),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        // Một lần: migration hỏng thì sửa rồi chạy lượt mới, không tự thử lại.
        let options = EnqueueOptions { max_attempts: Some(1), ..EnqueueOptions::default() };
        self.jobs
            .enqueue_database(&transaction, &MigratePlacements { run_id: row.id }, options)
            .await?;
        transaction.commit().await?;
        tracing::info!(run_id = %row.id, placement = ?row.placement, "placement migration run requested");
        Ok(row.into())
    }

    async fn get(&self, id: Uuid) -> Result<MigrationRun, Error> {
        Entity::find_by_id(id).one(self.control().read()).await?.map(Into::into).ok_or_else(|| {
            Error::not_found("kernel.migration_run_not_found", "There is no migration run `{run}`.")
                .with_param("run", id)
        })
    }

    async fn list(&self) -> Result<Vec<MigrationRun>, Error> {
        let rows =
            Entity::find().order_by_desc(Column::Id).limit(50).all(self.control().read()).await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Chạy lượt `id` (bộ chạy job).
    async fn run(&self, id: Uuid) -> Result<(), Error> {
        let Some(row) = Entity::find_by_id(id).one(self.control().write()).await? else {
            return Ok(());
        };
        if row.status != "pending" {
            return Ok(());
        }
        self.set(id, "running", None, None).await?;
        let placements = self.setup.placements()?;
        let mut applied = Vec::new();
        let mut failure = None;
        // Từng placement một, để lượt hỏng giữa chừng vẫn ghi phần đã xong.
        let names: Vec<String> = match &row.placement {
            Some(name) => vec![name.clone()],
            None => placements.pending(None).await?.into_iter().map(|entry| entry.name).collect(),
        };
        for name in names {
            match placements.up_placements(Some(&name)).await {
                Ok(done) => applied.extend(lists(done)),
                Err(error) => {
                    failure = Some(error.message());
                    break;
                }
            }
        }
        let result = serde_json::to_string(&applied).map_err(Error::internal)?;
        let status = if failure.is_some() { "failed" } else { "done" };
        tracing::info!(run_id = %id, status, "placement migration run finished");
        self.set(id, status, Some(result), failure).await
    }

    async fn set(
        &self,
        id: Uuid,
        status: &str,
        result: Option<String>,
        error: Option<String>,
    ) -> Result<(), Error> {
        let mut active = entity::ActiveModel {
            id: Set(id),
            status: Set(status.to_owned()),
            updated_at: Set(now()),
            ..Default::default()
        };
        if let Some(result) = result {
            active.result = Set(result);
        }
        if error.is_some() {
            active.error = Set(error);
        }
        active.update(self.control().write()).await?;
        Ok(())
    }
}

/// Bộ chạy job `kernel.migrate_placements`.
pub(crate) struct MigrateRunner(pub(crate) Arc<MigrationRuns>);

#[async_trait::async_trait]
impl JobHandler for MigrateRunner {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let payload: MigratePlacements =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.run(payload.run_id).await
    }
}

// ---- REST ----------------------------------------------------------------------------------------

/// Nhân sự nền tảng có `kernel.tenants.operate`.
fn operator(principal: &Principal) -> Result<(), Error> {
    principal.require(&crate::builtin::TENANTS_OPERATE)?;
    super::require_platform(principal)
}

/// What to migrate.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MigrationRunRequest {
    /// Name of one placement; omitted: every placement (not `main`, which the server migrates
    /// before it starts).
    #[schema(example = "eu-1")]
    placement: Option<String>,
    /// `true`: only list the migrations that would run, change nothing. Default `false`.
    #[serde(default)]
    dry_run: bool,
}

/// The pending migrations a dry run found.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct MigrationDryRun {
    /// Always `true`.
    dry_run: bool,
    /// Per placement, the migrations that would run (empty: up to date).
    pending: Vec<PlacementMigrationList>,
}

pub(crate) fn routes(runs: Arc<MigrationRuns>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(start_migration_run, list_migration_runs))
        .routes(routes!(get_migration_run))
        .with_state(runs)
}

/// Migrate the tenant databases (placements), or see what would run.
///
/// Platform staff with `kernel.tenants.operate`. With `dry_run: true` the answer (200) lists the
/// pending migrations of each placement and nothing changes. Otherwise a run starts in the
/// background (201, `Location` is its status): placements by name, each `migrating` then `ready`;
/// the first failure marks that placement `behind` and stops the run — fix it and start again,
/// what is already applied is skipped. One run at a time.
#[utoipa::path(
    post,
    path = "/placements/migrations",
    extensions(("x-permissions" = json!(["kernel.tenants.operate"]))),
    operation_id = "start_placement_migrations",
    security(("bearer" = [])),
    request_body = MigrationRunRequest,
    responses(
        (status = 200, description = "Dry run: the pending migrations of each placement.", body = Data<MigrationDryRun>),
        (status = 201, description = "The run, `pending`; `Location` is its status.", body = Data<MigrationRun>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `kernel.tenants.operate` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such placement; code `tenant_placement_not_found`.", body = Problem),
        (status = 409, description = "A run is already in progress; code `kernel.migration_run_busy`.", body = Problem),
    )
)]
async fn start_migration_run(
    State(runs): State<Arc<MigrationRuns>>,
    principal: Principal,
    ValidJson(input): ValidJson<MigrationRunRequest>,
) -> Result<axum::response::Response, Error> {
    use axum::response::IntoResponse;
    operator(&principal)?;
    let placement = input.placement.map(|name| name.trim().to_owned()).filter(|n| !n.is_empty());
    if input.dry_run {
        let pending = runs.pending(placement.as_deref()).await?;
        let data = MigrationDryRun { dry_run: true, pending };
        return Ok(Data { data }.into_response());
    }
    let run = runs.start(Actor::from(&principal), placement).await?;
    Ok(Created::new(format!("/kernel/placements/migrations/{}", run.id), run).into_response())
}

/// List the recent migration runs.
///
/// Platform staff with `kernel.tenants.operate`. Newest first, at most 50.
#[utoipa::path(
    get,
    path = "/placements/migrations",
    extensions(("x-permissions" = json!(["kernel.tenants.operate"]))),
    operation_id = "list_placement_migrations",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The runs, newest first.", body = Data<Vec<MigrationRun>>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `kernel.tenants.operate` (`permission_denied`).", body = Problem),
    )
)]
async fn list_migration_runs(
    State(runs): State<Arc<MigrationRuns>>,
    principal: Principal,
) -> Result<Data<Vec<MigrationRun>>, Error> {
    operator(&principal)?;
    Ok(Data { data: runs.list().await? })
}

/// Get a migration run.
///
/// Platform staff with `kernel.tenants.operate`. Poll it until `done` or `failed`.
#[utoipa::path(
    get,
    path = "/placements/migrations/{id}",
    extensions(("x-permissions" = json!(["kernel.tenants.operate"]))),
    operation_id = "get_placement_migration",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the run.")),
    responses(
        (status = 200, description = "The run.", body = Data<MigrationRun>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `kernel.tenants.operate` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such run; code `kernel.migration_run_not_found`.", body = Problem),
    )
)]
async fn get_migration_run(
    State(runs): State<Arc<MigrationRuns>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<MigrationRun>, Error> {
    operator(&principal)?;
    Ok(Data { data: runs.get(id).await? })
}

// ---- MCP -----------------------------------------------------------------------------------------

/// Which run to read.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MigrationRunRef {
    /// Id of the run.
    id: Uuid,
}

/// Kết quả của tool `kernel.migrate_placements`: chạy thử hay lượt vừa xếp.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ToolAnswer {
    DryRun(MigrationDryRun),
    Run(MigrationRun),
}

pub(crate) fn tools(runs: Arc<MigrationRuns>) -> Vec<crate::mcp::Tool> {
    use crate::mcp::Tool;
    let (start, get) = (runs.clone(), runs);
    vec![
        Tool::new(
            "migrate_placements",
            "Migrate the tenant databases (placements), or with `dry_run` only list the pending \
             migrations. A run goes on in the background; follow it with \
             `kernel.get_migration_run` until `done` or `failed`. Platform staff only.",
            move |_tenant: super::TenantContext,
                  principal: Principal,
                  input: MigrationRunRequest| {
                let runs = start.clone();
                async move {
                    operator(&principal)?;
                    let placement = input
                        .placement
                        .map(|name| name.trim().to_owned())
                        .filter(|n| !n.is_empty());
                    if input.dry_run {
                        let pending = runs.pending(placement.as_deref()).await?;
                        return Ok::<_, Error>(ToolAnswer::DryRun(MigrationDryRun {
                            dry_run: true,
                            pending,
                        }));
                    }
                    Ok(ToolAnswer::Run(runs.start(Actor::from(&principal), placement).await?))
                }
            },
        )
        .requires(crate::builtin::TENANTS_OPERATE),
        Tool::new(
            "get_migration_run",
            "Get a placement migration run: its `status` (`pending`, `running`, `done`, `failed`), \
             the migrations applied per placement and the error.",
            move |_tenant: super::TenantContext, principal: Principal, input: MigrationRunRef| {
                let runs = get.clone();
                async move {
                    operator(&principal)?;
                    runs.get(input.id).await
                }
            },
        )
        .requires(crate::builtin::TENANTS_OPERATE)
        .read_only(),
    ]
}
