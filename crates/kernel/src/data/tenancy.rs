//! Multi-tenancy foundation (step 33, D-185, D-253): one installation serves many **customers** (tenants) whose
//! data never mixes.
//!
//! - [`TenantId`]: the tenant's ID. [`DEFAULT_TENANT`] is the default tenant, which is also the platform tenant
//!   where the operator's accounts live; the data of an older installation belongs to it.
//! - [`TenantContext`]: the tenant of a request / job / event, with the database that holds the tenant's data.
//!   Every service function that touches tenant-level data takes it and gets the database through
//!   [`TenantContext::database`], so forgetting the tenant leaves no database to query (D-253 item 1).
//! - [`DatabaseRegistry`]: the database registry: the control database (`DATABASE_URL`) and the placements
//!   (data locations: `shared` or `dedicated`), opened lazily, with a limit.
//! - Mode ([`TenancyMode`], `AUTOXIDE_TENANCY_MODE`): `single` (the default: one tenant, no extra query
//!   per request) or `multi`.
//! - A tenant's own domains ([`TenantDomains`], `domains.rs`): verified with a TXT record; a request
//!   to a verified domain goes straight to the tenant (D-190).
//! - Moving a tenant between placements ([`TenantMoves`], `moves.rs`): `shared` ↔ `dedicated`, safe to rerun
//!   after a failure midway.
//! - The token in an email link that carries the tenant ([`link_token`], [`resolve_link_token`],
//!   [`OptionalTenant`]): clicking a link on the main domain still reaches the right tenant (D-281).
//!
//! The control-plane tables (`kernel_tenants`, `kernel_placements`, `kernel_tenant_domains`) are created by the
//! internal `kernel` module (`schema.rs`); they hold data only in the control database.

pub(crate) mod admin;
mod config;
mod current;
pub mod distribution;
pub(crate) mod domains;
pub(crate) mod erasure;
pub(crate) mod exports;
mod link_token;
pub mod migration;
pub(crate) mod migration_runs;
pub(crate) mod moves;
pub(crate) mod org_settings;
pub(crate) mod org_tree;
mod placement_health;
mod placements;
mod registry;
pub mod relocate;
pub(crate) mod request;
pub(crate) mod residency;
pub mod sandbox;
pub(crate) mod schema;
mod scope;
pub(crate) mod settings;
mod site;
mod verified_domains;

use crate::time::Timestamp;
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::database::Database;
use crate::error::Error;

pub use admin::{
    NewTenant, PLACEMENTS_READ, Placement, PlacementStorage, TENANTS_MANAGE, TENANTS_READ, Tenant,
    TenantAdmin, TenantChanges, TenantExpanded, TenantFilter, TenantSeed, require_platform,
};
pub use config::{TenancyConfig, WorkerPlacements};
pub use current::CurrentTenant;
pub use domains::{
    CHALLENGE_PREFIX, CHALLENGE_VALUE_PREFIX, DOMAINS_MANAGE, DnsTxtLookup, DomainRecord,
    TenantDomain, TenantDomains, normalize_domain,
};
pub use erasure::{TENANTS_DELETE, TenantErasure};
pub use exports::{DATA_EXPORT, TenantExport, TenantExportLink, TenantExports};
pub use link_token::{OptionalTenant, link_token, resolve_link_token, split_link_token};
pub use migration_runs::PlacementMigrationQueue;

