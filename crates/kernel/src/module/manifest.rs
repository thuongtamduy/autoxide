//! [`Manifest`]: everything a module declares that is known before it starts (D-20261126-module-manifest).
//!
//! The rule in one sentence: what the core needs to know without starting the module (migrations, permissions,
//! events, settings, translations, quotas, dependencies, tier…) goes into the manifest; what needs the module's
//! services (routes, MCP tools, subscriptions, jobs…) is registered on [`StartContext`](super::StartContext) in
//! [`Module::start`](super::Module::start).

use crate::access::Permission;
use crate::event::EventInfo;
use crate::migration::{DataUpgrade, MigrationTrait};

use super::Stability;
use super::navigation::Navigation;

/// Builds the module's migrations (usually the module's own `migration::all`).
pub type MigrationsFn = fn() -> Vec<Box<dyn MigrationTrait>>;
/// Builds the module's data upgrades (usually the module's own `migration::upgrades`).
pub type UpgradesFn = fn() -> Vec<Box<dyn DataUpgrade>>;

/// What a module declares to the core before it starts: returned by
/// [`Module::manifest`](super::Module::manifest).
///
/// Built with one function per concept; every list function adds to what is already there, so a manifest can
/// be built in steps:
///
/// ```
/// use autoxide_kernel::module::{Manifest, Stability};
///
/// let manifest = Manifest::new("notes", "Personal notes.")
///     .stability(Stability::Experimental)
///     .depends_on(["mail"])
///     .translation("vi", r#"{"notes.not_found": "Note not found."}"#);
/// assert_eq!(manifest.name(), "notes");
/// ```
///
/// New concepts arrive as new builder functions, so a module written today keeps compiling. Concepts whose
/// shape may still change are declared through [`unstable`](Self::unstable).
//
// Trường `pub(crate)`: trait `Module` (ở `module.rs`, mô-đun cha) đọc thẳng để dựng các hàm cũ; người ngoài crate
// chỉ thấy builder. `#[non_exhaustive]` không cần vì không có trường công khai — thêm trường không phá vỡ.
#[derive(Clone)]
pub struct Manifest {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    /// `false` only for [`Manifest::legacy`]: the module still declares itself with the old `Module` functions.
    pub(crate) declared: bool,
    pub(crate) stability: Stability,
    pub(crate) essential: bool,
    pub(crate) always_enabled: bool,
    pub(crate) depends_on: Vec<&'static str>,
    pub(crate) migrations: Option<MigrationsFn>,
    pub(crate) upgrades: Option<UpgradesFn>,
    pub(crate) records: Vec<crate::db::RecordInfo>,
    pub(crate) tenant_settings: Vec<crate::tenancy::TenantSetting>,
    pub(crate) org_settings: Vec<crate::tenancy::OrgSetting>,
    pub(crate) retained_tables: Vec<&'static str>,
    pub(crate) retention: Vec<crate::privacy::RetentionPolicy>,
    pub(crate) control_plane_tables: Vec<&'static str>,
    pub(crate) features: Vec<crate::entitlement::Feature>,
    pub(crate) limits: Vec<crate::entitlement::Limit>,
    pub(crate) permissions: Vec<Permission>,
    pub(crate) events: Vec<EventInfo>,
    pub(crate) resource_events: Vec<crate::resource::ResourceEvent>,
    pub(crate) schedules: Vec<crate::job::Schedule>,
    pub(crate) deprecations: Vec<crate::http::Deprecation>,
    pub(crate) body_limits: Vec<crate::http::BodyLimit>,
    pub(crate) translations: Vec<(&'static str, &'static str)>,
    pub(crate) navigation: Navigation,
    pub(crate) reason_lists: Vec<crate::ops::reasons::ReasonList>,
    pub(crate) overridable_texts: Vec<crate::ops::texts::OverridableText>,
    pub(crate) relocation: crate::tenancy::Relocation,
    pub(crate) unstable: UnstableManifest,
}

impl Manifest {
    /// A manifest for the module `name` (lowercase letters, digits and `_`, starting with a letter; also the prefix
    /// of every table and configuration variable of the module), with one English sentence saying what the module
    /// does (for `/llms.txt` and the module registry).
    pub fn new(name: &'static str, description: &'static str) -> Self {
        Manifest { name, description, declared: true, ..Self::legacy() }
    }

