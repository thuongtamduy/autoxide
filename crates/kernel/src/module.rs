//! The module contract: what a module declares to the core, and [`Modules`] - the set of modules registered
//! in the program.
//!
//! A module is a crate in `modules/<name>`. It exposes exactly two things (`cargo xtask check` verifies
//! this):
//!
//! - `pub fn module() -> impl Module` - for the binary to register it;
//! - `pub mod api` - traits and data types for other modules to use.
//!
//! The binary registers every module in ONE place (`crates/app/src/modules.rs`). The core does not know any
//! module name: it works only through the [`Module`] trait, so adding a module does not require changing
//! the core.
//!
//! A module declares itself in two places, one way for each concept (D-20261126-module-manifest):
//!
//! - [`Module::manifest`] returns a [`Manifest`]: everything known before the module starts (name,
//!   migrations, permissions, events, settings, translations, quotas, dependencies, tier…);
//! - [`Module::start`] builds the module's services and registers on [`StartContext`] what needs them: routes,
//!   MCP tools, subscriptions, jobs, traits for other modules.
//!
//! The contract grows by adding builder functions to [`Manifest`] and registration functions to
//! [`StartContext`], so existing modules keep compiling. The other functions of [`Module`] are the old way of
//! declaring a module, kept until every module has moved to the manifest; a module that implements
//! [`Module::manifest`] implements none of them.

pub use crate::ops::client_config::{ClientSection, TenantClientSection};
use jiff::SignedDuration;
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::migration::MigrationTrait;
use utoipa_axum::router::OpenApiRouter;

use crate::access::{Permission, PermissionCatalog};
use crate::config::{ConfigReader, Invalid};
use crate::database::Database;
use crate::error::Error;
use crate::event::{EventCatalog, EventInfo, Outbox, Subscription};
use crate::i18n::Translations;
use crate::job::{JobQueue, JobRegistration, Schedule};
use crate::privacy::{PersonalData, PersonalDataExport};

mod apis;
mod contribute;
mod dormant;
mod later;
mod manifest;
pub mod navigation;
mod registry;
mod started;

mod selection;

use apis::Apis;
pub use dormant::DormantContext;
pub(crate) use dormant::installed;
pub use later::Later;
pub use manifest::{Manifest, MigrationsFn, UnstableManifest, UpgradesFn};
pub use registry::{ModuleError, Modules};
pub use selection::{DormantModule, InactiveModule, MODULES, Selection, SelectionError, Stability};
pub use started::KernelHandle;

/// What a module declares to the core.
///
/// ```
/// use autoxide_kernel::config::{ConfigReader, Invalid, Setting};
/// use autoxide_kernel::module::{Manifest, Module};
///
/// const PAGE_SIZE: Setting =
///     Setting::new("AUTOXIDE_NOTES_PAGE_SIZE", "Notes returned per page.").with_default("50");
///
/// #[derive(Default)]
/// struct NotesModule {
///     page_size: u32,
/// }
///
/// impl Module for NotesModule {
///     fn manifest(&self) -> Manifest {
///         Manifest::new("notes", "Personal notes.")
///     }
///
///     fn configure(&mut self, reader: &mut ConfigReader) -> Result<(), Invalid> {
///         self.page_size = reader.get(&PAGE_SIZE)?;
///         Ok(())
///     }
/// }
///
/// // The only function the module crate exposes to the binary.
/// pub fn module() -> impl Module {
///     NotesModule::default()
/// }
/// ```
//
// `Send + Sync`: module được dùng từ nhiều thread của máy chủ. Trình biên dịch
// từ chối module chứa thứ không an toàn khi chia sẻ giữa các thread.
pub trait Module: Send + Sync {
    /// Everything the module declares before it starts (see [`Manifest`]). The default is the old way: the
    /// module answers through the other functions of this trait instead.
    fn manifest(&self) -> Manifest {
        Manifest::legacy()
    }