/// The `kernel.placements` check of `autoxide doctor`.
pub(crate) fn placement_health(
    registry: DatabaseRegistry,
) -> std::sync::Arc<dyn crate::doctor::Probe> {
    std::sync::Arc::new(placement_health::PlacementHealth { registry })
}
pub use moves::{
    TENANTS_MOVE, TenantMove, TenantMoveBlocker, TenantMovePlan, TenantMoves, TenantTableRows,
};
pub use org_settings::{OrgSetting, OrgSettingCatalog, OrgSettingCheck};
pub use org_tree::{OrgTree, nearest, root_immutable, root_of};
pub use placements::{
    MigrationReport, NewPlacement, PlacementCheck, PlacementFailure, PlacementMigrations,
    Placements, SchemaState,
};
pub use registry::{
    ChildTenant, DatabaseRegistry, FileStore, PLACEMENT_READ_URL, PLACEMENT_STORAGE, PLACEMENT_URL,
};
pub use relocate::{
    Merge, RELOCATIONS_REQUEST, RelocateTable, Relocation, RelocationBlocker, RelocationConflict,
    RelocationContext, RelocationInput, RelocationKind, RelocationPlan, RelocationSwitch,
    RelocationTable, RelocationWarning, SealedColumn, Split, SwitchHook, TENANTS_RELOCATE,
    TablePolicy, TableReport, TenantRelocation, TenantRelocations,
};
pub use request::{TENANT_HEADER, TenantHint};
pub use residency::{Residency, parse_country};
pub use sandbox::{
    NewSandbox, SANDBOXES_LIMIT, SANDBOXES_MANAGE, SANDBOXES_READ, Sandbox, TenantKind,
    TenantSandboxes,
};
pub use scope::{ScopeExt, TenantScoped};
pub use settings::{
    BUSINESS_MODEL, BUSINESS_MODELS, CONNECTION_GROUP, DOMAIN_GROUP, LINK_GROUP, PUBLIC_URL,
    SETTINGS_MANAGE, SettingCheck, SettingValue, SiteLinks, TenantSetting, TenantSettings,
    parsed_setting,
};
pub use site::{SITE_PARAM, Site, SiteQuery, site_not_found};

pub(crate) use config::read_setting;

/// Where the `kernel` module receives what every module provides once they have started (filled by
/// `Modules::start_in`): the seeders for new tenants, the DNS lookup, the organization tree, and what
/// moving a tenant needs.
pub(crate) struct Slots {
    pub(crate) seeds: admin::Seeds,
    pub(crate) dns: domains::DnsSlot,
    pub(crate) orgs: domains::OrgSlot,
    pub(crate) moves: moves::MoveSetup,
}

/// The ID of a tenant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TenantId(Uuid);

impl TenantId {
    /// The tenant with ID `id`.
    pub const fn from_uuid(id: Uuid) -> Self {
        TenantId(id)
    }

    /// The tenant's UUID (to write into the `tenant_id` column).
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<TenantId> for Uuid {
    fn from(id: TenantId) -> Self {
        id.0
    }
}

/// The ID of a node in a tenant's organization tree (step 34). A domain can be attached to a node (see
/// the wave 2 design).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OrgId(Uuid);

impl OrgId {
    pub const fn from_uuid(id: Uuid) -> Self {
        OrgId(id)
    }

    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for OrgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A tenant's organization tree as the kernel sees it: it only asks "does this node belong to the tenant".
/// The module that holds the organization tree (step 34) publishes `context.provide::<dyn OrgNodes>(…)` in
/// `Module::start`; until a module publishes it, the kernel treats the tenant as having no nodes (e.g. a
/// domain can only be attached to `null`).
///
/// Distinct from `OrgDirectory` of the `organizations` module (step 34, the full contract of the organization
/// tree for other modules): this is the narrow part the kernel needs, and the kernel depends on no module.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait OrgNodes: Send + Sync {
    /// `org` is a node (still usable) in the organization tree of tenant `tenant`.
    async fn has_node(&self, tenant: &TenantContext, org: OrgId) -> Result<bool, Error>;
}

/// The ID of a placement (a data location: one database).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlacementId(Uuid);

impl PlacementId {
    pub const fn from_uuid(id: Uuid) -> Self {
        PlacementId(id)
    }

    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for PlacementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The default tenant, which is also the platform tenant. A fixed ID in valid UUIDv7 form: a module's
/// migration assigns old data to it without reading a table of `kernel` (a deliberate exception, D-253 item
/// 3).
pub const DEFAULT_TENANT: TenantId =
    TenantId(Uuid::from_u128(0x0000_0000_0000_7000_8000_0000_0000_0001));

/// "No tenant": only for database-wide jobs (cleanup schedules) in the `<m>_jobs` table. `NULL` is not used
/// because a unique index treats `NULL`s as different from each other (D-253 item 4).
pub const NO_TENANT: TenantId = TenantId(Uuid::nil());

/// Placement `main`: the control database (`DATABASE_URL`). The default tenant lives here.
pub const MAIN_PLACEMENT: PlacementId =
    PlacementId(Uuid::from_u128(0x0000_0000_0000_7000_8000_0000_0000_0002));

/// `true` since every module has moved to per-tenant data (step 33 B14, D-253 item 6): the `multi` mode can be
/// enabled with `AUTOXIDE_TENANCY_MODE`.
pub const MULTI_TENANT_READY: bool = true;

/// Mode of the installation (`AUTOXIDE_TENANCY_MODE`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenancyMode {
    /// One tenant (the default tenant); reads no domain or header, runs no extra query.
    #[default]
    Single,
    /// Many tenants: the tenant is derived from the credential, header, domain.
    Multi,
}

/// Status of a tenant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantStatus {
    /// Being provisioned (database created, initial data seeded); not serving yet.
    Provisioning,
    Active,
    /// Suspended: every route of the tenant returns `tenant_suspended`.
    Suspended,
    /// Maintenance: every route of the tenant returns `tenant_maintenance`.
    Maintenance,
    /// Being moved to another database: treated like maintenance.
    Moving,
    /// Deleted for good (data deleted, only a row left in the tenant registry): requests get
    /// `tenant_not_found`.
    Deleted,
    /// Merged into another tenant (step 38): the data has moved to the receiving tenant; requests get
    /// `tenant_not_found`.
    Merged,
}

