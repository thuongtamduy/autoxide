//! Splitting / merging tenants (step 38 item 3, D-20261118-step38-relocation): split an organization branch of
//! tenant T into a separate tenant T' in another database, or merge tenant T' into tenant T under a node,
//! **keeping primary keys and history unchanged**.
//!
//! - Each module declares how each of its tenant-level tables travels ([`Module::relocation`]): by node
//!   ([`TablePolicy::ByOrg`]), by parent row ([`TablePolicy::FollowParent`]), copied by the rows that point to
//!   them ([`TablePolicy::Referenced`]), whole tenant ([`TablePolicy::TenantWide`]), must be empty
//!   ([`TablePolicy::MustBeEmpty`]) or handled by the module ([`TablePolicy::Custom`]). An undeclared table is
//!   inferred from its columns (has `org_id` → by node; job queue / outbox → must be empty; otherwise →
//!   stays), with a warning in the plan; the binary's test rejects undeclared tables.
//! - One run is one `kernel_relocations` row (control plane) that goes through `planned` → `frozen` →
//!   `copied` → `verified` → `switched` → `cleaned` → `done` (or `failed`); each step is re-runnable
//!   (`autoxide tenant split --resume <id>`, job `kernel.relocate`). There is no transaction across the two
//!   databases: copying is ONE transaction at the target, switching is one transaction at the control
//!   database, and cleaning deletes at the source only rows that already exist at the target.
//! - Source and target must be two different databases (to move within the same database, use
//!   `autoxide tenant move` first). On a split, the new tenant has an ID equal to the ID of the branch root
//!   node (a tenant's root node has an ID equal to the tenant ID, D-198), so merging back returns every row
//!   to its old place.
//! - Only platform staff can run it (CLI, or REST with `dry_run` by default, D-210); a tenant only sends a
//!   request (`POST /api/v1/kernel/tenant/relocation-requests`), which goes into the platform staff's inbox.
//!
//! [`Module::relocation`]: crate::module::Module::relocation

mod http;
mod kernel_tables;
mod plan;
mod policy;
mod prepare;
mod run;
mod steps;
mod store;

use crate::time::Timestamp;
use sea_orm::DatabaseTransaction;
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, Query};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{OrgId, TenantId};
use crate::access::{Permission, Risk};
use crate::config::Setting;
use crate::database::Database;
use crate::error::Error;
use crate::event::EventInfo;

pub(crate) use http::{RelocationRequestsWork, routes};
pub(crate) use kernel_tables::kernel_relocation;
pub(crate) use policy::Policies;
pub(crate) use run::Wiring;
pub use run::{RelocationInput, TenantRelocations};
pub(crate) use store::CreateRelocations;

/// Splits / merges tenants (platform staff only).
pub const TENANTS_RELOCATE: Permission = Permission::new(
    "kernel.tenants.relocate",
    "Split a branch of a tenant into a tenant of its own, or merge a tenant into another (platform \
     staff only); the tenants answer 503 while it runs.",
)
.with_risk(Risk::Destructive);

/// A tenant administrator sends a split / merge request to platform staff.
pub const RELOCATIONS_REQUEST: Permission = Permission::new(
    "kernel.relocations.request",
    "Ask the platform staff to split a branch of your tenant into a tenant of its own, or to merge \
     tenants.",
);

pub(crate) const RELOCATION_STARTED: EventInfo = EventInfo::new(
    "kernel.relocation_started",
    "A split or merge of tenants started; the tenant answers 503 until it is done.",
);
pub(crate) const RELOCATION_FINISHED: EventInfo = EventInfo::new(
    "kernel.relocation_finished",
    "A split or merge of tenants finished: the rows moved to the target tenant.",
);
pub(crate) const RELOCATION_FAILED: EventInfo = EventInfo::new(
    "kernel.relocation_failed",
    "A split or merge of tenants stopped before changing anything; the tenant works again.",
);

/// Waits until the tenant's due outbox / jobs are empty before copying.
pub const DRAIN_SECONDS: Setting = Setting::new(
    "AUTOXIDE_RELOCATION_DRAIN_SECONDS",
    "Seconds a split or merge of tenants waits for the tenant's queued events and due jobs to run \
     before it gives up (1 to 86400).",
)
.with_default("300");