    /// The module name: lowercase letters, digits and `_`, starting with a letter. It is also the prefix of
    /// every table (`<name>_…`) and every configuration variable (`AUTOXIDE_<NAME>_…`) of the module.
    ///
    /// Old way: use [`Manifest::new(name, description)`](Manifest::new) instead.
    fn name(&self) -> &'static str {
        self.manifest().name
    }

    /// The module's migrations, in the order they are applied. Once committed they are never edited - add a
    /// new migration at the end.
    ///
    /// Old way: use [`Manifest::migrations`](Manifest::migrations) instead.
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        self.manifest().migrations.map(|build| build()).unwrap_or_default()
    }

    /// A module's data upgrades (D-257): data upgrades that need the program's configuration (e.g.
    /// re-encrypting old secrets). Empty by default. See `autoxide_kernel::migration::DataUpgrade`.
    ///
    /// Old way: use [`Manifest::upgrades`](Manifest::upgrades) instead.
    fn upgrades(&self) -> Vec<Box<dyn crate::migration::DataUpgrade>> {
        self.manifest().upgrades.map(|build| build()).unwrap_or_default()
    }

    /// Configuration the module accepts per tenant (D-258): named `<module>.<key>`, read with
    /// `context.require::<dyn TenantSettings>()` (`autoxide_kernel::tenancy`). A tenant that has not set
    /// one falls back to the shared configuration. Connections to partners (SMTP server, payment
    /// gateway...) declare `.group(CONNECTION_GROUP)` and are named `<module>.connection.<kind>.<field>`;
    /// when renaming a setting, declare the old name with `.formerly(…)`. Empty by default.
    ///
    /// Old way: use [`Manifest::tenant_settings`](Manifest::tenant_settings) instead.
    fn tenant_settings(&self) -> Vec<crate::tenancy::TenantSetting> {
        self.manifest().tenant_settings
    }

    /// The module's settings inherited down the organization tree (step 34): named `<module>.<name>`, with
    /// defaults and a validator. The `organizations` module stores and serves the value per node; a node
    /// that sets none uses the value of the nearest ancestor. Empty by default.
    ///
    /// Old way: use [`Manifest::org_settings`](Manifest::org_settings) instead.
    fn org_settings(&self) -> Vec<crate::tenancy::OrgSetting> {
        self.manifest().org_settings
    }

    /// Seeds the module's initial data for a new tenant (e.g. the `admin` role, lock rows) - called after
    /// `start`, when a tenant is created (`autoxide tenant create`, `POST /api/v1/kernel/tenants`). Must be
    /// re-runnable. Seeds nothing by default.
    ///
    /// Old way: use `context.contribute::<dyn TenantSeed>(…)` ([`StartContext::contribute`]) in `start` instead.
    fn seed_tenant(&self) -> Option<std::sync::Arc<dyn crate::tenancy::TenantSeed>> {
        None
    }

    /// The module's sample data for new tenants provisioned under a plan with `sample_data` (step 35): a
    /// few records so the tenant can see how the module is used. Runs only for modules the plan enables.
    /// Must be re-runnable and must not create duplicates. None by default.
    ///
    /// Old way: use `context.contribute::<dyn SampleData>(…)` ([`StartContext::contribute`]) in `start` instead.
    fn sample_data(&self) -> Option<std::sync::Arc<dyn crate::tenancy::TenantSeed>> {
        None
    }

    /// The module's tenant-level tables that must be KEPT when a tenant is permanently deleted (D-337):
    /// vouchers, accounting books, invoices kept for at least 10 years (Accounting Law 88/2015, Article 41;
    /// D-190). Empty by default: the module's data is deleted along with the tenant.
    ///
    /// Old way: use [`Manifest::retained_tables`](Manifest::retained_tables) instead.
    fn retained_tables(&self) -> Vec<&'static str> {
        self.manifest().retained_tables
    }

    /// The kinds of personal data the module holds and the retention period of each kind (step 37, D-360).
    /// Called after `configure` (retention periods are read from the module's configuration); the kernel
    /// collects them into a [`RetentionCatalog`](crate::privacy::RetentionCatalog), which `compliance`
    /// publishes in the processing activity register. Empty by default.
    ///
    /// Old way: use [`Manifest::retention`](Manifest::retention) instead.
    fn retention(&self) -> Vec<crate::privacy::RetentionPolicy> {
        self.manifest().retention
    }

    /// The module's control-plane tables (step 33, D-253): tables shared across the whole platform that live
    /// only in the control database. Each must be one of the module's own tables (`<module>_…`), or the
    /// modules do not build. A control-plane table may still carry `tenant_id` when it keeps platform data
    /// about each tenant (subscriptions, domains), but a `scope = global` record never maps such a table.
    /// Every other table of the module (except `<m>_migrations`) is a tenant-level table and must have
    /// `tenant_id` - the testkit and `cargo xtask check` verify this. Empty by default: business modules
    /// have almost no tables of this kind.
    ///
    /// Old way: use [`Manifest::control_plane_tables`](Manifest::control_plane_tables) instead.
    fn control_plane_tables(&self) -> Vec<&'static str> {
        self.manifest().control_plane_tables
    }

    /// The module's plan-based on / off features (`<module>.<name>`, step 35). The module asks with
    /// [`EntitlementsHandle::require_feature`](crate::entitlement::EntitlementsHandle) before work that
    /// needs the feature. Empty by default.
    ///
    /// Old way: use [`Manifest::features`](Manifest::features) instead.
    fn features(&self) -> Vec<crate::entitlement::Feature> {
        self.manifest().features
    }

    /// The module's plan-based quotas (`<module>.<name>`, step 35): the module reserves / releases through
    /// [`EntitlementsHandle`](crate::entitlement::EntitlementsHandle). Empty by default.
    ///
    /// Old way: use [`Manifest::limits`](Manifest::limits) instead.
    fn limits(&self) -> Vec<crate::entitlement::Limit> {
        self.manifest().limits
    }

    /// A foundation module that a plan never blocks (login, files, audit...). Default `false`: the module
    /// is governed by plans - if not yet enabled for a tenant, its routes, MCP tools, jobs and events are
    /// blocked.
    ///
    /// Old way: use [`Manifest::always_enabled`](Manifest::always_enabled) instead.
    fn always_enabled(&self) -> bool {
        self.manifest().always_enabled
    }

    /// The module's tier (step 70 P3): `Official` (default) runs when the installer chooses nothing;
    /// `Experimental` - API and tables may still change - runs only when explicitly enabled in
    /// `AUTOXIDE_MODULES`. An official module may depend only on official modules.
    ///
    /// Old way: use [`Manifest::stability`](Manifest::stability) instead.
    fn stability(&self) -> Stability {
        self.manifest().stability
    }

    /// The core cannot run without this module (login, idempotency...): always enabled, `-name` in
    /// `AUTOXIDE_MODULES` is rejected. So are the modules it [`depends_on`](Self::depends_on). Default
    /// `false`. Different from [`always_enabled`](Self::always_enabled) - that one is about each tenant's
    /// PLAN, this one is about the whole installation.
    ///
    /// Old way: use [`Manifest::essential`](Manifest::essential) instead.
    fn essential(&self) -> bool {
        self.manifest().essential
    }

    /// The records (`#[derive(Record)]`) of the module, as `RecordInfo::of::<Note>()`: at startup the kernel
    /// checks each against the live schema (table, columns, `tenant_id` for a tenant record) and refuses to
    /// start on a mismatch.
    ///
    /// Old way: use [`Manifest::records`](Manifest::records) instead.
    fn records(&self) -> Vec<crate::db::RecordInfo> {
        self.manifest().records
    }

    /// The names of modules that this module MUST have: every `context.require::<…>()?` to a trait of
    /// another module (and every event of another module that it listens to) is a name here. Enabling this
    /// module enables those modules with it. Things used "if present" (`.require::<…>().ok()`) are NOT
    /// declared here. Empty by default.
    ///
    /// Old way: use [`Manifest::depends_on`](Manifest::depends_on) instead.
    fn depends_on(&self) -> Vec<&'static str> {
        self.manifest().depends_on
    }

    /// Reads the module's own configuration and keeps it inside itself. Called once at startup, before
    /// anything else. Errors are recorded in `reader` and reported together, in one pass, with the kernel's
    /// and other modules' configuration errors.
    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        // `let _ =`: tham số không dùng tới trong cài đặt mặc định.
        let _ = reader;
        Ok(())
    }

    /// Starts the module: builds its service (with the database taken from `context`), publishes traits for
    /// other modules to use ([`StartContext::provide`]) and fetches traits of other modules
    /// ([`StartContext::require`]). Called once, after `configure`, in registration order - a module must
    /// be registered AFTER the modules it needs.
    ///
    /// The module keeps what it built inside itself for [`routes`](Self::routes) to use.
    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let _ = context;
        Ok(())
    }

    /// The module's permissions. Names are written `<module name>.<action>`; the core checks them at
    /// startup. A released permission is a contract: renaming it makes roles that hold the old permission
    /// lose access.
    ///
    /// Old way: use [`Manifest::permissions`](Manifest::permissions) instead.
    fn permissions(&self) -> Vec<Permission> {
        self.manifest().permissions
    }

    /// The events the module emits. With events the module must have a `<name>_events` table (add
    /// `autoxide_kernel::event::outbox_migration` to its migrations) and record events with
    /// [`StartContext::outbox`].
    ///
    /// Old way: use [`Manifest::events`](Manifest::events) instead.
    fn events(&self) -> Vec<EventInfo> {
        self.manifest().events
    }

    /// Lifecycle events of the object kinds the module owns (D-202): which event means an object no longer
    /// accepts new interaction or was removed, so a module that attaches data to objects (e.g. `comments`)
    /// can close / hide its own part without knowing the owning module. The event must be in the module's
    /// own [`events`](Self::events), the kind must be `<module name>.<kind>`, and the event's subject is
    /// `<kind>:<id>`. The kernel checks this at startup.
    ///
    /// Old way: use [`Manifest::resource_events`](Manifest::resource_events) instead.
    fn resource_events(&self) -> Vec<crate::resource::ResourceEvent> {
        self.manifest().resource_events
    }

    /// The events the module wants to receive (of every module, including itself). Called after `start`.
    ///
    /// Old way: use [`StartContext::subscribe`](StartContext::subscribe) (one call per subscription) instead.
    fn subscriptions(&self) -> Vec<Subscription> {
        Vec::new()
    }

    /// Dormant mode (step 70 P9, D-20261125-dormant-modules): called INSTEAD of `start` when the module is
    /// in the program but turned off (`AUTOXIDE_MODULES`), after every enabled module has started. A module
    /// that holds personal data registers here the same export part and account-deletion handler it
    /// registers when enabled, so a turned-off module's data is never left out of a person's export or
    /// deletion. Nothing else of the module runs. Does nothing by default.
    fn dormant(&self, context: &mut DormantContext<'_>) -> Result<(), Error> {
        let _ = context;
        Ok(())
    }

    /// Runners for the module's job kinds (`<name>.<action>`). With jobs the module must have a
    /// `<name>_jobs` table (add `autoxide_kernel::job::jobs_migration` to its migrations). Called after
    /// `start`.
    ///
    /// Old way: use [`StartContext::job`](StartContext::job) (one call each) instead.
    fn jobs(&self) -> Vec<JobRegistration> {
        Vec::new()
    }

    /// The module's periodic schedules: each schedule puts a job (of a kind the module itself runs) on the
    /// queue after every interval. Called after `start`.
    ///
    /// Old way: use [`Manifest::schedules`](Manifest::schedules) instead.
    fn schedules(&self) -> Vec<Schedule> {
        self.manifest().schedules
    }

    /// The module's deprecated endpoints: the kernel marks them in OpenAPI and attaches the `Deprecation` /
    /// `Sunset` / `Link` headers to responses (D-059).
    ///
    /// Old way: use [`Manifest::deprecations`](Manifest::deprecations) instead.
    fn deprecations(&self) -> Vec<crate::http::Deprecation> {
        self.manifest().deprecations
    }

    /// The module's MCP tools (D-068). Called after `start` (tools usually call into the service built
    /// then). Names are declared short (`create_note`); the kernel adds the `<module>.` prefix.
    ///
    /// Old way: use [`StartContext::tools`](StartContext::tools) instead.
    fn tools(&self) -> Vec<crate::mcp::Tool> {
        Vec::new()
    }

    /// The module's MCP resources (step 40 P2): read-only documents for agents - guides, the JSON Schema of
    /// a record kind. Called after `start`. The path is declared short (`guide`); the kernel assembles the
    /// URI `autoxide://<module>/<path>`. Default: none.
    ///
    /// Old way: use [`StartContext::mcp_resource`](StartContext::mcp_resource) (one call each) instead.
    fn mcp_resources(&self) -> Vec<crate::mcp::McpResource> {
        Vec::new()
    }

    /// The module's MCP prompts (step 40 P2): prompt templates for common tasks. Called after `start`.
    /// Names are declared short (`draft_post`); the kernel adds the `<module>.` prefix. Default: none.
    ///
    /// Old way: use [`StartContext::prompt`](StartContext::prompt) (one call each) instead.
    fn prompts(&self) -> Vec<crate::mcp::Prompt> {
        Vec::new()
    }

    /// One English sentence saying what the module does, for `/llms.txt` and the module registry (step 40
    /// P2). Default: empty.
    ///
    /// Old way: use [`Manifest::new(name, description)`](Manifest::new) instead.
    fn description(&self) -> &'static str {
        self.manifest().description
    }

    /// Installation checks for `autoxide doctor` (D-166): whether storage is writable, whether an external
    /// server responds, whether initial data is missing... Called after `start`. Names are declared short
    /// (`storage`); the kernel adds the `<module>.` prefix. A check is read-only, or cleans up whatever it
    /// wrote as a probe.
    ///
    /// Old way: use [`StartContext::check`](StartContext::check) (one call each) instead.
    fn checks(&self) -> Vec<crate::doctor::Check> {
        Vec::new()
    }

    /// The module's sub-area groups and menu items for the UI (step 51 P1). The kernel collects them at
    /// startup (duplicate groups must be declared identically) and filters by tenant, plan and permission
    /// at `GET /api/v1/kernel/navigation`. Default: none.
    ///
    /// Old way: use [`Manifest::navigation`](Manifest::navigation) instead.
    fn navigation(&self) -> navigation::Navigation {
        self.manifest().navigation
    }

    /// The module's reason lists (step 51 P18, D-223): named `<module>.<action>`, with default codes and
    /// English labels. A tenant can rename labels, enable / disable them, and add its own codes via
    /// `/api/v1/kernel/reasons`. Default: none.
    ///
    /// Old way: use [`Manifest::reason_lists`](Manifest::reason_lists) instead.
    fn reason_lists(&self) -> Vec<crate::ops::reasons::ReasonList> {
        self.manifest().reason_lists
    }

    /// Tenant-customizable texts (step 51 P21, D-235): terminology and end-user-facing sentences, keyed
    /// `<module>.<…>`, with a default English sentence. Error codes and error messages can never be
    /// customized. A tenant customizes them via `/api/v1/kernel/texts`; modules read them with
    /// `TextOverrides::text`. Default: none.
    ///
    /// Old way: use [`Manifest::overridable_texts`](Manifest::overridable_texts) instead.
    fn overridable_texts(&self) -> Vec<crate::ops::texts::OverridableText> {
        self.manifest().overridable_texts
    }

    /// The module's record kinds that accept custom fields (step 51 P19, D-235). The module stores the
    /// values on the record (the `custom_fields` column) and validates with `CustomFields::validate`.
    /// Default: none.
    ///
    /// Old way: use [`UnstableManifest::custom_field_targets`](UnstableManifest::custom_field_targets) (inside [`Manifest::unstable`](Manifest::unstable)) instead.
    fn custom_field_targets(&self) -> Vec<crate::ops::custom_fields::CustomFieldTarget> {
        self.manifest().unstable.custom_field_targets
    }

    /// The module's admin commands for `autoxide task` (D-169): work an operator runs by hand, e.g.
    /// rebuilding derived data after a configuration change. Called after `start`. Names are declared short
    /// (`refresh_variants`); the kernel adds the `<module>.` prefix.
    ///
    /// Old way: use [`StartContext::task`](StartContext::task) (one call each) instead.
    fn tasks(&self) -> Vec<crate::task::Task> {
        Vec::new()
    }

    /// Routes that need a request body larger than the shared limit (`AUTOXIDE_HTTP_BODY_LIMIT_BYTES`),
    /// e.g. file uploads. Called after `configure` (the limit may come from the module's configuration).
    ///
    /// Old way: use [`Manifest::body_limits`](Manifest::body_limits) instead.
    fn body_limits(&self) -> Vec<crate::http::BodyLimit> {
        self.manifest().body_limits
    }

    /// The module's error message translations: `(language, JSON content)`, usually
    /// `vec![("vi", include_str!("../locales/vi.json"))]`. Every error `code` of the module must have a
    /// translation (`cargo xtask check` verifies this).
    ///
    /// Old way: use [`Manifest::translation`](Manifest::translation) (one call per language) instead.
    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        self.manifest().translations
    }

    /// Non-secret configuration values the UI needs (language, upload limits...), served at
    /// `GET /api/v1/kernel/client-config` under the module name (D-271). The endpoint needs no sign-in:
    /// only put values that anyone may see. Called once after the `start` of every module.
    ///
    /// Old way: use [`StartContext::client_config`](StartContext::client_config) instead.
    fn client_config(&self) -> Result<Option<ClientSection>, Error> {
        Ok(None)
    }

    /// The module's REST routes, with their OpenAPI documentation. Called after `start`. The core mounts
    /// them under `/api/v1/<module name>`: the routes declared as `/` and `/{id}` of module `notes` become
    /// `/api/v1/notes` and `/api/v1/notes/{id}`.
    ///
    /// Old way: use [`StartContext::routes`](StartContext::routes) instead.
    fn routes(&self) -> OpenApiRouter {
        OpenApiRouter::new()
    }

    /// The module's REST endpoints (functions with `#[endpoint]`, listed in [`routes!`](crate::routes)), mounted
    /// like [`routes`](Self::routes) under `/api/v1/<module name>`. Called after `start`. A module uses this or
    /// `routes`; the module manifest replaces both.
    ///
    /// Old way: use [`StartContext::routes`](StartContext::routes) instead.
    fn endpoints(&self) -> crate::http::Routes {
        crate::http::Routes::new()
    }

    /// Report figures that the module's events produce (step 38, D-20261118-step38-report-facts): each
    /// [`FactSource`](crate::report::FactSource) says which figure one event of the module ITSELF
    /// (`<module>.…`, declared in [`events`](Self::events)) produces. The `reporting` module listens to
    /// those events and pours the figures into the aggregated report store of the tenant tree. The kernel
    /// checks the declarations at startup. Default: none.
    ///
    /// Old way: use [`UnstableManifest::facts`](UnstableManifest::facts) (inside [`Manifest::unstable`](Manifest::unstable)) instead.
    fn facts(&self) -> Vec<crate::report::FactSource> {
        self.manifest().unstable.facts
    }

    /// How each tenant-level table of the module travels when an organization branch is split off into its
    /// own tenant or two tenants are merged (step 38, D-20261118-step38-relocation): by node, by parent
    /// row, copied by the row pointing to it, the whole tenant, must be empty, or the module handles it
    /// itself ([`TablePolicy`](crate::tenancy::TablePolicy)); plus the tenant-encrypted secret columns.
    /// Default: inferred from columns (has `org_id` -> by node; a queue -> must be empty; otherwise ->
    /// stays, with a warning in the plan) - the binary's test blocks undeclared tables.
    ///
    /// Old way: use [`Manifest::relocation`](Manifest::relocation) instead.
    fn relocation(&self) -> crate::tenancy::Relocation {
        self.manifest().relocation
    }

    /// The module's web analytics event kinds (step 57, D-220): the website sends kind `browser`, kind
    /// `server` is derived from the module's own events. The `analytics` module accepts only declared
    /// kinds. The kernel checks the declarations at startup. Default: none.
    ///
    /// Old way: use [`UnstableManifest::analytics_events`](UnstableManifest::analytics_events) (inside [`Manifest::unstable`](Manifest::unstable)) instead.
    fn analytics_events(&self) -> Vec<crate::analytics::AnalyticsEvent> {
        self.manifest().unstable.analytics_events
    }

    /// Funnels the module declares (step 57), e.g. `orders.checkout`: the steps are declared event kinds
    /// (of any module). Default: none.
    ///
    /// Old way: use [`UnstableManifest::funnels`](UnstableManifest::funnels) (inside [`Manifest::unstable`](Manifest::unstable)) instead.
    fn funnels(&self) -> Vec<crate::analytics::Funnel> {
        self.manifest().unstable.funnels
    }

    /// The module's built-in automation rule templates (step 58): id `<module>.<name>`, rule body in the
    /// format of the `automations` module. Called after `start`. Default: none.
    ///
    /// Old way: use [`UnstableManifest::automation_templates`](UnstableManifest::automation_templates) (inside [`Manifest::unstable`](Manifest::unstable)) instead.
    fn automation_templates(&self) -> Vec<crate::automation::AutomationTemplate> {
        self.manifest().unstable.automation_templates
    }

    /// Copies the module's configuration into a new sandbox (step 38, D-232): called by the
    /// `kernel.sandbox_copy` job in registration order. Configuration only, never personal data,
    /// transactions or secrets. Called after `start`. Default: copies nothing.
    ///
    /// Old way: use `context.contribute::<dyn SandboxCopy>(…)` ([`StartContext::contribute`]) in `start` instead.
    fn sandbox_copy(&self) -> Option<Arc<dyn crate::tenancy::sandbox::SandboxCopy>> {
        None
    }

    /// The module's record kinds that can be pushed from a sandbox to the real tenant as drafts (step 38,
    /// D-232): `kind` is `<module>.<kind>`. Called after `start`. Default: none.
    ///
    /// Old way: use [`UnstableManifest::promotable`](UnstableManifest::promotable) (inside [`Manifest::unstable`](Manifest::unstable)) instead.
    fn promotable(&self) -> Vec<crate::tenancy::sandbox::Promotable> {
        self.manifest().unstable.promotable
    }

    /// Record kinds an offline app can sync (step 69 P2, D-20261119-step69-sync): a module that has them
    /// adds `sync::changes_migration` to its migrations, records changes with `StartContext::sync_log` in
    /// the write transaction, and serves `GET <path>/changes`. The kernel checks the declarations at
    /// startup. Default: none.
    ///
    /// Old way: use [`UnstableManifest::sync_resources`](UnstableManifest::sync_resources) (inside [`Manifest::unstable`](Manifest::unstable)) instead.
    fn sync_resources(&self) -> Vec<crate::sync::SyncResource> {
        self.manifest().unstable.sync_resources
    }

    /// Blocks the module provides to block-built pages (step 67, [`crate::blocks`]). A block that needs
    /// data attaches a [`BlockData`](crate::blocks::BlockData) at `start` with
    /// [`StartContext::block_data`]. Default: none.
    ///
    /// Old way: use [`UnstableManifest::blocks`](UnstableManifest::blocks) (inside [`Manifest::unstable`](Manifest::unstable)) instead.
    fn blocks(&self) -> Vec<crate::blocks::BlockType> {
        self.manifest().unstable.blocks
    }
}

