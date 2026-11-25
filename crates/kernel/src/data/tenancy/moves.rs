//! Moving a tenant between placements, `shared` ↔ `dedicated` (spec 33 "Provisioning and moving tenants"):
//! the command `autoxide tenant move` (runs directly on the server) and REST / MCP for platform staff only
//! (dry run by default; a real run is done by a background job).
//!
//! A move is one `kernel_tenant_moves` row (control plane) that goes through these states:
//!
//! 1. Start: in ONE transaction on the control database, the tenant goes `active` → `moving` (every request
//!    to the tenant gets `503 tenant_maintenance`) and the move is recorded as `copying`.
//! 2. `copying` → `copied`: waits for other servers to see `moving` (tenant cache TTL) and for running
//!    requests to finish (`AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS`); waits for the tenant's jobs / events
//!    being run at the source to release their hold (`claimed_until`): workers and the dispatcher take no
//!    new work of a `moving` tenant; both databases must be fully migrated and different; copies every
//!    `tenant_id = X` row of every tenant-level table (including outbox and job queue: undelivered events
//!    and unrun jobs follow the data) in ONE transaction at the target, with the event
//!    `kernel.tenant_moved`; compares the row count of each table.
//! 3. `copied` → `switched`: recompares the row counts, then in one transaction on the control database
//!    changes the tenant's `placement_id` to the target and sets it back to `active`: the tenant is served
//!    from the new database immediately.
//! 4. `switched` → `done`: deletes the tenant's rows at the source (`keep_source`: deletes only the job
//!    queue and outbox so nothing runs twice; the remaining data is a backup copy, deleted later with
//!    [`TenantMoves::purge`]).
//!
//! - **Safe to rerun**: each step holds a lease (`runner`, `runner_until`, extended while running); two
//!   parties never run the same move. If the process dies midway: the tenant stays `moving` (or is already
//!   `active` at the target if past step 3); moving again to the same placement continues from the recorded
//!   state (step 2 deletes the tenant's rows at the target and copies again). Can be aborted before step 3
//!   ([`TenantMoves::abort`]).
//! - There are never two "live" copies: `placement_id` changes only after the row counts match, and the
//!   source is deleted only after the change.
//! - Not copied: control-plane tables (`kernel_tenants`, `kernel_placements`, domains...).
//! - Stored files (keys `tenants/<id>/…`): if two placements share a store (the same shared store, or own
//!   stores that point to the same place) nothing is copied. With different stores: step 2 copies every
//!   file of the tenant to the target's store (deleting the tenant's extra files there first, comparing
//!   file counts afterwards), step 4 deletes the files at the source (except `keep_source`; `purge` deletes
//!   them later). Before copying or deleting, a marker file is written at the target and looked for at the
//!   source: two different configurations that are actually the same place (e.g. an own store pointing at
//!   the very directory of the shared store) count as the same store, so a file in use is never deleted.

mod claims;
mod http;

use crate::time::Timestamp;
use sea_orm::sea_query::{Condition, Expr};
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use utoipa::ToSchema;
use uuid::Uuid;

use super::admin::{placement, tenant};
use super::{
    DEFAULT_TENANT, DatabaseRegistry, MAIN_PLACEMENT, PlacementId, TenantContext, TenantId,
    TenantStatus, errors,
};
use crate::access::{Permission, Risk};
use crate::database::Database;
use crate::error::Error;
use crate::event::{Actor, Event, EventInfo, Outbox};
use crate::i18n::{Language, Translations};
use crate::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use crate::migration::Migrator;
use crate::migration::prelude::*;
use crate::storage::Storage;
use crate::transfer::{CopiedTable, TableKind, TenantTables, TransferError, same_database};

pub(super) use claims::wait_claims;
pub(crate) use http::{routes, tools};

mod migration;

pub(crate) use migration::CreateTenantMoves;
mod files;
mod finish;
mod phases;
mod preflight;
mod steps;

/// Moves a tenant's data to another placement.
pub const TENANTS_MOVE: Permission = Permission::new(
    "kernel.tenants.move",
    "Move a tenant's data to another placement (database); the tenant is unavailable during the \
     move (platform staff only).",
)
.with_risk(Risk::Destructive);

pub(crate) const TENANT_MOVED: EventInfo = EventInfo::new(
    "kernel.tenant_moved",
    "The tenant's data was moved to another placement (database).",
);