/// Job kind that runs a split / merge started through REST.
pub(crate) const RELOCATE: &str = "kernel.relocate";

// Tách hay gộp.
/// Split or merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RelocationKind {
    /// A branch of the organization tree becomes a tenant of its own.
    Split,
    /// A tenant joins another tenant under one of its units.
    Merge,
}

impl RelocationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RelocationKind::Split => "split",
            RelocationKind::Merge => "merge",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "split" => Some(RelocationKind::Split),
            "merge" => Some(RelocationKind::Merge),
            _ => None,
        }
    }
}

/// A `TenantWide` row on a split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    /// Stays with the source tenant; the new tenant has none.
    Stay,
    /// Copied to the new tenant while the source tenant keeps it (shared configuration: roles…).
    Copy,
}

/// A `TenantWide` row on a merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Merge {
    /// Moves to the receiving tenant (a row whose primary key already exists at the receiving tenant, the copy
    /// made at split time, is skipped: it is the same row).
    Import,
    /// Not moved: stays with the merged tenant (subscriptions, sessions, counters…).
    Skip,
}

/// How a tenant-level table travels on a split / merge.
#[derive(Clone)]
#[non_exhaustive]
pub enum TablePolicy {
    /// A row whose node `column` belongs to the split branch goes with the branch; `NULL` or a node outside the branch stays.
    ByOrg { column: &'static str },
    /// A table that mirrors the organization tree (one row per node, `id` is the node ID, `parent` is the
    /// parent node): like `ByOrg` on `id`, and the parent node of the branch root becomes `NULL` on a split and
    /// becomes the receiving node on a merge.
    OrgTree { id: &'static str, parent: &'static str },
    /// A row follows its parent row in `parent_table` (`column` points to `parent_column`): when the parent
    /// goes, the child goes; when the parent is copied, the child is copied.
    FollowParent { parent_table: &'static str, column: &'static str, parent_column: &'static str },
    /// A row that a row of `by_table` (moved or copied) points to through `by_column` → `column`: it is COPIED
    /// to the new tenant while the source tenant keeps it (e.g. the account of a person with a role at a node
    /// in the branch).
    Referenced { by_table: &'static str, by_column: &'static str, column: &'static str },
    /// Whole tenant, not by node.
    TenantWide { on_split: Split, on_merge: Merge },
    /// A queue (outbox, job): the tenant's due part must finish running before copying; not copied.
    MustBeEmpty,
    /// A table handled at the same time by table `by` of the same module (e.g. the head of the audit hash chain).
    HandledBy { by: &'static str },
    /// The module handles it itself (audit, organization tree, number sequences).
    Custom(Arc<dyn RelocateTable>),
}

impl TablePolicy {
    /// Policy name in the plan.
    pub fn name(&self) -> &'static str {
        match self {
            TablePolicy::ByOrg { .. } => "by_org",
            TablePolicy::OrgTree { .. } => "org_tree",
            TablePolicy::FollowParent { .. } => "follow_parent",
            TablePolicy::Referenced { .. } => "referenced",
            TablePolicy::TenantWide { .. } => "tenant_wide",
            TablePolicy::MustBeEmpty => "must_be_empty",
            TablePolicy::HandledBy { .. } => "handled_by",
            TablePolicy::Custom(_) => "custom",
        }
    }
}

impl std::fmt::Debug for TablePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A column that holds a secret encrypted per tenant (`Sealer`, AAD includes the tenant): when the tenant
/// changes, the value is re-encrypted for the new tenant; otherwise the new tenant could not open it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SealedColumn {
    pub table: &'static str,
    pub column: &'static str,
    /// The purpose used when encrypting (e.g. `webhooks.endpoint_secret`).
    pub purpose: &'static str,
    /// The row-identifying column used when encrypting (usually `id`).
    pub row_column: &'static str,
}

/// Split / merge policy of a module ([`Module::relocation`](crate::module::Module::relocation)).
#[derive(Clone, Default)]
pub struct Relocation {
    tables: Vec<(&'static str, TablePolicy)>,
    sealed: Vec<SealedColumn>,
    switch_hooks: Vec<Arc<dyn SwitchHook>>,
}

impl std::fmt::Debug for Relocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Relocation")
            .field("tables", &self.tables)
            .field("sealed", &self.sealed)
            .field("switch_hooks", &self.switch_hooks.len())
            .finish()
    }
}

/// A split / merge run that is switching (the `switch` step).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RelocationSwitch {
    pub kind: RelocationKind,
    /// The tenant whose data leaves (the tenant a branch is split from; the tenant being merged).
    pub source: TenantId,
    /// The receiving tenant (the new tenant on a split; the receiving tenant on a merge).
    pub target: TenantId,
}