/// What a module receives at startup.
pub struct StartContext<'a> {
    module: &'static str,
    /// The module implements [`Module::manifest`]: only then may it register in `start` (`started.rs`).
    declared: bool,
    /// What the module registered in `start` (routes, tools, subscriptions…), see `started.rs`.
    registered: started::Registered,
    database: &'a Database,
    permissions: &'a PermissionCatalog,
    events: &'a EventCatalog,
    translations: &'a Translations,
    wake: &'a Arc<tokio::sync::Notify>,
    apis: &'a mut Apis,
    personal_data: &'a PersonalDataExport,
    retention: &'a crate::privacy::RetentionCatalog,
    resources: &'a crate::resource::ResourceDirectory,
    work: &'a crate::ops::work::WorkSources,
    finders: &'a crate::ops::find::FinderSources,
    suggestions: &'a crate::ops::suggest::SuggestSources,
    metrics: &'a crate::ops::summary::SummaryMetrics,
    exports: &'a crate::ops::list_exports::ExportSources,
    bulk: &'a crate::ops::bulk::BulkActions,
    resolvers: &'a crate::ops::resolve::CodeResolvers,
    setup: &'a crate::ops::setup::SetupSteps,
    reasons: &'a crate::ops::reasons::ReasonCodes,
    probes: &'a crate::ops::connections::ConnectionProbes,
    taggables: &'a crate::ops::tags::Taggables,
    texts: &'a crate::ops::texts::TextOverrides,
    custom_fields: &'a crate::ops::custom_fields::CustomFields,
    references: &'a crate::ops::usages::ReferenceSources,
    resource_events: &'a [crate::resource::ResourceEvent],
    verification: &'a crate::access::PostingVerification,
    tenancy: &'a crate::tenancy::DatabaseRegistry,
    org_settings: &'a crate::tenancy::OrgSettingCatalog,
    entitlements: &'a crate::entitlement::EntitlementsHandle,
    counters:
        &'a mut std::collections::BTreeMap<&'static str, Arc<dyn crate::entitlement::UsageCounter>>,
    facts: &'a crate::report::FactCatalog,
    analytics: &'a crate::analytics::AnalyticsCatalog,
    blocks: &'a crate::blocks::Blocks,
    automations: &'a crate::automation::AutomationDirectory,
    sync_keep: SignedDuration,
    /// Wraps the partner contracts so the sandbox always uses the fake implementation (step 38, D-232).
    partners: &'a crate::tenancy::sandbox::PartnerGuard,
    /// The registry for sandbox copy / push of every module, filled in after every module has started.
    sandbox: &'a crate::tenancy::sandbox::SandboxCatalog,
}