impl TenantStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TenantStatus::Provisioning => "provisioning",
            TenantStatus::Active => "active",
            TenantStatus::Suspended => "suspended",
            TenantStatus::Maintenance => "maintenance",
            TenantStatus::Moving => "moving",
            TenantStatus::Deleted => "deleted",
            TenantStatus::Merged => "merged",
        }
    }

    /// Status stored in the `status` column.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "provisioning" => Some(TenantStatus::Provisioning),
            "active" => Some(TenantStatus::Active),
            "suspended" => Some(TenantStatus::Suspended),
            "maintenance" => Some(TenantStatus::Maintenance),
            "moving" => Some(TenantStatus::Moving),
            "deleted" => Some(TenantStatus::Deleted),
            "merged" => Some(TenantStatus::Merged),
            _ => None,
        }
    }
}

/// Kind of placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementKind {
    /// Many tenants share one database (told apart by `tenant_id`).
    Shared,
    /// A database dedicated to one tenant.
    Dedicated,
}

impl PlacementKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PlacementKind::Shared => "shared",
            PlacementKind::Dedicated => "dedicated",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "shared" => Some(PlacementKind::Shared),
            "dedicated" => Some(PlacementKind::Dedicated),
            _ => None,
        }
    }
}

/// The tenant of a request, job or event, with the database that holds the tenant's data. Cheap to clone
/// (inside is an `Arc`); two contexts are equal when they have the same tenant.
#[derive(Clone)]
pub struct TenantContext(Arc<Inner>);

#[derive(Clone)]
struct Inner {
    id: TenantId,
    slug: String,
    name: String,
    status: TenantStatus,
    placement: PlacementId,
    database: Database,
    /// Whether the placement has applied all migrations (`schema_state = ready`); if not, the tenant gets
    /// `tenant_maintenance`.
    schema_ready: bool,
    /// The placement's own file store (`autoxide placement storage`); `None`: the shared store.
    storage: Option<Arc<dyn crate::storage::Storage>>,
    /// Read-only (`kernel_tenants.read_only`): writes are blocked where the request's tenant is resolved.
    read_only: bool,
    /// Sandbox (D-232): the original tenant and the expiry; `None` for a real tenant.
    sandbox: Option<(TenantId, Option<Timestamp>)>,
    /// The request came from a page on this tenant's own verified domain (CORS, D-20261125-tenant-cors).
    from_page_origin: bool,
}