/// Work a module must do SYNCHRONOUSLY when a split / merge run switches (D-20261119-relocation-switch-hook):
/// it runs inside the transaction of the `switch` step at the control database, BEFORE the new tenant becomes
/// active. On error the whole transaction is dropped: the new tenant does not open, and the run stops at the
/// previous step and can be re-run. Used for security policy keyed by tenant ID at the control database (e.g.
/// the permission fence of `auth`), which must take effect from the very first request to the new tenant.
/// Must be re-runnable.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait SwitchHook: Send + Sync {
    async fn switch(
        &self,
        control: &DatabaseTransaction,
        relocation: &RelocationSwitch,
    ) -> Result<(), Error>;
}

impl Relocation {
    /// Runs `hook` in the switch step of every split / merge run ([`SwitchHook`]).
    ///
    /// Old way: contribute the hook in `start` with `context.contribute::<dyn SwitchHook>(…)` instead.
    pub fn on_switch(mut self, hook: Arc<dyn SwitchHook>) -> Self {
        self.switch_hooks.push(hook);
        self
    }

    /// The declared switch-step hooks.
    pub fn switch_hooks(&self) -> &[Arc<dyn SwitchHook>] {
        &self.switch_hooks
    }

    /// Declares nothing: every table is inferred from its columns (with warnings in the plan). The default
    /// implementation of `Module::relocation`.
    pub fn inferred() -> Self {
        Relocation::default()
    }

    pub fn new() -> Self {
        Relocation::default()
    }

    /// Table `table` follows `policy`.
    pub fn table(mut self, table: &'static str, policy: TablePolicy) -> Self {
        self.tables.push((table, policy));
        self
    }