/// Kind of job that runs a move started through REST / MCP.
pub(crate) const MOVE_TENANT: &str = "kernel.move_tenant";

const COPYING: &str = "copying";
const COPIED: &str = "copied";
const SWITCHED: &str = "switched";
const DONE: &str = "done";
const ABORTED: &str = "aborted";
const UNFINISHED: [&str; 3] = [COPYING, COPIED, SWITCHED];

/// A party holds a move for at most this long without extending; while running it extends every third of that.
const LEASE: Duration = Duration::from_secs(120);
/// The `kernel` outbox table: gets the extra event `kernel.tenant_moved` at the target.
const KERNEL_EVENTS: &str = "kernel_events";

fn now() -> Timestamp {
    Timestamp::now()
}

mod entity {
    //! The `kernel_tenant_moves` table (control plane).
    use crate::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_tenant_moves")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub from_placement_id: Uuid,
        pub to_placement_id: Uuid,
        pub keep_source: bool,
        pub state: String,
        /// JSON `[{"table": …, "rows": …}]`: the rows copied per table.
        pub tables: String,
        pub error: Option<String>,
        pub runner: Option<String>,
        pub runner_until: Option<Timestamp>,
        /// When the tenant became `moving`: other servers' caches wait from here.
        pub moving_since: Timestamp,
        pub requested_by: Option<Uuid>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
        pub finished_at: Option<Timestamp>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

use entity::{Column, Entity, Model};

/// Rows of the tenant in one table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TenantTableRows {
    #[schema(example = "notes_note")]
    pub table: String,
    pub rows: u64,
}

impl From<CopiedTable> for TenantTableRows {
    fn from(copied: CopiedTable) -> Self {
        TenantTableRows { table: copied.table, rows: copied.rows }
    }
}

/// Why the move cannot run now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TenantMoveBlocker {
    /// Stable code, the same as the error a real run would answer.
    #[schema(example = "tenant_move_schema_differs")]
    pub code: String,
    pub message: String,
}

/// What a move would do (dry run). Nothing is changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TenantMovePlan {
    pub tenant_id: Uuid,
    /// Slug of the tenant.
    #[schema(example = "acme")]
    pub tenant: String,
    /// Placement that keeps the tenant's data now.
    #[schema(example = "main")]
    pub from: String,
    /// Placement the data would move to.
    #[schema(example = "acme-db")]
    pub to: String,
    pub keep_source: bool,
    /// `true` when nothing blocks the move (`blocking` is empty).
    pub ready: bool,
    /// What must be fixed first; empty when the move can run.
    pub blocking: Vec<TenantMoveBlocker>,
    /// An interrupted move to `to` that a real run resumes.
    pub resumes: Option<Uuid>,
    /// Rows of the tenant that would be copied, per table, in copy order.
    pub tables: Vec<TenantTableRows>,
    /// Sum of `tables[].rows`.
    pub rows: u64,
    /// Files of the tenant that would be copied to the file storage of `to`. `0` in two cases:
    /// both placements keep files in the same storage (nothing to copy), or the tenant has no
    /// files.
    pub files: u64,
    /// Seconds the tenant answers `503 tenant_maintenance` before copying starts (other servers
    /// see the move and requests in progress finish); copying takes longer for many rows.
    pub settle_seconds: u64,
}

/// A move of a tenant to another placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TenantMove {
    pub id: Uuid,
    pub tenant_id: Uuid,
    #[schema(example = "main")]
    pub from: String,
    #[schema(example = "acme-db")]
    pub to: String,
    pub keep_source: bool,
    /// `copying` (the tenant answers `503`), `copied`, `switched` (the tenant works from `to`;
    /// the old placement is being cleaned), `done` or `aborted`.
    #[schema(example = "done")]
    pub state: String,
    /// Rows copied per table (after `copied`).
    pub tables: Vec<TenantTableRows>,
    /// Sum of `tables[].rows`.
    pub rows: u64,
    /// Why the last run stopped, when it did; running the move again resumes it.
    pub error: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub finished_at: Option<Timestamp>,
}

#[derive(Debug, Serialize)]
struct TenantMoved {
    tenant_id: Uuid,
    from: String,
    to: String,
    rows: u64,
}

impl Event for TenantMoved {
    const NAME: &'static str = "kernel.tenant_moved";
}

/// Job that runs move `move_id` to completion.
#[derive(Debug, Serialize, Deserialize)]
struct MoveTenant {
    move_id: Uuid,
}