impl TenantContext {
    /// Context of tenant `id` on `database` (built by the database registry and testkit; modules do not build
    /// it themselves).
    pub fn new(
        id: TenantId,
        slug: impl Into<String>,
        name: impl Into<String>,
        status: TenantStatus,
        placement: PlacementId,
        database: Database,
    ) -> Self {
        TenantContext(Arc::new(Inner {
            id,
            slug: slug.into(),
            name: name.into(),
            status,
            placement,
            database,
            schema_ready: true,
            storage: None,
            read_only: false,
            sandbox: None,
            from_page_origin: false,
        }))
    }

    /// Same tenant, with files stored at `storage` (the placement's own store) instead of the shared store.
    /// Built by the database registry; tests build it to try a placement with its own store.
    #[must_use]
    pub fn with_storage(self, storage: Arc<dyn crate::storage::Storage>) -> Self {
        let inner = Arc::unwrap_or_clone(self.0);
        TenantContext(Arc::new(Inner { storage: Some(storage), ..inner }))
    }

    /// The own file store of the tenant's placement; `None`: the tenant uses the shared store.
    /// [`TenantStorage::new`](crate::storage::TenantStorage::new) chooses it automatically.
    pub fn own_storage(&self) -> Option<&Arc<dyn crate::storage::Storage>> {
        self.0.storage.as_ref()
    }

    /// The placement with its own store that the tenant uses; `None`: the tenant uses the shared store. The
    /// job that cleans up orphan files compares against [`FileStore::placement`] so it only considers
    /// tenants of the store being scanned.
    pub fn storage_placement(&self) -> Option<PlacementId> {
        self.0.storage.as_ref().map(|_| self.0.placement)
    }

    /// Same tenant, marked as reached from a page on its own verified domain.
    #[must_use]
    pub(crate) fn reached_from_page(self) -> Self {
        let inner = Arc::unwrap_or_clone(self.0);
        TenantContext(Arc::new(Inner { from_page_origin: true, ..inner }))
    }

    /// Whether the request came from a page on this tenant's own verified domain (CORS).
    pub(crate) fn is_from_page_origin(&self) -> bool {
        self.0.from_page_origin
    }

    /// Same tenant, marked as having a placement that has not applied all migrations (built by the database
    /// registry when the placement's `schema_state` is not `ready`).
    #[must_use]
    pub fn with_schema_ready(self, ready: bool) -> Self {
        let inner = Arc::unwrap_or_clone(self.0);
        TenantContext(Arc::new(Inner { schema_ready: ready, ..inner }))
    }

    /// Whether the tenant's placement has applied all migrations.
    pub fn schema_ready(&self) -> bool {
        self.0.schema_ready
    }

    /// Same tenant, marked read-only (built by the database registry from `kernel_tenants.read_only`; tests
    /// build it).
    #[must_use]
    pub fn with_read_only(self, read_only: bool) -> Self {
        let inner = Arc::unwrap_or_clone(self.0);
        TenantContext(Arc::new(Inner { read_only, ..inner }))
    }

    /// Read-only tenant: reads work, writes through REST / MCP are blocked (`tenant_read_only`). The
    /// tenant's jobs and events still run (work accepted earlier, cleanup as the rules require).
    pub fn read_only(&self) -> bool {
        self.0.read_only
    }

    /// Same tenant, as a sandbox of tenant `of`, expiring at `expires_at` (built by the database registry from
    /// `kernel_tenants`; tests build it).
    #[must_use]
    pub fn with_sandbox_of(self, of: TenantId, expires_at: Option<Timestamp>) -> Self {
        let inner = Arc::unwrap_or_clone(self.0);
        TenantContext(Arc::new(Inner { sandbox: Some((of, expires_at)), ..inner }))
    }

    /// A test tenant (sandbox, D-232): partners are always simulated, mail and webhooks do not leave.
    pub fn is_sandbox(&self) -> bool {
        self.0.sandbox.is_some()
    }

    /// The original tenant of the sandbox; `None` for a real tenant.
    pub fn sandbox_of(&self) -> Option<TenantId> {
        self.0.sandbox.map(|(of, _)| of)
    }

    /// Expiry of the sandbox; `None` for a real tenant.
    pub fn sandbox_expires_at(&self) -> Option<Timestamp> {
        self.0.sandbox.and_then(|(_, at)| at)
    }