impl StartContext<'_> {
    /// The control-plane handles of this module (`crate::db`): global records only; sweeping every tenant
    /// is `all_tenants("reason")`. Tenant data goes through `tenant.db()`.
    pub fn control_plane(&self) -> crate::db::ControlPlane {
        crate::db::ControlPlane::new(self.database.clone(), self.module)
    }

    /// The database registry (step 33): the installation's mode and each tenant's database. A service gets
    /// the database for tenant-level data through [`TenantContext`](crate::tenancy::TenantContext), not
    /// through [`database`](Self::database).
    pub fn tenancy(&self) -> crate::tenancy::DatabaseRegistry {
        self.tenancy.clone()
    }

    /// The per-node settings that every module has declared (`Module::org_settings`), for modules to store
    /// values.
    pub fn org_settings(&self) -> crate::tenancy::OrgSettingCatalog {
        self.org_settings.clone()
    }

    /// Per-tenant entitlements (step 35): ask whether a module / feature is enabled, reserve quota.
    /// Available from the first module's start; before `plans` (or another provider) starts it means "no
    /// limit".
    pub fn entitlements(&self) -> crate::entitlement::EntitlementsHandle {
        self.entitlements.clone()
    }

    /// Registers a real counter for the quota `limit` of this module (the reconciliation job fixes any
    /// drift).
    pub fn usage_counter(
        &mut self,
        limit: &crate::entitlement::Limit,
        counter: Arc<dyn crate::entitlement::UsageCounter>,
    ) -> Result<(), Error> {
        let owned = self
            .entitlements
            .catalog()
            .limit(limit.key())
            .is_some_and(|found| found.module == self.module);
        if !owned {
            return Err(Error::internal(format!(
                "module `{}` registers a usage counter for `{}`, which it does not declare in Module::limits",
                self.module,
                limit.key()
            )));
        }
        if self.counters.insert(limit.key(), counter).is_some() {
            return Err(Error::internal(format!(
                "usage counter for `{}` registered twice",
                limit.key()
            )));
        }
        Ok(())
    }

    /// The program's database (the control database: a read pool and a write pool). For things that are
    /// truly installation-wide (e.g. the root signing key); tenant-level data goes through `TenantContext`
    /// (step 33).
    pub fn database(&self) -> &Database {
        self.database
    }

    /// Every permission of every registered module (e.g. to check that a role is only granted permissions
    /// that exist).
    pub fn permissions(&self) -> &PermissionCatalog {
        self.permissions
    }

    /// The translations of the kernel and every module (e.g. to fill in mail templates per language).
    pub fn translations(&self) -> &Translations {
        self.translations
    }

    /// Every event kind of every registered module (e.g. to validate event names a client sends).
    pub fn events(&self) -> &EventCatalog {
        self.events
    }

    /// The module's job queue (the `<name>_jobs` table).
    pub fn jobs(&self) -> JobQueue {
        JobQueue::new(self.module)
    }

    /// Where the module records its events. An error if the module declares no events (`Module::events`).
    pub fn outbox(&self) -> Result<Outbox, Error> {
        if !self.events.modules().contains(&self.module) {
            return Err(Error::internal(format!(
                "module `{}` asks for an outbox but declares no events in `Module::events`",
                self.module
            )));
        }
        Ok(Outbox::with_wake(self.module, Arc::clone(self.wake)))
    }

    /// Where the module writes and reads the change stream for offline apps (step 69 P2): the table
    /// `<module>_changes`, with retention per `AUTOXIDE_SYNC_RETENTION_DAYS`.
    pub fn sync_log(&self) -> crate::sync::SyncLog {
        crate::sync::SyncLog::new(self.module, self.sync_keep)
    }

    /// Registers the part of personal data the module holds, so it is included in a user's data export (see
    /// [`crate::privacy`]). Each module registers at most once.
    pub fn personal_data(&mut self, exporter: Arc<dyn PersonalData>) -> Result<(), Error> {
        self.personal_data.register(self.module, exporter)
    }

    /// The registry of every module's personal data, for the account-management module to export data. The
    /// registry includes parts registered by modules that start later.
    pub fn personal_data_export(&self) -> PersonalDataExport {
        self.personal_data.clone()
    }

    /// The registry of retention periods every module declares (`Module::retention`), complete even for
    /// modules registered later.
    pub fn retention_catalog(&self) -> crate::privacy::RetentionCatalog {
        self.retention.clone()
    }

    /// Registers an object kind `kind` (`<module>.<kind>`) of this module, with its lookup (see
    /// [`crate::resource`]).
    pub fn resource_type(
        &mut self,
        kind: &'static str,
        resolver: Arc<dyn crate::resource::ResourceResolver>,
    ) -> Result<(), Error> {
        self.resources.register(self.module, kind, resolver)
    }

    /// Registers a work source for the work inbox (step 51 P2); the id must be `<module>.<action>`.
    pub fn work_source(
        &mut self,
        source: Arc<dyn crate::ops::work::WorkSource>,
    ) -> Result<(), Error> {
        self.work.register(self.module, source)
    }

    /// Registers a source for cross-area search (step 51 P3); the kind must be `<module>.<kind>`.
    pub fn finder_source(
        &mut self,
        source: Arc<dyn crate::ops::find::FinderSource>,
    ) -> Result<(), Error> {
        self.finders.register(self.module, source)
    }

    /// Contributes a search suggestion source for the website (D-201); the kind must be `<module>.<kind>`.
    pub fn suggest_source(
        &mut self,
        source: Arc<dyn crate::ops::suggest::SuggestSource>,
    ) -> Result<(), Error> {
        self.suggestions.register(self.module, source)
    }

    /// The (shared) registry of suggestion sources: the module serving the suggestion endpoint holds it.
    /// Sources of modules registered later are still present at query time.
    pub fn suggest_sources(&self) -> crate::ops::suggest::SuggestSources {
        self.suggestions.clone()
    }

    /// Declares the module's searchable document kind (step 40 P7): `kind.kind` is `<module>.<kind>`,
    /// `source` reads documents from the module's tables. When a record changes, call
    /// `SearchIndex::enqueue` inside the change's transaction (obtained with
    /// `context.require::<SearchIndex>()`); a job syncs after commit.
    pub fn search_documents(
        &mut self,
        kind: crate::search::SearchKind,
        source: Arc<dyn crate::search::SearchSource>,
    ) -> Result<(), Error> {
        self.require::<crate::search::SearchIndex>()?.register(self.module, kind, source)
    }

    /// Adds a search engine named `name` (an external engine module such as Meilisearch, D-220); used when
    /// `AUTOXIDE_SEARCH_BACKEND = name`. Business modules need no changes.
    pub fn search_backend(
        &mut self,
        name: &str,
        backend: Arc<dyn crate::search::SearchBackend>,
    ) -> Result<(), Error> {
        self.require::<crate::search::SearchIndex>()?.add_backend(name, backend)
    }

    /// Registers a summary metric (step 51 P4); the id must be `<module>.<metric>` and must have a
    /// definition.
    pub fn summary_metric(
        &mut self,
        metric: Arc<dyn crate::ops::summary::SummaryMetric>,
    ) -> Result<(), Error> {
        self.metrics.register(self.module, metric)
    }

    /// The registry of every module's summary metrics (step 51 P4): reads another module's metrics through
    /// a shared contract (the anomaly alerts of `automations`, step 58). It includes metrics of modules
    /// registered later.
    pub fn summary_metrics(&self) -> crate::ops::summary::SummaryMetrics {
        self.metrics.clone()
    }

    /// Registers an exportable list (step 51 P16); `resource` must be the list id `<module>.<operation id>`
    /// from P7.
    pub fn export_source(
        &mut self,
        source: Arc<dyn crate::ops::list_exports::ExportSource>,
    ) -> Result<(), Error> {
        self.exports.register(self.module, source)
    }

    /// Registers a bulk action (step 51 P8) on a list `<module>.<operation id>`.
    pub fn bulk_action(
        &mut self,
        action: Arc<dyn crate::ops::bulk::BulkAction>,
    ) -> Result<(), Error> {
        self.bulk.register(self.module, action)
    }

    /// Registers a code resolver (step 51 P9) for `GET /kernel/resolve`; kind `<module>.<kind>`.
    pub fn code_resolver(
        &mut self,
        resolver: Arc<dyn crate::ops::resolve::CodeResolver>,
    ) -> Result<(), Error> {
        self.resolvers.register(self.module, resolver)
    }

    /// Registers a setup step (step 51 P10) for `GET /kernel/setup`; id `<module>.<step>`, the English
    /// label translated through the module's `text:`.
    pub fn setup_step(&mut self, step: Arc<dyn crate::ops::setup::SetupStep>) -> Result<(), Error> {
        self.setup.register(self.module, step)
    }

    /// The reason lists of every module (step 51 P18): a service validates the code a user sends with
    /// `ReasonCodes::ensure`.
    pub fn reason_codes(&self) -> crate::ops::reasons::ReasonCodes {
        self.reasons.clone()
    }

    /// Registers how to test an outbound connection (`<module>.connection.<kind>`) for
    /// `POST /api/v1/kernel/settings/{name}/test`.
    pub fn connection_probe(
        &mut self,
        probe: Arc<dyn crate::ops::connections::ConnectionProbe>,
    ) -> Result<(), Error> {
        self.probes.register(self.module, probe)
    }

    /// Allows internal labels (step 51 P20) on the module's record kind `kind` (must be in the object
    /// registry); `write`: the permission to write the record - needed to attach / detach labels.
    pub fn taggable(&mut self, kind: &'static str, write: Permission) -> Result<(), Error> {
        self.taggables.register(self.module, kind, write)
    }

    /// Declares the module's records that point to other records (step 51, D-267): "where is it used".
    pub fn reference_source(&mut self, source: Arc<dyn crate::ops::usages::ReferenceSource>) {
        self.references.register(self.module, source);
    }

    /// Custom fields (step 51 P19): validates values, writes filter indexes, builds filter conditions.
    pub fn custom_fields(&self) -> crate::ops::custom_fields::CustomFields {
        self.custom_fields.clone()
    }

    /// Tenant-customized texts (step 51 P21): looked up before the default text.
    pub fn text_overrides(&self) -> crate::ops::texts::TextOverrides {
        self.texts.clone()
    }

    /// Records the last use / error of an outbound connection (shown at `GET /api/v1/kernel/settings`).
    pub fn connection_status(&self) -> crate::ops::connections::ConnectionStatus {
        crate::ops::connections::ConnectionStatus
    }

    /// The object registry of every module, for looking up objects of other modules. The registry includes
    /// kinds registered by modules that start later.
    pub fn resources(&self) -> crate::resource::ResourceDirectory {
        self.resources.clone()
    }

    /// Registers an author-label calculator for object kind `kind` (of any module), with every label
    /// `codes` it may return (see [`crate::resource`], D-202).
    pub fn badge_source(
        &mut self,
        kind: &'static str,
        codes: &'static [&'static str],
        resolver: Arc<dyn crate::resource::BadgeResolver>,
    ) -> Result<(), Error> {
        self.resources.register_badges(self.module, kind, codes, resolver)
    }

    /// The block registry of every module (step 67): validates props, reads block data in batches. The
    /// registry includes the blocks of modules that start later.
    pub fn blocks(&self) -> crate::blocks::Blocks {
        self.blocks.clone()
    }

    /// Attaches a data provider `source` to the block `name` that this module declares with
    /// [`BlockType::with_data`](crate::blocks::BlockType::with_data).
    pub fn block_data(
        &mut self,
        name: &'static str,
        source: Arc<dyn crate::blocks::BlockData>,
    ) -> Result<(), Error> {
        self.blocks.register(self.module, name, source)
    }

    /// Lifecycle events of every module declared through [`Module::resource_events`] - complete from the
    /// very first module's start.
    pub fn resource_events(&self) -> &[crate::resource::ResourceEvent] {
        self.resource_events
    }

    /// Registers the "is the account verified to post content" checker (Decree 147/2024, D-202). Only one
    /// module in the program registers it.
    pub fn posting_verifier(
        &mut self,
        verifier: Arc<dyn crate::access::PostingVerifier>,
    ) -> Result<(), Error> {
        self.verification.register(self.module, verifier)
    }

    /// Where to ask whether an account is verified (also sees checkers registered by modules that start
    /// later).
    pub fn posting_verification(&self) -> crate::access::PostingVerification {
        self.verification.clone()
    }

    /// Publishes a trait for other modules to use. `T` is the `dyn Trait` type declared in the module's
    /// `api`: `context.provide::<dyn UserDirectory>(Arc::new(service))?`.
    ///
    /// Each trait has only one publisher; a second publication is an error. The kernel's partner contracts
    /// (payment, SMS, mail...) are wrapped by the kernel so the sandbox always uses the fake implementation
    /// (D-232).
    //
    // `T: ?Sized`: cho phép `T` là `dyn Trait` (kiểu không có kích thước cố định).
    pub fn provide<T>(&mut self, api: Arc<T>) -> Result<(), Error>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        let api = self.partners.wrap(api);
        self.apis.provide(self.module, api)
    }

    /// Fetches a trait published by a module registered BEFORE:
    /// `let users = context.require::<dyn UserDirectory>()?;`
    pub fn require<T>(&self) -> Result<Arc<T>, Error>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.apis.require(self.module)
    }

    /// A handle to the trait `T` published by any module - including one registered AFTER
    /// (D-20261122-late-apis). The kernel fills it after every module has started; read it with
    /// [`Later::get`] while serving, not in `start`. No module publishes it (disabled, not installed) ->
    /// `None`. Use it when `require` is not possible because of the registration order, e.g. `auth` asking
    /// `orders`: `let phones = context.later::<dyn VerifiedPhoneTokens>();`
    pub fn later<T>(&mut self) -> Later<T>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.apis.later()
    }

    /// Contributes an implementation for a trait that MANY modules provide (step 36), e.g. each partner
    /// connector module contributes a `dyn PaymentGateway`:
    /// `context.contribute::<dyn PaymentGateway>(Arc::new(vnpay))?`. Unlike [`provide`](Self::provide)
    /// (exactly one publisher): any number of modules may contribute, and each module contributes each trait
    /// at most once.
    ///
    /// It is also the one way to plug into the kernel's extension points: initial data for a new tenant
    /// (`dyn TenantSeed`), sample data (`dyn SampleData`), sandbox copy (`dyn SandboxCopy`), personal data
    /// (`dyn PersonalData`), work inbox, search, summary metrics, list exports, bulk actions, code resolvers,
    /// setup steps, connection probes, references and the posting verifier — see the table in the
    /// `module::contribute` source, and `docs/MODULE_GUIDE.md`. The older one-function-per-concept
    /// registrations (`personal_data`, `work_source`…) still work during the transition.
    pub fn contribute<T>(&mut self, api: Arc<T>) -> Result<(), Error>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        // `&api` (một `&Arc<T>`) tự đổi thành `&dyn Any` để kernel thử từng registry của nó.
        if let Some(registered) = self.contribute_to_kernel(&api) {
            return registered;
        }
        let api = self.partners.wrap(api);
        self.apis.contribute(self.module, api)
    }

    /// Every implementation of `T` contributed by modules registered BEFORE, in registration order; if
    /// nobody has contributed it is empty (the user decides whether that is an error).
    pub fn contributions<T>(&self) -> Vec<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.apis.contributions()
    }

    /// Every report fact source (`Module::facts`) of every registered module - including modules registered
    /// later - with the module name, in registration order (step 38). The `reporting` module reads them to
    /// listen to exactly those events.
    pub fn fact_sources(&self) -> Vec<(&'static str, crate::report::FactSource)> {
        self.facts.sources()
    }

    /// Every analytics event kind and funnel of every registered module - including modules registered
    /// later (step 57). The `analytics` module reads them to know which events to accept.
    pub fn analytics_catalog(&self) -> crate::analytics::AnalyticsCatalog {
        self.analytics.clone()
    }

    /// The registry of MCP tools and automation rule templates of every module (step 58), filled in after
    /// EVERY module has started: hold the handle at `start`, read only while serving.
    pub fn automation_directory(&self) -> crate::automation::AutomationDirectory {
        self.automations.clone()
    }

    /// The sandbox copy / push registry of every module (used only by the kernel); read while serving, not
    /// at `start`.
    pub(crate) fn sandbox_catalog(&self) -> crate::tenancy::sandbox::SandboxCatalog {
        self.sandbox.clone()
    }

    /// The distributor of parent tenant -> child tenant data for this module (step 38): writes a
    /// distribution in the parent tenant's transaction, and the `<module>.distribute` job applies it to
    /// each child tenant. See [`crate::tenancy::distribution`].
    pub fn distributor(&self) -> crate::tenancy::distribution::Distributor {
        crate::tenancy::distribution::Distributor::new(
            self.module,
            self.tenancy.clone(),
            self.entitlements.clone(),
        )
    }
}