    /// What [`Module::manifest`](super::Module::manifest) returns for a module that has not moved to the
    /// manifest yet: no name, nothing declared. The core then reads the module's old `Module` functions.
    pub(crate) fn legacy() -> Self {
        Manifest {
            name: "",
            description: "",
            declared: false,
            stability: Stability::Official,
            essential: false,
            always_enabled: false,
            depends_on: Vec::new(),
            migrations: None,
            upgrades: None,
            records: Vec::new(),
            tenant_settings: Vec::new(),
            org_settings: Vec::new(),
            retained_tables: Vec::new(),
            retention: Vec::new(),
            control_plane_tables: Vec::new(),
            features: Vec::new(),
            limits: Vec::new(),
            permissions: Vec::new(),
            events: Vec::new(),
            resource_events: Vec::new(),
            schedules: Vec::new(),
            deprecations: Vec::new(),
            body_limits: Vec::new(),
            translations: Vec::new(),
            navigation: Navigation::default(),
            reason_lists: Vec::new(),
            overridable_texts: Vec::new(),
            relocation: crate::tenancy::Relocation::inferred(),
            unstable: UnstableManifest::default(),
        }
    }

    /// The module name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The one-sentence description.
    pub fn description(&self) -> &'static str {
        self.description
    }

    /// The module's tier (step 70 P3): `Official` (default) runs when the installer chooses nothing;
    /// `Experimental` - API and tables may still change - runs only when explicitly enabled in `AUTOXIDE_MODULES`.
    /// An official module may depend only on official modules.
    pub fn stability(mut self, stability: Stability) -> Self {
        self.stability = stability;
        self
    }

    /// The core cannot run without this module (login, idempotency...): always enabled, `-name` in
    /// `AUTOXIDE_MODULES` is rejected, and so are the modules it depends on. About the whole installation, unlike
    /// [`always_enabled`](Self::always_enabled).
    pub fn essential(mut self) -> Self {
        self.essential = true;
        self
    }

    /// A foundation module that a tenant's plan never blocks (login, files, audit...). Without it the module is
    /// governed by plans: while not enabled for a tenant, its routes, MCP tools, jobs and events are blocked.
    pub fn always_enabled(mut self) -> Self {
        self.always_enabled = true;
        self
    }

    /// Modules this module MUST have: every `context.require::<…>()?` to a trait of another module (and every
    /// event of another module that it listens to) names a module here. Enabling this module enables them.
    /// Things used "if present" (`.require::<…>().ok()`) are not declared here.
    pub fn depends_on(mut self, modules: impl IntoIterator<Item = &'static str>) -> Self {
        self.depends_on.extend(modules);
        self
    }

    /// The function that builds the module's migrations, in the order they are applied. Once committed a
    /// migration is never edited - add a new one at the end.
    pub fn migrations(mut self, migrations: MigrationsFn) -> Self {
        self.migrations = Some(migrations);
        self
    }

    /// The function that builds the module's data upgrades (D-257): upgrades that need the program's
    /// configuration, such as re-encrypting old secrets.
    pub fn upgrades(mut self, upgrades: UpgradesFn) -> Self {
        self.upgrades = Some(upgrades);
        self
    }

    /// The module's records (`#[derive(Record)]`), as `RecordInfo::of::<Note>()`: at startup the kernel checks each
    /// against the live schema and refuses to start on a mismatch.
    pub fn records(mut self, records: impl IntoIterator<Item = crate::db::RecordInfo>) -> Self {
        self.records.extend(records);
        self
    }

    /// Configuration the module accepts per tenant (D-258), named `<module>.<key>`; a tenant that has not set one
    /// falls back to the shared configuration.
    pub fn tenant_settings(
        mut self,
        settings: impl IntoIterator<Item = crate::tenancy::TenantSetting>,
    ) -> Self {
        self.tenant_settings.extend(settings);
        self
    }

    /// Settings inherited down the organization tree (step 34), named `<module>.<name>`.
    pub fn org_settings(
        mut self,
        settings: impl IntoIterator<Item = crate::tenancy::OrgSetting>,
    ) -> Self {
        self.org_settings.extend(settings);
        self
    }

    /// Tenant-level tables that must be KEPT when a tenant is permanently deleted (D-337), such as accounting
    /// records kept for the period the law requires.
    pub fn retained_tables(mut self, tables: impl IntoIterator<Item = &'static str>) -> Self {
        self.retained_tables.extend(tables);
        self
    }

    /// The kinds of personal data the module holds and how long each is kept (step 37, D-360). The manifest is
    /// read again after `configure`, so periods may come from the module's configuration.
    pub fn retention(
        mut self,
        policies: impl IntoIterator<Item = crate::privacy::RetentionPolicy>,
    ) -> Self {
        self.retention.extend(policies);
        self
    }

    /// Control-plane tables (step 33, D-253): shared across the platform, living only in the control database.
    /// Every other table of the module is tenant-level and must have `tenant_id`.
    pub fn control_plane_tables(mut self, tables: impl IntoIterator<Item = &'static str>) -> Self {
        self.control_plane_tables.extend(tables);
        self
    }

    /// Plan-based on / off features (`<module>.<name>`, step 35).
    pub fn features(
        mut self,
        features: impl IntoIterator<Item = crate::entitlement::Feature>,
    ) -> Self {
        self.features.extend(features);
        self
    }

    /// Plan-based quotas (`<module>.<name>`, step 35). A real counter for a quota is registered at start with
    /// [`StartContext::usage_counter`](super::StartContext::usage_counter).
    pub fn limits(mut self, limits: impl IntoIterator<Item = crate::entitlement::Limit>) -> Self {
        self.limits.extend(limits);
        self
    }

    /// The module's permissions, named `<module>.<action>`. A released permission is a contract: renaming it makes
    /// roles that hold the old name lose access.
    pub fn permissions(mut self, permissions: impl IntoIterator<Item = Permission>) -> Self {
        self.permissions.extend(permissions);
        self
    }

    /// The events the module emits. A module with events adds `event::outbox_migration` to its migrations and
    /// records events with the `Outbox` from [`StartContext::kernel`](super::StartContext::kernel).
    pub fn events(mut self, events: impl IntoIterator<Item = EventInfo>) -> Self {
        self.events.extend(events);
        self
    }

    /// Lifecycle events of the object kinds the module owns (D-202); each event must be one of the module's own
    /// [`events`](Self::events).
    pub fn resource_events(
        mut self,
        events: impl IntoIterator<Item = crate::resource::ResourceEvent>,
    ) -> Self {
        self.resource_events.extend(events);
        self
    }

    /// Periodic schedules: each puts a job of a kind the module runs (registered at start with
    /// [`StartContext::job`](super::StartContext::job)) on the queue after every interval.
    pub fn schedules(mut self, schedules: impl IntoIterator<Item = crate::job::Schedule>) -> Self {
        self.schedules.extend(schedules);
        self
    }

    /// Deprecated endpoints and tools (D-059): marked in OpenAPI, answered with `Deprecation` / `Sunset` /
    /// `Link` headers.
    pub fn deprecations(
        mut self,
        deprecations: impl IntoIterator<Item = crate::http::Deprecation>,
    ) -> Self {
        self.deprecations.extend(deprecations);
        self
    }

    /// Routes that need a request body larger than the shared limit (`AUTOXIDE_HTTP_BODY_LIMIT_BYTES`), such as
    /// file uploads. Read after `configure`, so the limit may come from the module's configuration.
    pub fn body_limits(mut self, limits: impl IntoIterator<Item = crate::http::BodyLimit>) -> Self {
        self.body_limits.extend(limits);
        self
    }

    /// Translations of the module's error codes and texts for `language`, as JSON, usually
    /// `.translation("vi", include_str!("../locales/vi.json"))`. Every error `code` of the module must have one
    /// (`cargo xtask check` verifies this).
    pub fn translation(mut self, language: &'static str, json: &'static str) -> Self {
        self.translations.push((language, json));
        self
    }

    /// Sub-area groups and menu items for the UI (step 51 P1).
    pub fn navigation(mut self, navigation: Navigation) -> Self {
        self.navigation = navigation;
        self
    }

    /// Reason lists (step 51 P18, D-223), named `<module>.<action>`.
    pub fn reason_lists(
        mut self,
        lists: impl IntoIterator<Item = crate::ops::reasons::ReasonList>,
    ) -> Self {
        self.reason_lists.extend(lists);
        self
    }

    /// Tenant-customizable texts (step 51 P21, D-235), keyed `<module>.<…>`.
    pub fn overridable_texts(
        mut self,
        texts: impl IntoIterator<Item = crate::ops::texts::OverridableText>,
    ) -> Self {
        self.overridable_texts.extend(texts);
        self
    }

    /// How each tenant-level table travels when a branch is split off or two tenants are merged (step 38).
    /// Without it the policy is inferred from the columns.
    pub fn relocation(mut self, relocation: crate::tenancy::Relocation) -> Self {
        self.relocation = relocation;
        self
    }

    /// Declarations of the experimental tier (D-20261126-contract-tiers): their shape may change in a minor
    /// release.
    pub fn unstable(mut self, build: impl FnOnce(UnstableManifest) -> UnstableManifest) -> Self {
        self.unstable = build(self.unstable);
        self
    }
}