    /// Kind of tenant.
    pub fn kind(&self) -> TenantKind {
        if self.is_sandbox() { TenantKind::Sandbox } else { TenantKind::Production }
    }

    /// The default tenant on `database`: `single` mode, and the transition path for code that has not yet
    /// received a tenant context (D-253).
    pub fn default_on(database: Database) -> Self {
        Self::new(
            DEFAULT_TENANT,
            "default",
            "Default",
            TenantStatus::Active,
            MAIN_PLACEMENT,
            database,
        )
    }

    /// Transition path (step 33 A4): code that has not yet received a tenant context (modules not yet
    /// converted in part B) uses the default tenant on the database it holds. Marked `#[deprecated]` once
    /// part B is done (nobody left calling it), removed at the start of step 34 (D-253 item 7).
    #[doc(hidden)]
    #[deprecated(
        note = "dùng `TenantContext` của request, job hay event; cần khách mặc định thì `default_on`"
    )]
    pub fn legacy_default(database: &Database) -> Self {
        Self::default_on(database.clone())
    }

    pub fn id(&self) -> TenantId {
        self.0.id
    }

    /// Short name (lowercase, digits, `-`), unique across the platform.
    pub fn slug(&self) -> &str {
        &self.0.slug
    }

    pub fn name(&self) -> &str {
        &self.0.name
    }

    pub fn status(&self) -> TenantStatus {
        self.0.status
    }

    pub fn placement(&self) -> PlacementId {
        self.0.placement
    }

    /// The tenant's database handles (`crate::db`): every tenant record is filtered by this tenant.
    pub fn db(&self) -> crate::db::TenantDb<'_> {
        crate::db::TenantDb::new(self)
    }

    /// The database that holds the tenant's data.
    pub fn database(&self) -> &Database {
        &self.0.database
    }

    /// The platform tenant (the default tenant): where the operator's staff live.
    pub fn is_platform(&self) -> bool {
        self.0.id == DEFAULT_TENANT
    }

    /// Prefix of the tenant's file-store keys: empty for the default tenant (old files need not move),
    /// `tenants/<id>/` for other tenants.
    pub fn storage_prefix(&self) -> String {
        if self.is_platform() { String::new() } else { format!("tenants/{}/", self.0.id) }
    }
}

impl PartialEq for TenantContext {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

impl Eq for TenantContext {}

impl fmt::Debug for TenantContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TenantContext")
            .field("id", &self.0.id)
            .field("slug", &self.0.slug)
            .field("status", &self.0.status)
            .field("read_only", &self.0.read_only)
            .field("placement", &self.0.placement)
            .finish_non_exhaustive()
    }
}

/// Shared errors of the multi-tenancy part (stable codes, translations in `crates/kernel/locales/vi.json`).
pub mod errors {
    use crate::error::Error;

    /// The request's tenant cannot be determined.
    pub fn tenant_required() -> Error {
        Error::bad_request(
            "tenant_required",
            "No customer (tenant) could be told for this request. Send it to the tenant's domain, \
             add the header `Autoxide-Tenant: <slug>`, or sign in.",
        )
    }

    pub fn tenant_not_found(tenant: &str) -> Error {
        Error::not_found("tenant_not_found", "There is no tenant `{tenant}`.")
            .with_param("tenant", tenant)
    }

    pub fn tenant_suspended() -> Error {
        Error::forbidden(
            "tenant_suspended",
            "This tenant is suspended. Contact the service provider.",
        )
    }

    pub fn tenant_read_only() -> Error {
        Error::forbidden(
            "tenant_read_only",
            "This tenant is read-only for now: its data can be read and exported, but not \
             changed. Contact the service provider.",
        )
    }

    pub fn tenant_maintenance(seconds: u64) -> Error {
        Error::unavailable(
            "tenant_maintenance",
            "The data of this tenant is under maintenance. Retry in {seconds} seconds.",
        )
        .with_param("seconds", seconds)
        .with_retry_after(seconds)
    }

    pub fn tenant_mismatch() -> Error {
        Error::forbidden(
            "tenant_mismatch",
            "The credentials belong to another tenant than the one this request is for.",
        )
    }