/// Names the database reserves as prefixes for system tables: SQLite reserves `sqlite_…`, Postgres reserves
/// `pg_…`. A module with such a name would create tables that are invalid on one of them.
const RESERVED_NAMES: [&str; 2] = ["sqlite", "pg"];

/// The problems in a list of module names; empty means valid.
pub(crate) fn module_name_problems(names: &[&str]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = BTreeSet::new();
    for name in names {
        if name.is_empty() {
            // Module không cài `manifest` (mặc định không tên) mà cũng không cài `name` cũ.
            problems.push(
                "module name `` must be lowercase letters, digits and `_`, starting with a letter: a module with \
                 no name does not implement `Module::manifest` (build it with `Manifest::new(name, …)`)"
                    .to_owned(),
            );
            continue;
        }
        if !is_identifier(name) {
            problems.push(format!(
                "module name `{name}` must be lowercase letters, digits and `_`, starting with a letter"
            ));
        }
        if RESERVED_NAMES.contains(name) {
            problems.push(format!(
                "module name `{name}` is reserved: its tables would use a prefix the database \
                 keeps for system tables"
            ));
        }
        if !seen.insert(*name) {
            problems.push(format!("module `{name}` is registered twice"));
        }
    }
    // `auth` và `auth_keys` cùng tồn tại thì bảng `auth_keys_x` thuộc về ai? Cấm
    // từ đầu để tiền tố bảng luôn chỉ ra đúng một module.
    for first in &seen {
        for second in &seen {
            if second.starts_with(&format!("{first}_")) {
                problems.push(format!(
                    "module names `{first}` and `{second}` are ambiguous: tables of `{second}` \
                     would also carry the prefix of `{first}`"
                ));
            }
        }
    }
    problems
}

/// Lowercase letters, digits and `_`; starts with a letter; does not end with `_`.
pub(crate) fn is_identifier(text: &str) -> bool {
    text.chars().next().is_some_and(|first| first.is_ascii_lowercase())
        && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !text.ends_with('_')
}