impl Job for MoveTenant {
    const KIND: &'static str = MOVE_TENANT;
}

/// What a move needs from every module: the module name (table owner), control-plane tables, migrations. Filled
/// once after every module has started (`Modules::start_in`). Cheap to clone.
#[derive(Clone, Default)]
pub(crate) struct MoveSetup(Arc<OnceLock<Setup>>);

pub(crate) struct Setup {
    pub(crate) modules: Vec<&'static str>,
    pub(crate) control_plane: Vec<&'static str>,
    pub(crate) migrator: Migrator,
    /// Placement management (migrating a placement through the API, D-334).
    pub(crate) placements: super::Placements,
    /// Tables kept when a tenant is deleted for good (`Module::retained_tables`, D-337).
    pub(crate) retained: Vec<&'static str>,
    /// Split / merge policy of every module (`Module::relocation`, step 38).
    pub(crate) policies: super::relocate::Policies,
    /// Modules present in the binary but turned off (`AUTOXIDE_MODULES`) with their own migrations (dormant
    /// mode, step 70 P9, D-20261125-dormant-modules): deleting, exporting, moving, splitting and merging a
    /// tenant cover their tables too. `control_plane`, `retained` and `policies` already include their
    /// declarations.
    pub(crate) dormant: Vec<(&'static str, Migrator)>,
}

impl MoveSetup {
    pub(crate) fn fill(&self, setup: Setup) {
        let _ = self.0.set(setup);
    }

    /// Before copying a tenant from `source` to `target` (move, split, merge; a real run, never a dry run):
    /// every turned-off module whose tables exist at `source` OR `target` gets its migrations applied at both,
    /// so its tables exist with the same columns on both sides and travel with the tenant (dormant mode,
    /// D-20261125-dormant-modules). A module that never ran on either database is left alone.
    pub(crate) async fn prepare_dormant(
        &self,
        source: &Database,
        target: &Database,
    ) -> Result<(), Error> {
        for (module, migrator) in &self.get()?.dormant {
            if !installed_on(module, source).await? && !installed_on(module, target).await? {
                continue;
            }
            for database in [source, target] {
                migrator.up(database).await.map_err(|error| Error::internal(error.to_string()))?;
            }
        }
        Ok(())
    }

    /// The modules whose tables a DRY RUN reads between `source` and `target` (it changes no schema): the
    /// enabled modules, and the turned-off modules that have tables on both databases. A turned-off module
    /// with tables at `source` only is returned in the second list: its rows are counted at `source`, and a
    /// real run creates its tables at `target` first.
    pub(crate) async fn dry_run_modules(
        &self,
        source: &Database,
        target: &Database,
    ) -> Result<(Vec<&'static str>, Vec<&'static str>), Error> {
        let setup = self.get()?;
        let mut both = setup.modules.clone();
        let mut source_only = Vec::new();
        for (module, _) in &setup.dormant {
            match (installed_on(module, source).await?, installed_on(module, target).await?) {
                (true, true) => both.push(module),
                (true, false) => source_only.push(*module),
                _ => {}
            }
        }
        Ok((both, source_only))
    }

    /// Split / merge policy of every module.
    pub(crate) fn policies(&self) -> Result<super::relocate::Policies, Error> {
        Ok(self.get()?.policies.clone())
    }

    /// Migrations still missing at `database`, across every registered module.
    pub(crate) async fn pending(
        &self,
        database: &Database,
    ) -> Result<usize, crate::migration::MigrationError> {
        match self.get() {
            Ok(setup) => Ok(setup.migrator.pending(database).await?.len()),
            Err(_) => Ok(0),
        }
    }

    /// Tables that every module declares kept when a tenant is deleted for good.
    pub(crate) fn retained(&self) -> Result<Vec<&'static str>, Error> {
        Ok(self.get()?.retained.clone())
    }

    /// Placement management, once every module has started.
    pub(crate) fn placements(&self) -> Result<super::Placements, Error> {
        Ok(self.get()?.placements.clone())
    }

    fn get(&self) -> Result<&Setup, Error> {
        self.0.get().ok_or_else(|| Error::internal("tenant moves are used before modules started"))
    }

    /// Names of every enabled module and the control-plane tables (scheduled backups).
    pub(crate) fn tables(&self) -> Result<(Vec<&'static str>, Vec<&'static str>), Error> {
        let setup = self.get()?;
        Ok((setup.modules.clone(), setup.control_plane.clone()))
    }