    /// The request came from the web origin (`Origin`) of one tenant's verified domain but acts for another
    /// tenant: a page on a tenant's domain may only call the API as that tenant.
    pub fn tenant_origin_mismatch() -> Error {
        Error::forbidden(
            "tenant_origin_mismatch",
            "This request comes from a web page on another tenant's domain. A page on a tenant's \
             domain may only call the API for that tenant: call from that tenant's own page, or add \
             the page's origin to AUTOXIDE_HTTP_CORS_ORIGINS.",
        )
    }

    pub fn tenant_platform_only() -> Error {
        Error::forbidden("tenant_platform_only", "Only administrators of the platform can do this.")
    }

    pub fn tenant_single_mode() -> Error {
        Error::conflict(
            "tenant_single_mode",
            "This installation runs in single-tenant mode; set AUTOXIDE_TENANCY_MODE=multi to add \
             tenants.",
        )
    }

    pub fn tenant_placement_taken(placement: &str) -> Error {
        Error::conflict(
            "tenant_placement_taken",
            "There is already a placement named `{placement}`.",
        )
        .with_param("placement", placement)
    }

    pub fn tenant_placement_behind(placement: &str) -> Error {
        Error::conflict(
            "tenant_placement_behind",
            "The placement `{placement}` has migrations to apply: run `autoxide migrate up --all` \
             first.",
        )
        .with_param("placement", placement)
    }

    pub fn tenant_residency_violation(country: &str) -> Error {
        Error::conflict(
            "tenant_residency_violation",
            "The tenant must keep its data in {country}; this placement does not meet that (its \
             country is unknown or different). Set the placement's country with `autoxide \
             placement country`, or choose a placement in {country}.",
        )
        .with_param("country", country)
    }

    pub fn tenant_slug_taken(slug: &str) -> Error {
        Error::conflict("tenant_slug_taken", "Another tenant already uses the slug `{slug}`.")
            .with_param("slug", slug)
    }

    pub fn tenant_setting_unknown(name: &str) -> Error {
        Error::new(
            crate::error::ErrorKind::Validation,
            "tenant_setting_unknown",
            "No module takes the setting `{name}`. List them with `GET /api/v1/kernel/settings`.",
        )
        .with_param("name", name)
    }

    pub fn tenant_placement_not_found(placement: &str) -> Error {
        Error::not_found("tenant_placement_not_found", "There is no placement `{placement}`.")
            .with_param("placement", placement)
    }

    pub fn tenant_placement_storage_in_use(placement: &str, tenants: u64) -> Error {
        Error::conflict(
            "tenant_placement_storage_in_use",
            "{tenants} tenant(s) keep their data on the placement `{placement}`; changing its file \
             storage would hide their files. Move them to another placement first, or pass \
             `--force` when the new storage already holds the same files (e.g. a new access key).",
        )
        .with_param("placement", placement)
        .with_param("tenants", tenants)
    }

    pub fn tenant_domain_invalid(domain: &str) -> Error {
        // Giá trị do client gửi: chỉ nhắc lại phần đầu.
        let shown: String = domain.chars().take(100).collect();
        Error::new(
            crate::error::ErrorKind::Validation,
            "tenant_domain_invalid",
            "`{domain}` is not a domain that can be used: send a name such as `shop.example` \
             (punycode for international names), not an IP address, `localhost` or the \
             platform's own domain.",
        )
        .with_param("domain", shown)
    }

    pub fn tenant_domain_taken(domain: &str) -> Error {
        Error::conflict("tenant_domain_taken", "The domain `{domain}` belongs to another tenant.")
            .with_param("domain", domain)
    }

    pub fn tenant_domain_not_found() -> Error {
        Error::not_found(
            "tenant_domain_not_found",
            "This domain is not one of the current tenant's domains.",
        )
    }

    pub fn tenant_domain_revoked() -> Error {
        Error::conflict(
            "tenant_domain_revoked",
            "This domain was removed from the tenant and cannot be changed. Add it again with \
             `POST /api/v1/kernel/domains`.",
        )
    }