impl std::fmt::Debug for Manifest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Manifest")
            .field("name", &self.name)
            .field("stability", &self.stability)
            .finish_non_exhaustive()
    }
}

/// The experimental part of a [`Manifest`]: concepts whose shape may still change in a minor release
/// (D-20261126-contract-tiers).
#[derive(Clone, Default)]
pub struct UnstableManifest {
    pub(crate) custom_field_targets: Vec<crate::ops::custom_fields::CustomFieldTarget>,
    pub(crate) facts: Vec<crate::report::FactSource>,
    pub(crate) analytics_events: Vec<crate::analytics::AnalyticsEvent>,
    pub(crate) funnels: Vec<crate::analytics::Funnel>,
    pub(crate) automation_templates: Vec<crate::automation::AutomationTemplate>,
    pub(crate) promotable: Vec<crate::tenancy::sandbox::Promotable>,
    pub(crate) sync_resources: Vec<crate::sync::SyncResource>,
    pub(crate) blocks: Vec<crate::blocks::BlockType>,
}

impl UnstableManifest {
    /// Record kinds that accept custom fields (step 51 P19, D-235).
    pub fn custom_field_targets(
        mut self,
        targets: impl IntoIterator<Item = crate::ops::custom_fields::CustomFieldTarget>,
    ) -> Self {
        self.custom_field_targets.extend(targets);
        self
    }

