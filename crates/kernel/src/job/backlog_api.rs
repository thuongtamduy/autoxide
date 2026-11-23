//! Backlog over REST and MCP (step 39, A3): the same figures as the gauges at `/metrics`, for operators and
//! agents without Prometheus. Only platform tenant staff (`require_platform`), with permission
//! `kernel.ops.read`: the figures cover the whole database, including every tenant on it.

use crate::time::Timestamp;
use std::sync::Arc;

use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::backlog::{BacklogModules, BacklogSnapshot};
use crate::access::{Permission, Principal};
use crate::error::Error;
use crate::mcp::{NoInput, Tool};
use crate::response::{Data, Problem};
use crate::tenancy::DatabaseRegistry;

pub(crate) const OPS_READ: Permission = Permission::new(
    "kernel.ops.read",
    "See queue backlogs, dead jobs, undelivered events, database connections and the newest \
     backup of every database (platform staff only).",
);

/// Backlogs of every database this server handles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub(crate) struct OpsBacklog {
    pub checked_at: Timestamp,
    pub databases: Vec<DatabaseBacklog>,
}

/// Backlog of one database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub(crate) struct DatabaseBacklog {
    /// `main`, or the placement of tenants with their own database.
    #[schema(example = "main")]
    pub database: String,
    /// Modules with a job queue or events, in start order of their names.
    pub modules: Vec<ModuleQueue>,
    /// The `read` and `write` connection pools.
    pub pools: Vec<Pool>,
    /// When the newest successful scheduled backup of this database finished; `null` when none.
    pub last_backup_at: Option<Timestamp>,
}

/// Jobs and events of one module on one database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ModuleQueue {
    #[schema(example = "mail")]
    pub module: String,
    /// Jobs not done yet, scheduled ones included.
    pub jobs_pending: u64,
    /// How long the oldest job that is due has waited, in seconds; 0 when none.
    pub jobs_oldest_pending_seconds: u64,
    /// Jobs that failed for good (the dead-letter queue: `autoxide jobs failed`).
    pub jobs_dead: u64,
    /// Events not delivered yet.
    pub events_pending: u64,
    /// How long the oldest undelivered event has waited, in seconds; 0 when none.
    pub events_oldest_pending_seconds: u64,
    /// Events whose delivery failed for good.
    pub events_dead: u64,
}

/// Connections of one pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub(crate) struct Pool {
    /// `read` or `write`.
    pub pool: String,
    pub in_use: u32,
    pub idle: u32,
    pub max: u32,
}

impl DatabaseBacklog {
    pub(crate) fn from_snapshot(
        snapshot: BacklogSnapshot,
        last_backup_at: Option<Timestamp>,
    ) -> Self {
        DatabaseBacklog {
            database: snapshot.database,
            modules: snapshot
                .modules
                .into_iter()
                .map(|found| ModuleQueue {
                    module: found.module.to_owned(),
                    jobs_pending: found.jobs_pending,
                    jobs_oldest_pending_seconds: found.jobs_oldest_pending_seconds,
                    jobs_dead: found.jobs_dead,
                    events_pending: found.events_pending,
                    events_oldest_pending_seconds: found.events_oldest_pending_seconds,
                    events_dead: found.events_dead,
                })
                .collect(),
            pools: snapshot
                .pools
                .into_iter()
                .map(|pool| Pool {
                    pool: pool.pool.to_owned(),
                    in_use: pool.in_use,
                    idle: pool.idle,
                    max: pool.max,
                })
                .collect(),
            last_backup_at,
        }
    }
}

/// What REST and MCP need.
#[derive(Clone)]
pub(crate) struct BacklogState {
    pub registry: DatabaseRegistry,
    pub modules: BacklogModules,
}

/// Checks permission then reads - one place for both REST and MCP (architecture rule 8).
async fn read(state: &BacklogState, principal: &Principal) -> Result<OpsBacklog, Error> {
    principal.require(&OPS_READ)?;
    crate::tenancy::require_platform(principal)?;
    super::backlog::status(&state.registry, &state.modules).await
}

pub(crate) fn routes(state: BacklogState) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(get_backlog)).with_state(Arc::new(state))
}

/// See the job and event backlogs of every database.
///
/// Requires `kernel.ops.read`, as staff of the platform tenant. Counts what waits in the job queue
/// and the event outbox of each module, the dead-letter queue, the connections in use and the
/// newest scheduled backup of every database — the figures behind the alerts in
/// `deploy/prometheus/autoxide-alerts.yml`. Counts cover every tenant on a database.
#[utoipa::path(
    get,
    path = "/ops/backlog",
    extensions(("x-permissions" = json!(["kernel.ops.read"]))),
    operation_id = "get_ops_backlog",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The backlog of every database.", body = Data<OpsBacklog>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.ops.read` (`permission_denied`), or not staff of the platform tenant (`tenant_platform_only`).", body = Problem),
    )
)]
async fn get_backlog(
    State(state): State<Arc<BacklogState>>,
    principal: Principal,
) -> Result<Data<OpsBacklog>, Error> {
    Ok(Data { data: read(&state, &principal).await? })
}

/// The MCP tool `kernel.ops_backlog` - calls exactly the function REST uses.
pub(crate) fn tools(state: BacklogState) -> Vec<Tool> {
    vec![
        Tool::new(
            "ops_backlog",
            "See the job and event backlogs, the dead-letter queue, the database connections and \
             the newest backup of every database (platform staff only). Use it to find out why \
             mail, webhooks or reports are late.",
            move |_tenant: crate::tenancy::TenantContext, principal: Principal, _: NoInput| {
                let state = state.clone();
                async move { read(&state, &principal).await }
            },
        )
        .requires(OPS_READ)
        .read_only(),
    ]
}