    pub fn tenant_domain_dns_failed(error: &str) -> Error {
        Error::unavailable("tenant_domain_dns_failed", "The DNS lookup failed: {error}")
            .with_param("error", error)
    }

    pub fn tenant_moving() -> Error {
        Error::conflict(
            "tenant_moving",
            "The tenant is being moved to another placement. Wait until the move is done, or \
             abort it with `autoxide tenant move <tenant> --abort`.",
        )
    }

    pub fn tenant_move_platform() -> Error {
        Error::conflict(
            "tenant_move_platform",
            "The platform tenant always keeps its data on the placement `main`; it cannot be moved.",
        )
    }

    pub fn tenant_move_same_placement(placement: &str) -> Error {
        Error::conflict(
            "tenant_move_same_placement",
            "The tenant already keeps its data on the placement `{placement}`.",
        )
        .with_param("placement", placement)
    }

    pub fn tenant_move_not_active(status: &str) -> Error {
        Error::conflict(
            "tenant_move_not_active",
            "Only an active tenant can be moved; this one is `{status}`.",
        )
        .with_param("status", status)
    }

    pub fn tenant_move_running(placement: &str) -> Error {
        Error::conflict(
            "tenant_move_running",
            "A move of this tenant to the placement `{placement}` is running now. Follow it with \
             the move status (`GET /api/v1/kernel/tenants/<tenant>/moves/<move>`) and try again when it is done.",
        )
        .with_param("placement", placement)
    }

    pub fn tenant_move_unfinished(placement: &str) -> Error {
        Error::conflict(
            "tenant_move_unfinished",
            "A move of this tenant to the placement `{placement}` stopped before the end. Resume \
             it by moving the tenant to `{placement}` again, or abort it with \
             `autoxide tenant move <tenant> --abort`.",
        )
        .with_param("placement", placement)
    }

    pub fn tenant_move_not_abortable() -> Error {
        Error::conflict(
            "tenant_move_not_abortable",
            "The tenant already uses its new placement; the move only has to clean the old one. \
             Resume it instead of aborting: move the tenant to the same placement again.",
        )
    }

    pub fn tenant_move_schema_differs(placement: &str, pending: usize) -> Error {
        Error::conflict(
            "tenant_move_schema_differs",
            "The placement `{placement}` has {pending} migration(s) not applied: run \
             `autoxide migrate up --all` so both databases have the same schema.",
        )
        .with_param("placement", placement)
        .with_param("pending", pending)
    }

    pub fn tenant_move_same_database(placement: &str) -> Error {
        Error::conflict(
            "tenant_move_same_database",
            "The placement `{placement}` is the same database as the tenant's current one; moving \
             would delete the tenant's data. Give each placement its own database.",
        )
        .with_param("placement", placement)
    }

    pub fn tenant_move_unreachable(placement: &str, error: &str) -> Error {
        Error::unavailable(
            "tenant_move_unreachable",
            "Cannot reach the database of the placement `{placement}`: {error}",
        )
        .with_param("placement", placement)
        .with_param("error", error)
    }

    pub fn tenant_move_storage_unavailable(placement: &str, error: &str) -> Error {
        Error::unavailable(
            "tenant_move_storage_unavailable",
            "Cannot reach the file storage of the placement `{placement}`: {error}",
        )
        .with_param("placement", placement)
        .with_param("error", error)
    }

    pub fn tenant_move_busy(jobs: u64) -> Error {
        Error::unavailable(
            "tenant_move_busy",
            "{jobs} job(s) or event(s) of the tenant are still being run at the old placement. \
             The move is paused; run it again in a few minutes.",
        )
        .with_param("jobs", jobs)
    }

    pub fn tenant_move_count_mismatch(table: &str, source: u64, target: u64) -> Error {
        Error::conflict(
            "tenant_move_count_mismatch",
            "After copying, `{table}` has {target} row(s) of the tenant at the new placement and \
             {source} at the old one; the tenant keeps its old placement. Run the move again to \
             copy once more.",
        )
        .with_param("table", table)
        .with_param("source", source)
        .with_param("target", target)
    }