    /// Tables by node in the column `org_id`.
    pub fn by_org(self, tables: &[&'static str]) -> Self {
        tables
            .iter()
            .fold(self, |this, table| this.table(table, TablePolicy::ByOrg { column: "org_id" }))
    }

    /// Table `table` follows its parent row in `parent_table` (`column` points to `parent_table.id`).
    pub fn follow(
        self,
        table: &'static str,
        column: &'static str,
        parent_table: &'static str,
    ) -> Self {
        self.table(table, TablePolicy::FollowParent { parent_table, column, parent_column: "id" })
    }

    /// Table `table` is copied by the rows of `by_table` that point to it (`by_table.by_column` → `id`).
    pub fn referenced(
        self,
        table: &'static str,
        by_table: &'static str,
        by_column: &'static str,
    ) -> Self {
        self.table(table, TablePolicy::Referenced { by_table, by_column, column: "id" })
    }

    /// Whole-tenant configuration: copied to the new tenant on a split, moved on a merge.
    pub fn copy_on_split(self, tables: &[&'static str]) -> Self {
        self.tenant_wide(tables, Split::Copy, Merge::Import)
    }

    /// Whole-tenant data stays with the source tenant on a split and moves on a merge (e.g. buyers, subscribers).
    pub fn stay(self, tables: &[&'static str]) -> Self {
        self.tenant_wide(tables, Split::Stay, Merge::Import)
    }

    /// Temporary or tenant-private data (sessions, counters, tokens, subscriptions): goes nowhere.
    pub fn ephemeral(self, tables: &[&'static str]) -> Self {
        self.tenant_wide(tables, Split::Stay, Merge::Skip)
    }

    fn tenant_wide(self, tables: &[&'static str], on_split: Split, on_merge: Merge) -> Self {
        tables.iter().fold(self, |this, table| {
            this.table(table, TablePolicy::TenantWide { on_split, on_merge })
        })
    }

    /// A queue that must be empty (outbox, job; recognized by columns so it usually needs no declaration).
    pub fn must_be_empty(self, tables: &[&'static str]) -> Self {
        tables.iter().fold(self, |this, table| this.table(table, TablePolicy::MustBeEmpty))
    }

    /// Table `table` is handled at the same time by table `by`.
    pub fn handled_by(self, table: &'static str, by: &'static str) -> Self {
        self.table(table, TablePolicy::HandledBy { by })
    }

    /// Table `table` is handled by the module itself.
    pub fn custom(self, table: &'static str, handler: Arc<dyn RelocateTable>) -> Self {
        self.table(table, TablePolicy::Custom(handler))
    }

    /// Column `table.column` holds a secret encrypted per tenant with `purpose`, rows identified by
    /// `row_column`.
    pub fn sealed(
        mut self,
        table: &'static str,
        column: &'static str,
        purpose: &'static str,
        row_column: &'static str,
    ) -> Self {
        self.sealed.push(SealedColumn { table, column, purpose, row_column });
        self
    }

    /// The declared tables.
    pub fn tables(&self) -> &[(&'static str, TablePolicy)] {
        &self.tables
    }

    /// The declared secret columns.
    pub fn sealed_columns(&self) -> &[SealedColumn] {
        &self.sealed
    }
}

/// What a module does itself for one table ([`TablePolicy::Custom`]). Every function is re-runnable: `copy`
/// runs in a transaction at the target (on failure the target is as if not copied), `clean` in a transaction
/// at the source and deletes only rows that already exist at the target.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait RelocateTable: Send + Sync {
    /// Number of rows that will go, conflicts, warnings; changes nothing.
    async fn plan(&self, context: &RelocationContext) -> Result<TableReport, Error>;
    /// Writes the table's part to the target (through `target`); returns the number of rows written.
    async fn copy(
        &self,
        context: &RelocationContext,
        target: &DatabaseTransaction,
    ) -> Result<u64, Error>;
    /// Deletes at the source (through `source`) the part that has moved; returns the number of rows deleted.
    async fn clean(
        &self,
        context: &RelocationContext,
        source: &DatabaseTransaction,
    ) -> Result<u64, Error>;
}

/// Plan of one `Custom` table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TableReport {
    pub rows: u64,
    pub conflicts: Vec<RelocationConflict>,
    pub warnings: Vec<String>,
}

impl TableReport {
    pub fn rows(rows: u64) -> Self {
        TableReport { rows, ..TableReport::default() }
    }
}

/// What a `Custom` table needs to know about the split / merge run.
#[derive(Clone)]
pub struct RelocationContext {
    id: Uuid,
    kind: RelocationKind,
    source_tenant: TenantId,
    target_tenant: TenantId,
    source: Database,
    target: Database,
    org: OrgId,
    moved: Arc<HashSet<Uuid>>,
    tables: crate::transfer::TenantTables,
}

impl RelocationContext {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: Uuid,
        kind: RelocationKind,
        source_tenant: TenantId,
        target_tenant: TenantId,
        source: Database,
        target: Database,
        org: OrgId,
        moved: HashSet<Uuid>,
        tables: crate::transfer::TenantTables,
    ) -> Self {
        RelocationContext {
            id,
            kind,
            source_tenant,
            target_tenant,
            source,
            target,
            org,
            moved: Arc::new(moved),
            tables,
        }
    }

    /// Tenant-level tables of the two databases (read / write rows by column type).
    pub(crate) fn tables(&self) -> &crate::transfer::TenantTables {
        &self.tables
    }

    /// ID of the split / merge run.
    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn kind(&self) -> RelocationKind {
        self.kind
    }

    /// The tenant whose data leaves (T on a split, T' on a merge).
    pub fn source_tenant(&self) -> TenantId {
        self.source_tenant
    }

    /// The tenant that receives the data (T' on a split, T on a merge).
    pub fn target_tenant(&self) -> TenantId {
        self.target_tenant
    }

    /// Database of the source tenant.
    pub fn source(&self) -> &Database {
        &self.source
    }

    /// Database of the target tenant.
    pub fn target(&self) -> &Database {
        &self.target
    }

    /// Split: the branch root node (= the new tenant's ID). Merge: the node of the receiving tenant under which
    /// the root of the merged tenant becomes a child.
    pub fn org(&self) -> OrgId {
        self.org
    }

    /// Split: `id` is a node of the branch, or the primary key of a row that goes / is copied under the general
    /// policy. Merge: always `true` (everything of the merged tenant goes).
    pub fn moved(&self, id: Uuid) -> bool {
        self.kind == RelocationKind::Merge || self.moved.contains(&id)
    }

    /// Condition "a source tenant row whose `column` is a node of the branch" (split), or "a source tenant row"
    /// (merge). Read at the source database.
    pub fn branch_condition(&self, column: &str) -> Condition {
        let mine = Condition::all()
            .add(Expr::col(Alias::new("tenant_id")).eq(self.source_tenant.as_uuid()));
        match self.kind {
            RelocationKind::Merge => mine,
            RelocationKind::Split => mine.add(
                Expr::col(Alias::new(column)).in_subquery(
                    Query::select()
                        .column(Alias::new("descendant_id"))
                        .from(Alias::new("kernel_org_closure"))
                        .and_where(
                            Expr::col(Alias::new("tenant_id")).eq(self.source_tenant.as_uuid()),
                        )
                        .and_where(Expr::col(Alias::new("ancestor_id")).eq(self.org.as_uuid()))
                        .to_owned(),
                ),
            ),
        }
    }
}

/// Something that blocks a split or merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RelocationBlocker {
    /// Stable code, the same as the error a real run would answer.
    #[schema(example = "tenancy.relocation_same_database")]
    pub code: String,
    pub message: String,
}

/// Rows that cannot move as they are: a reference across the boundary or a unique value taken in
/// the target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RelocationConflict {
    #[schema(example = "content_entries")]
    pub table: String,
    /// `reference` (a row that stays points to a row that goes, or the other way), `unique` (the
    /// target tenant already has the value) or a code of the module.
    #[schema(example = "unique")]
    pub code: String,
    pub message: String,
    /// Rows in conflict.
    pub count: u64,
    /// Keys of the first rows in conflict (at most 20).
    pub rows: Vec<String>,
}

impl RelocationConflict {
    pub fn new(
        table: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
        rows: Vec<String>,
    ) -> Self {
        let count = rows.len() as u64;
        RelocationConflict {
            table: table.into(),
            code: code.into(),
            message: message.into(),
            count,
            rows: rows.into_iter().take(20).collect(),
        }
    }
}

/// Something the operator should know; it does not block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RelocationWarning {
    #[schema(example = "shop_notes")]
    pub table: String,
    #[schema(example = "relocation_policy_inferred")]
    pub code: String,
    pub message: String,
}

/// What happens to one table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RelocationTable {
    #[schema(example = "ledger_entries")]
    pub table: String,
    /// The policy of the table: `by_org`, `org_tree`, `follow_parent`, `referenced`,
    /// `tenant_wide`, `must_be_empty`, `handled_by` or `custom`.
    #[schema(example = "follow_parent")]
    pub policy: String,
    /// `move` (copied to the target, then deleted from the source), `copy` (the source keeps
    /// them too), `stay`, `drain` (due queue rows that must run first) or `custom`.
    #[schema(example = "move")]
    pub action: String,
    /// Rows concerned.
    pub rows: u64,
    /// Merge: rows whose key the target already has (the copy made when the tenant was split);
    /// they are skipped as the same rows.
    pub existing: u64,
}