    /// Report figures the module's own events produce (step 38).
    pub fn facts(mut self, facts: impl IntoIterator<Item = crate::report::FactSource>) -> Self {
        self.facts.extend(facts);
        self
    }

    /// Web analytics event kinds (step 57).
    pub fn analytics_events(
        mut self,
        events: impl IntoIterator<Item = crate::analytics::AnalyticsEvent>,
    ) -> Self {
        self.analytics_events.extend(events);
        self
    }

    /// Funnels whose steps are declared analytics event kinds (step 57).
    pub fn funnels(mut self, funnels: impl IntoIterator<Item = crate::analytics::Funnel>) -> Self {
        self.funnels.extend(funnels);
        self
    }

    /// Built-in automation rule templates, id `<module>.<name>` (step 58).
    pub fn automation_templates(
        mut self,
        templates: impl IntoIterator<Item = crate::automation::AutomationTemplate>,
    ) -> Self {
        self.automation_templates.extend(templates);
        self
    }

    /// Record kinds that can be pushed from a sandbox to the real tenant as drafts (step 38, D-232).
    pub fn promotable(
        mut self,
        kinds: impl IntoIterator<Item = crate::tenancy::sandbox::Promotable>,
    ) -> Self {
        self.promotable.extend(kinds);
        self
    }

    /// Record kinds an offline app can sync (step 69 P2): the module adds `sync::changes_migration` to its
    /// migrations and records changes with the `SyncLog` in the write transaction.
    pub fn sync_resources(
        mut self,
        resources: impl IntoIterator<Item = crate::sync::SyncResource>,
    ) -> Self {
        self.sync_resources.extend(resources);
        self
    }

    /// Blocks for block-built pages (step 67); a block with data attaches it at start.
    pub fn blocks(mut self, blocks: impl IntoIterator<Item = crate::blocks::BlockType>) -> Self {
        self.blocks.extend(blocks);
        self
    }
}

impl std::fmt::Debug for UnstableManifest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("UnstableManifest").finish_non_exhaustive()
    }
}