    pub fn tenant_move_not_found(id: &str) -> Error {
        Error::not_found("tenant_move_not_found", "The tenant has no move `{id}`.")
            .with_param("id", id)
    }

    pub fn tenant_purge_current(placement: &str) -> Error {
        Error::conflict(
            "tenant_purge_current",
            "The placement `{placement}` keeps the tenant's live data; only a copy left on \
             another placement (by `--keep-source`) can be deleted.",
        )
        .with_param("placement", placement)
    }
}

/// The default tenant on a database that connects nowhere: for kernel unit tests that run no queries.
#[cfg(test)]
pub(crate) fn test_tenant() -> TenantContext {
    TenantContext::default_on(Database::detached())
}

/// Checks whether `tenant` can be served: suspended → `tenant_suspended`; maintenance / being moved / being
/// provisioned, or a placement not fully migrated → `tenant_maintenance`.
pub fn ensure_serving(tenant: &TenantContext) -> Result<(), Error> {
    if !tenant.schema_ready() && tenant.status() != TenantStatus::Suspended {
        return Err(errors::tenant_maintenance(30));
    }
    match tenant.status() {
        TenantStatus::Active => Ok(()),
        TenantStatus::Suspended => Err(errors::tenant_suspended()),
        TenantStatus::Maintenance | TenantStatus::Moving | TenantStatus::Provisioning => {
            Err(errors::tenant_maintenance(30))
        }
        TenantStatus::Deleted | TenantStatus::Merged => {
            Err(errors::tenant_not_found(&tenant.id().to_string()))
        }
    }
}

/// Marks a route that can still write when the tenant is read-only: login, refresh / end session, password
/// reset, links in email (confirm, unsubscribe: withdrawing consent must always work), data export. Attach
/// it with [`allowed_while_read_only`] on the sub-router that holds exactly those routes.
#[derive(Clone, Copy, Debug, Default)]
pub struct AllowedWhileReadOnly;

/// Layer that attaches [`AllowedWhileReadOnly`] to the routes added to the router before it:
/// `OpenApiRouter::new().routes(routes!(login)).layer(allowed_while_read_only())`.
pub fn allowed_while_read_only() -> axum::Extension<AllowedWhileReadOnly> {
    axum::Extension(AllowedWhileReadOnly)
}

/// Checks whether a request (`method`, `extensions`) is allowed for `tenant`: a read-only tenant accepts
/// only `GET`, `HEAD`, `OPTIONS`, and routes marked [`AllowedWhileReadOnly`]; anything else gets
/// `tenant_read_only`. The kernel calls it where the request's tenant is resolved (extractors
/// `TenantContext`, `OptionalTenant`); MCP tools check by the tool's `read_only` flag.
pub fn ensure_writable(
    tenant: &TenantContext,
    method: &axum::http::Method,
    extensions: &axum::http::Extensions,
) -> Result<(), Error> {
    use axum::http::Method;
    if !tenant.read_only()
        || [Method::GET, Method::HEAD, Method::OPTIONS].contains(method)
        || extensions.get::<AllowedWhileReadOnly>().is_some()
    {
        return Ok(());
    }
    Err(errors::tenant_read_only())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_co_dinh_dung_dang_uuid_v7() {
        assert_eq!(DEFAULT_TENANT.to_string(), "00000000-0000-7000-8000-000000000001");
        assert_eq!(DEFAULT_TENANT.as_uuid().get_version_num(), 7);
        assert_eq!(MAIN_PLACEMENT.as_uuid().get_version_num(), 7);
        assert!(NO_TENANT.as_uuid().is_nil());
    }

    #[test]
    fn trang_thai_doc_ghi_du() {
        for status in [
            TenantStatus::Provisioning,
            TenantStatus::Active,
            TenantStatus::Suspended,
            TenantStatus::Maintenance,
            TenantStatus::Moving,
        ] {
            assert_eq!(TenantStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(TenantStatus::parse("gone"), None);
        for kind in [PlacementKind::Shared, PlacementKind::Dedicated] {
            assert_eq!(PlacementKind::parse(kind.as_str()), Some(kind));
        }
    }
}