/// What a split or merge would do (dry run). Nothing is changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RelocationPlan {
    pub kind: RelocationKind,
    /// Tenant whose rows go.
    pub source_tenant_id: Uuid,
    #[schema(example = "acme")]
    pub source: String,
    /// Tenant that receives them (split: the new tenant, its id is the id of the branch unit).
    pub target_tenant_id: Uuid,
    #[schema(example = "acme-north")]
    pub target: String,
    /// Split: the root unit of the branch. Merge: the unit of the receiving tenant the merged
    /// tenant goes under.
    pub org_id: Uuid,
    /// Placement (database) of the target.
    #[schema(example = "acme-north-db")]
    pub placement: String,
    /// `true` when nothing blocks and there is no conflict.
    pub ready: bool,
    pub blocking: Vec<RelocationBlocker>,
    pub conflicts: Vec<RelocationConflict>,
    pub warnings: Vec<RelocationWarning>,
    /// Every tenant-level table, in copy order.
    pub tables: Vec<RelocationTable>,
    /// Rows that move or are copied.
    pub rows: u64,
}

/// A split or merge of tenants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TenantRelocation {
    pub id: Uuid,
    pub kind: RelocationKind,
    /// `requested` (asked by the tenant, waiting for the platform staff), `planned`, `frozen`
    /// (the tenants answer 503), `copied`, `verified`, `switched`, `cleaned`, `done` or `failed`.
    #[schema(example = "done")]
    pub state: String,
    pub source_tenant_id: Uuid,
    pub target_tenant_id: Option<Uuid>,
    pub org_id: Option<Uuid>,
    pub placement: Option<String>,
    pub reason: String,
    /// The plan made when it started (or when it was requested).
    pub plan: Option<RelocationPlan>,
    /// Rows written to the target per table (after `copied`).
    pub tables: Vec<super::TenantTableRows>,
    /// Why the last run stopped; running it again resumes it.
    pub error: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub finished_at: Option<Timestamp>,
}