    /// Like [`tables`](Self::tables), with the turned-off modules too: what deleting, exporting, moving,
    /// splitting and merging a tenant cover (dormant mode). A module whose tables do not exist in a database
    /// simply has no table there.
    pub(crate) fn tenant_tables(&self) -> Result<(Vec<&'static str>, Vec<&'static str>), Error> {
        let setup = self.get()?;
        let mut modules = setup.modules.clone();
        modules.extend(setup.dormant.iter().map(|(module, _)| *module));
        Ok((modules, setup.control_plane.clone()))
    }
}

/// The module is installed at `database`: its history table records at least one migration (the module ran
/// there and was not migrated all the way down).
async fn installed_on(module: &str, database: &Database) -> Result<bool, Error> {
    Ok(crate::migration::module_installed(database.read(), &format!("{module}_migrations")).await?)
}

/// Result of checking a move, before running it.
struct Checked {
    tenant: tenant::Model,
    from: placement::Model,
    to: placement::Model,
    blockers: Vec<Error>,
    resume: Option<Model>,
    tables: Vec<TenantTableRows>,
    files: u64,
}

/// Moves tenants between placements. Published by the `kernel` module (`context.require::<TenantMoves>()`).
pub struct TenantMoves {
    registry: DatabaseRegistry,
    setup: MoveSetup,
    outbox: Outbox,
    jobs: JobQueue,
    /// Waits this long after setting `moving` before copying (tenant cache TTL + the maximum time of one
    /// request).
    settle: Duration,
    translations: Translations,
}

impl std::fmt::Debug for TenantMoves {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TenantMoves").field("settle", &self.settle).finish_non_exhaustive()
    }
}

fn transfer(error: TransferError) -> Error {
    Error::internal(error.to_string())
}

/// An error with its root cause, for the operator to read (column `error`, log).
pub(super) fn describe(error: &Error) -> String {
    match std::error::Error::source(error) {
        Some(source) => format!("{}: {source}", error.code()),
        None => format!("{}: {}", error.code(), error.message()),
    }
}

impl TenantMoves {
    pub(crate) fn new(
        registry: DatabaseRegistry,
        setup: MoveSetup,
        outbox: Outbox,
        jobs: JobQueue,
        request_timeout: Duration,
        translations: Translations,
    ) -> Self {
        let settle = Duration::from_secs(registry.config().cache_seconds) + request_timeout;
        TenantMoves { registry, setup, outbox, jobs, settle, translations }
    }

    fn control(&self) -> &Database {
        self.registry.control()
    }

    async fn tenant_row(&self, id_or_slug: &str) -> Result<tenant::Model, Error> {
        let select = match id_or_slug.parse::<Uuid>() {
            Ok(id) => tenant::Entity::find_by_id(id),
            Err(_) => tenant::Entity::find().filter(tenant::Column::Slug.eq(id_or_slug)),
        };
        select
            .one(self.control().write())
            .await?
            .ok_or_else(|| errors::tenant_not_found(id_or_slug))
    }

    async fn placements(&self) -> Result<Vec<placement::Model>, Error> {
        Ok(placement::Entity::find().all(self.control().write()).await?)
    }

    async fn unfinished(&self, tenant: Uuid) -> Result<Option<Model>, Error> {
        Ok(Entity::find()
            .filter(Column::TenantId.eq(tenant))
            .filter(Column::State.is_in(UNFINISHED))
            .one(self.control().write())
            .await?)
    }

    async fn database(&self, placement: &placement::Model) -> Result<Database, Error> {
        self.registry
            .placement_database(PlacementId::from_uuid(placement.id))
            .await
            .map_err(|error| errors::tenant_move_unreachable(&placement.name, &error.message()))
    }
}

/// Runner of the job `kernel.move_tenant` (a control-database job, belonging to no tenant): runs the move
/// to completion. A rerun of the same job takes back its own lease immediately.
pub(crate) struct MoveTenantRunner(pub(crate) Arc<TenantMoves>);

#[async_trait::async_trait]
impl JobHandler for MoveTenantRunner {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let payload: MoveTenant =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        let moved = self.0.run(payload.move_id, &format!("job-{}", job.id)).await?;
        tracing::info!(move_id = %moved.id, state = %moved.state, "tenant move job finished");
        Ok(())
    }
}