/// Error of the split / merge part (stable code, translation in `crates/kernel/locales/vi.json`).
pub(crate) mod errors {
    use crate::error::Error;

    pub fn conflict(count: usize, id: &str) -> Error {
        Error::conflict(
            "tenancy.relocation_conflict",
            "The split or merge cannot run: {count} conflicts, see the report of run `{id}`.",
        )
        .with_param("count", count)
        .with_param("id", id)
    }

    pub fn in_progress(id: &str) -> Error {
        Error::conflict(
            "tenancy.relocation_in_progress",
            "This tenant has a split or merge that is not finished (`{id}`). Resume it with \
             `--resume` or wait until it is done.",
        )
        .with_param("id", id)
    }

    pub fn not_found(id: &str) -> Error {
        Error::not_found("tenancy.relocation_not_found", "There is no split or merge `{id}`.")
            .with_param("id", id)
    }

    pub fn org_invalid(org: &str, tenant: &str) -> Error {
        Error::conflict(
            "tenancy.relocation_org_invalid",
            "Unit `{org}` is not a unit below the root of tenant `{tenant}`.",
        )
        .with_param("org", org)
        .with_param("tenant", tenant)
    }

    pub fn same_database(placement: &str) -> Error {
        Error::conflict(
            "tenancy.relocation_same_database",
            "The target placement `{placement}` is the database of the source tenant. Split into \
             another database, or move one tenant first with `autoxide tenant move`.",
        )
        .with_param("placement", placement)
    }

    pub fn tenant_exists(id: &str) -> Error {
        Error::conflict(
            "tenancy.relocation_tenant_exists",
            "A tenant with id `{id}` already exists, so the branch cannot become a tenant with \
             that id.",
        )
        .with_param("id", id)
    }

    pub fn not_active(tenant: &str, status: &str) -> Error {
        Error::conflict(
            "tenancy.relocation_not_active",
            "Tenant `{tenant}` is `{status}`; only an active tenant can be split or merged.",
        )
        .with_param("tenant", tenant)
        .with_param("status", status)
    }

    pub fn platform() -> Error {
        Error::conflict(
            "tenancy.relocation_platform",
            "The platform tenant cannot be split or merged.",
        )
    }

    pub fn cycle(tenant: &str) -> Error {
        Error::conflict(
            "tenancy.relocation_cycle",
            "Tenant `{tenant}` is below the tenant being merged; merge in the other direction.",
        )
        .with_param("tenant", tenant)
    }

    pub fn schema_differs(placement: &str, pending: usize) -> Error {
        Error::conflict(
            "tenancy.relocation_schema_differs",
            "The database of placement `{placement}` misses {pending} migrations. Run \
             `autoxide migrate up --all` first.",
        )
        .with_param("placement", placement)
        .with_param("pending", pending)
    }

    pub fn drain_timeout(rows: u64) -> Error {
        Error::conflict(
            "tenancy.relocation_drain_timeout",
            "{rows} queued events or due jobs of the tenant did not run in time; the tenant works \
             again. Try later, or raise AUTOXIDE_RELOCATION_DRAIN_SECONDS.",
        )
        .with_param("rows", rows)
    }

    pub fn count_mismatch(table: &str, source: u64, target: u64) -> Error {
        Error::conflict(
            "tenancy.relocation_count_mismatch",
            "Table `{table}`: {source} rows should be in the target, {target} are. Run the split \
             or merge again to copy again.",
        )
        .with_param("table", table)
        .with_param("source", source)
        .with_param("target", target)
    }
}
