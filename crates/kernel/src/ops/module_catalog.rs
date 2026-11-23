//! Installed-module registry: lets administrators (and AI agents) see which modules the running program
//! contains and what each module declares — permissions, events, jobs, schedules, MCP tools, deprecated
//! endpoints, migrations. This is the API part that a "manage modules" screen in the UI project
//! uses (D-155).
//!
//! The registry is READ-ONLY. Modules are registered at compile time (`crates/app/src/modules.rs`), so there
//! is no enabling / disabling modules at runtime.
//!
//! How it is built: [`Modules::new`](crate::module::Modules::new) creates an empty [`ModuleCatalog`] and
//! gives a copy to the internal `kernel` module; [`Modules::start`](crate::module::Modules::start)
//! fills the registry after EVERY module has started (tools, jobs and schedules exist only after `start`). The
//! `kernel` routes read the registry while serving requests:
//!
//! - `GET /api/v1/kernel/modules` — every module, in registration order; does not touch the database;
//! - `GET /api/v1/kernel/modules/{name}` — one module, with the status of each migration (read from
//!   that module's own `<name>_migrations` table).
//!
//! Both need the permission `kernel.modules.read`. The MCP tools `kernel.list_modules` and
//! `kernel.get_module` call the same functions.

use std::sync::{Arc, OnceLock};

use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;

use crate::access::{Permission, Principal};
use crate::database::Database;
use crate::error::Error;
use crate::i18n::Language;
use crate::mcp::{NoInput, Tool};
use crate::migration::{Migrator, ModuleMigrations};
use crate::module::Module;
use crate::response::{Data, Problem};

/// Permission to view the module registry.
pub(crate) const MODULES_READ: Permission = Permission::new(
    "kernel.modules.read",
    "See the installed modules and what each declares: permissions, events, jobs, schedules, \
     MCP tools, deprecated endpoints and migration status.",
);

/// The two internal kernel modules that are always installed (not registered by the binary).
const INTERNAL: [&str; 2] = [crate::builtin::NAME, crate::antispam::module::NAME];

/// The module registry, shared between `Modules` (the filler) and the `kernel` module (the reader).
//
// `Arc<OnceLock<…>>`: hai bên giữ cùng một ô; ô được ghi đúng một lần (sau `start`) rồi chỉ
// đọc, nên đọc từ nhiều thread không cần lock.
#[derive(Clone, Default)]
pub(crate) struct ModuleCatalog {
    entries: Arc<OnceLock<Vec<Entry>>>,
    /// Check steps (`autoxide doctor`) of every module, with the module name: operational alerts
    /// (`GET /kernel/ops/alerts`) rerun them.
    checks: Arc<OnceLock<Vec<(&'static str, crate::doctor::Check)>>>,
    /// Registry of work-inbox sources (step 51 P2): modules register at startup.
    work: crate::ops::work::WorkSources,
    /// Registry of cross-module search sources (step 51 P3).
    finders: crate::ops::find::FinderSources,
    /// Registry of search-suggestion sources for the website (D-201).
    suggestions: crate::ops::suggest::SuggestSources,
    /// Registry of summary metrics (step 51 P4).
    metrics: crate::ops::summary::SummaryMetrics,
    /// Registry of exportable lists (step 51 P16).
    exports: crate::ops::list_exports::ExportSources,
    /// Registry of bulk actions (step 51 P8).
    bulk: crate::ops::bulk::BulkActions,
    /// Registry of code recognizers (step 51 P9).
    resolvers: crate::ops::resolve::CodeResolvers,
    /// Registry of setup steps (step 51 P10).
    setup: crate::ops::setup::SetupSteps,
    /// List of reasons (step 51 P18).
    reasons: crate::ops::reasons::ReasonCodes,
    /// Registry of blocks for block-built pages (step 67).
    blocks: crate::blocks::Blocks,
    /// How to test outgoing connections (step 51).
    probes: crate::ops::connections::ConnectionProbes,
    /// Record kinds that can be labeled (step 51 P20).
    taggables: crate::ops::tags::Taggables,
    /// Phrases a tenant can change and phrases a tenant has changed (step 51 P21).
    texts: crate::ops::texts::TextOverrides,
    /// Custom fields (step 51 P19).
    custom_fields: crate::ops::custom_fields::CustomFields,
    /// Source for "where is this used" (step 51, D-267).
    references: crate::ops::usages::ReferenceSources,
    /// Per-record history (step 51 P13), published by the audit-log module; filled after every module has started.
    history: crate::ops::records::HistorySlot,
    /// The AI model (published by the `ai` module), filled after every module has started (step 52).
    model: crate::ops::filter_parse::TextModelSlot,
    /// WebSocket signal topics that modules contribute (step 69 P3), filled after every module has started.
    signal_topics: crate::event::signals::SignalTopicsSlot,
    /// Filter schema of every list (P7), filled when routes are built.
    lists: crate::ops::list_exports::ListsHandle,
    /// The gathered navigation registry (step 51 P1).
    navigation: Arc<OnceLock<crate::module::navigation::NavigationCatalog>>,
}

/// A module in the registry: its declaration (unchanged at runtime) and its own migrator, used to read
/// migration status.
struct Entry {
    info: ModuleInfo,
    migrator: Option<Migrator>,
}

impl ModuleCatalog {
    /// Fills the registry from the modules that have started. Call once; later calls are ignored.
    pub(crate) fn fill(
        &self,
        modules: &[Box<dyn Module>],
        entitlements: &crate::entitlement::EntitlementCatalog,
    ) {
        let entries = modules
            .iter()
            .enumerate()
            .map(|(position, module)| {
                let name = module.name();
                let migrations = module.migrations();
                let migration_names: Vec<String> =
                    migrations.iter().map(|migration| migration.name().to_owned()).collect();
                // `Migrator::new` chỉ lỗi khi khai báo sai — điều `Modules::migrator` đã kiểm
                // trước khi chương trình chạy tới đây. Lỗi thì chỉ mất phần trạng thái.
                // Kèm cả bản nâng cấp dữ liệu (`upgrades`): bảng lịch sử ghi cả chúng, thiếu thì đọc
                // trạng thái báo lỗi (module `auth`, `webhooks`).
                let set = ModuleMigrations::new(name, migrations).with_upgrades(module.upgrades());
                let migrator = Migrator::new(vec![set]).ok();
                let mut info = ModuleInfo::describe(module.as_ref(), position, migration_names);
                info.always_enabled = entitlements.is_always_enabled(name);
                info.requires =
                    entitlements.requires(name).into_iter().map(str::to_owned).collect();
                Entry { info, migrator }
            })
            .collect();
        // Đã điền rồi thì giữ bản cũ: sổ không đổi trong lúc chạy.
        let _ = self.entries.set(entries);
        let checks = modules
            .iter()
            .flat_map(|module| module.checks().into_iter().map(|check| (module.name(), check)))
            .collect();
        let _ = self.checks.set(checks);
    }

    pub(crate) fn work_sources(&self) -> crate::ops::work::WorkSources {
        self.work.clone()
    }

    pub(crate) fn finder_sources(&self) -> crate::ops::find::FinderSources {
        self.finders.clone()
    }

    pub(crate) fn suggest_sources(&self) -> crate::ops::suggest::SuggestSources {
        self.suggestions.clone()
    }

    pub(crate) fn summary_metrics(&self) -> crate::ops::summary::SummaryMetrics {
        self.metrics.clone()
    }

    pub(crate) fn export_sources(&self) -> crate::ops::list_exports::ExportSources {
        self.exports.clone()
    }

    pub(crate) fn bulk_actions(&self) -> crate::ops::bulk::BulkActions {
        self.bulk.clone()
    }

    pub(crate) fn code_resolvers(&self) -> crate::ops::resolve::CodeResolvers {
        self.resolvers.clone()
    }

    pub(crate) fn setup_steps(&self) -> crate::ops::setup::SetupSteps {
        self.setup.clone()
    }

    pub(crate) fn text_model(&self) -> crate::ops::filter_parse::TextModelSlot {
        self.model.clone()
    }

    pub(crate) fn signal_topics(&self) -> crate::event::signals::SignalTopicsSlot {
        self.signal_topics.clone()
    }

    pub(crate) fn history(&self) -> crate::ops::records::HistorySlot {
        self.history.clone()
    }

    pub(crate) fn reason_codes(&self) -> crate::ops::reasons::ReasonCodes {
        self.reasons.clone()
    }

    pub(crate) fn blocks(&self) -> crate::blocks::Blocks {
        self.blocks.clone()
    }

    pub(crate) fn connection_probes(&self) -> crate::ops::connections::ConnectionProbes {
        self.probes.clone()
    }

    pub(crate) fn taggables(&self) -> crate::ops::tags::Taggables {
        self.taggables.clone()
    }

    pub(crate) fn text_overrides(&self) -> crate::ops::texts::TextOverrides {
        self.texts.clone()
    }

    pub(crate) fn references(&self) -> crate::ops::usages::ReferenceSources {
        self.references.clone()
    }

    pub(crate) fn custom_fields(&self) -> crate::ops::custom_fields::CustomFields {
        self.custom_fields.clone()
    }

    pub(crate) fn lists(&self) -> crate::ops::list_exports::ListsHandle {
        self.lists.clone()
    }

    pub(crate) fn fill_navigation(&self, navigation: crate::module::navigation::NavigationCatalog) {
        let _ = self.navigation.set(navigation);
    }

    /// The navigation registry (empty before every module has started).
    pub(crate) fn navigation(&self) -> Option<&crate::module::navigation::NavigationCatalog> {
        self.navigation.get()
    }

    /// The check steps of every module (empty before every module has started).
    pub(crate) fn checks(&self) -> &[(&'static str, crate::doctor::Check)] {
        self.checks.get().map_or(&[], Vec::as_slice)
    }

    fn entries(&self) -> Result<&[Entry], Error> {
        self.entries.get().map(Vec::as_slice).ok_or_else(|| {
            Error::internal("the module catalog is read before every module has started")
        })
    }

    /// Every module, in registration order. The `applied` field of migrations is left `null`.
    pub(crate) fn list(&self) -> Result<Vec<ModuleInfo>, Error> {
        Ok(self.entries()?.iter().map(|entry| entry.info.clone()).collect())
    }

    /// One module by name, with migration status read from the database.
    pub(crate) async fn get(&self, database: &Database, name: &str) -> Result<ModuleInfo, Error> {
        let entry =
            self.entries()?.iter().find(|entry| entry.info.name == name).ok_or_else(|| {
                Error::not_found(
                    "kernel.module_not_found",
                    "No module named `{name}` is installed. `GET /api/v1/kernel/modules` lists \
                     the installed modules.",
                )
                .with_param("name", name.to_owned())
            })?;
        let mut info = entry.info.clone();
        if let Some(migrator) = &entry.migrator {
            let states = migrator.status(database).await.map_err(Error::internal)?;
            info.migrations = states
                .into_iter()
                .map(|state| ModuleMigration { name: state.name, applied: Some(state.applied) })
                .collect();
        }
        Ok(info)
    }
}

/// A module installed in this server, and everything it declares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ModuleInfo {
    /// The module name. Its REST endpoints live under `/api/v1/<name>`; its tables,
    /// permissions, events, error codes and job kinds start with `<name>`.
    #[schema(example = "notes")]
    name: String,
    /// One sentence about what the module does; empty when the module gives none.
    description: String,
    /// `true` for the modules the server always installs (`kernel`, `antispam`).
    internal: bool,
    /// Place in the start order, from 0. A module starts after the modules it depends on.
    position: u32,
    /// Permissions the module declares; roles grant them.
    permissions: Vec<ModulePermission>,
    /// Events the module emits.
    events: Vec<ModuleEvent>,
    /// Events the module listens to; `*` means every event.
    subscriptions: Vec<String>,
    /// Kinds of background job the module runs.
    jobs: Vec<String>,
    /// Recurring jobs the module schedules.
    schedules: Vec<ModuleSchedule>,
    /// MCP tools the module offers at `/mcp`.
    tools: Vec<ModuleTool>,
    /// MCP resources (read-only documents) the module offers at `/mcp`.
    resources: Vec<ModuleResource>,
    /// MCP prompts (templates for common jobs) the module offers at `/mcp`.
    prompts: Vec<ModulePrompt>,
    /// Endpoints of the module that are deprecated.
    deprecations: Vec<ModuleDeprecation>,
    /// Languages, besides English, the module's error messages are translated to.
    languages: Vec<String>,
    /// Database migrations of the module, in the order they apply.
    migrations: Vec<ModuleMigration>,
    /// Features of the module that a plan turns on or off (`<module>.<name>`).
    features: Vec<ModuleFeature>,
    /// Limits of the module that a plan sets (`<module>.<name>`).
    limits: Vec<ModuleLimit>,
    /// `true` when no plan can turn the module off (sign-in, files, audit…). Otherwise a tenant
    /// whose plan does not include the module gets `403 module_not_enabled` from its endpoints.
    always_enabled: bool,
    /// Modules this module uses; a plan that turns this module on must turn them on too.
    requires: Vec<String>,
    /// `official`, or `experimental`: the module's API and data may still change between
    /// releases, and it only runs where the installation turns it on (`AUTOXIDE_MODULES`).
    stability: String,
}

impl ModuleInfo {
    fn describe(module: &dyn Module, position: usize, migrations: Vec<String>) -> Self {
        let name = module.name();
        let mut languages: Vec<String> =
            module.translations().into_iter().map(|(language, _)| language.to_owned()).collect();
        languages.sort();
        languages.dedup();
        let deprecations = module.deprecations();
        ModuleInfo {
            name: name.to_owned(),
            description: module.description().to_owned(),
            resources: module
                .mcp_resources()
                .into_iter()
                .map(|resource| {
                    let resource = resource.qualify(name);
                    ModuleResource {
                        uri: resource.uri().to_owned(),
                        name: resource.name().to_owned(),
                        description: resource.description().to_owned(),
                        mime_type: resource.mime_type().to_owned(),
                        permissions: resource
                            .permission()
                            .map(|permission| permission.name().to_owned())
                            .into_iter()
                            .collect(),
                    }
                })
                .collect(),
            prompts: module
                .prompts()
                .into_iter()
                .map(|prompt| ModulePrompt {
                    name: format!("{name}.{}", prompt.name()),
                    description: prompt.description().to_owned(),
                    arguments: prompt
                        .arguments()
                        .iter()
                        .map(|argument| ModulePromptArgument {
                            name: argument.name.to_owned(),
                            description: argument.description.to_owned(),
                            required: argument.required,
                        })
                        .collect(),
                    tools: prompt
                        .uses_short()
                        .iter()
                        .map(|tool| format!("{name}.{tool}"))
                        .collect(),
                })
                .collect(),
            internal: INTERNAL.contains(&name),
            position: u32::try_from(position).unwrap_or(u32::MAX),
            permissions: module
                .permissions()
                .into_iter()
                .map(|permission| ModulePermission {
                    name: permission.name().to_owned(),
                    description: permission.description().to_owned(),
                    risk: permission.risk().as_str().to_owned(),
                })
                .collect(),
            events: module
                .events()
                .into_iter()
                .map(|event| ModuleEvent {
                    name: event.name().to_owned(),
                    description: event.description().to_owned(),
                    stream_permission: event
                        .stream_permission()
                        .map(|permission| permission.name().to_owned()),
                    stream_audience: event.stream_permission().is_some() && event.has_audience(),
                })
                .collect(),
            subscriptions: module
                .subscriptions()
                .iter()
                .map(|subscription| subscription.event().to_owned())
                .collect(),
            jobs: module.jobs().iter().map(|job| job.kind().to_owned()).collect(),
            schedules: module
                .schedules()
                .into_iter()
                .map(|schedule| ModuleSchedule {
                    name: schedule.name.to_owned(),
                    kind: schedule.kind.to_owned(),
                    every_seconds: schedule.every.as_secs(),
                })
                .collect(),
            tools: module
                .tools()
                .into_iter()
                .map(|tool| {
                    let tool = tool.qualify(name);
                    ModuleTool {
                        name: tool.name().to_owned(),
                        description: tool.description().to_owned(),
                        permissions: tool
                            .permission()
                            .into_iter()
                            .chain(tool.alternative())
                            .map(|permission| permission.name().to_owned())
                            .collect(),
                        read_only: tool.is_read_only(),
                        destructive: tool.is_destructive(),
                        deprecation: deprecations
                            .iter()
                            .find(|deprecation| {
                                deprecation
                                    .tool
                                    .is_some_and(|short| tool.name() == format!("{name}.{short}"))
                            })
                            .map(|deprecation| ModuleToolDeprecation {
                                since: deprecation.since.to_owned(),
                                sunset: deprecation.sunset.map(str::to_owned),
                                successor: deprecation
                                    .successor
                                    .map(|successor| format!("{name}.{successor}")),
                            }),
                    }
                })
                .collect(),
            deprecations: deprecations
                .iter()
                .filter(|deprecation| !deprecation.is_tool())
                .map(|deprecation| ModuleDeprecation {
                    method: deprecation.method.to_string(),
                    path: format!("{}/{name}{}", crate::http::API_PREFIX, deprecation.path),
                    since: deprecation.since.to_owned(),
                    sunset: deprecation.sunset.map(str::to_owned),
                    successor: deprecation.successor.map(str::to_owned),
                })
                .collect(),
            languages,
            migrations: migrations
                .into_iter()
                .map(|name| ModuleMigration { name, applied: None })
                .collect(),
            features: module
                .features()
                .into_iter()
                .map(|feature| ModuleFeature {
                    key: feature.key().to_owned(),
                    description: feature.description().to_owned(),
                })
                .collect(),
            limits: module
                .limits()
                .into_iter()
                .map(|limit| {
                    let kind = limit.kind();
                    ModuleLimit {
                        key: limit.key().to_owned(),
                        description: limit.description().to_owned(),
                        kind: kind.as_str().to_owned(),
                        unit: match kind {
                            crate::entitlement::LimitKind::Amount { unit } => Some(unit.to_owned()),
                            _ => None,
                        },
                        period: match kind {
                            crate::entitlement::LimitKind::Metered { .. } => {
                                Some("month".to_owned())
                            }
                            _ => None,
                        },
                        parts: limit.parts().map(str::to_owned),
                    }
                })
                .collect(),
            always_enabled: false,
            requires: Vec::new(),
            stability: module.stability().as_str().to_owned(),
        }
    }
}

/// A feature a plan can turn on or off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ModuleFeature {
    #[schema(example = "content.ai_suggestions")]
    key: String,
    description: String,
}

/// A limit a plan can set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ModuleLimit {
    #[schema(example = "files.storage_bytes")]
    key: String,
    description: String,
    /// `count` (records that exist), `amount` (a quantity in use, see `unit`) or `metered`
    /// (use within a period, back to 0 at its start, see `period`).
    #[schema(example = "amount")]
    kind: String,
    /// Unit of an `amount` limit, e.g. `bytes`.
    unit: Option<String>,
    /// Period of a `metered` limit: `month`.
    period: Option<String>,
    /// When set, a plan can also cap each part separately with the key `<key>.<part>`; this says
    /// what a part is (e.g. `unit type`: `organizations.units.store` caps units of type `store`).
    parts: Option<String>,
}

/// A permission a module declares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ModulePermission {
    #[schema(example = "notes.write")]
    name: String,
    description: String,
    /// How careful a grant needs to be: `read`, `write`, `destructive`, `sensitive_data` or `money`.
    #[schema(example = "write")]
    risk: String,
}

/// An event a module emits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ModuleEvent {
    #[schema(example = "notes.note_created")]
    name: String,
    description: String,
    /// Permission needed to receive this event on `GET /api/v1/kernel/events/stream`; absent when
    /// the event is not streamed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "notes.read")]
    stream_permission: Option<String>,
    /// The event also reaches, without `stream_permission`, the users it names as its audience
    /// (e.g. the customer and the assigned agent of a conversation).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    stream_audience: bool,
}

/// A recurring job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ModuleSchedule {
    name: String,
    /// The job kind it enqueues.
    kind: String,
    /// Seconds between two runs.
    every_seconds: u64,
}

/// An MCP tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ModuleTool {
    /// Full tool name, `<module>.<tool>`.
    #[schema(example = "notes.create_note")]
    name: String,
    description: String,
    /// The caller needs one of these permissions to see and call the tool; empty means any
    /// signed-in caller.
    permissions: Vec<String>,
    /// The tool changes nothing.
    read_only: bool,
    /// The tool deletes or overwrites data that cannot be recovered.
    destructive: bool,
    /// Set when the tool is deprecated: move to `successor` before `sunset`.
    deprecation: Option<ModuleToolDeprecation>,
}

/// An MCP resource: a read-only document an agent reads with `resources/read`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ModuleResource {
    /// Full URI, `autoxide://<module>/<path>`.
    #[schema(example = "autoxide://content/guide")]
    uri: String,
    name: String,
    description: String,
    #[schema(example = "text/markdown")]
    mime_type: String,
    /// The caller needs this permission to see and read the resource; empty means any
    /// signed-in caller.
    permissions: Vec<String>,
}

/// An MCP prompt: a template for a common job, fetched with `prompts/get`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ModulePrompt {
    /// Full prompt name, `<module>.<prompt>`.
    #[schema(example = "content.draft_post")]
    name: String,
    description: String,
    arguments: Vec<ModulePromptArgument>,
    /// Full names of the tools the prompt leads to; only callers who may call all of them see
    /// the prompt.
    tools: Vec<String>,
}

/// An argument of an MCP prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ModulePromptArgument {
    #[schema(example = "topic")]
    name: String,
    description: String,
    required: bool,
}

/// A deprecated MCP tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ModuleToolDeprecation {
    /// Deprecated since this date (`YYYY-MM-DD`, UTC).
    since: String,
    /// The tool goes away on this date, if one is set.
    sunset: Option<String>,
    /// Full name of the tool to use instead, if any.
    successor: Option<String>,
}

/// A deprecated endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ModuleDeprecation {
    #[schema(example = "GET")]
    method: String,
    /// Full path, e.g. `/api/v1/notes/{id}`.
    path: String,
    /// Deprecated since this date (`YYYY-MM-DD`, UTC).
    since: String,
    /// The endpoint goes away on this date, if one is set.
    sunset: Option<String>,
    /// Endpoint to use instead, if any.
    successor: Option<String>,
}

/// A database migration of a module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ModuleMigration {
    #[schema(example = "m0001_create_notes")]
    name: String,
    /// Whether the database has it applied. `null` in the module list, which does not read
    /// the database; `GET /api/v1/kernel/modules/{name}` fills it.
    applied: Option<bool>,
}

/// What the registry's routes and tools need.
#[derive(Clone)]
pub(crate) struct CatalogState {
    pub(crate) catalog: ModuleCatalog,
    pub(crate) database: Database,
    pub(crate) translations: crate::i18n::Translations,
}

impl ModuleInfo {
    /// Describes permissions and events in `language` (D-264).
    fn localized(mut self, translations: &crate::i18n::Translations, language: Language) -> Self {
        for permission in &mut self.permissions {
            permission.description = translations.describe(&permission.description, language);
        }
        for event in &mut self.events {
            event.description = translations.describe(&event.description, language);
        }
        self
    }
}

/// The registry's routes, mounted under `/api/v1/kernel`.
pub(crate) fn routes(state: CatalogState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_modules))
        .routes(routes!(get_module))
        .with_state(Arc::new(state))
}

/// List the installed modules.
///
/// Requires `kernel.modules.read`. Every module the server runs, in start order, with what
/// each declares. Modules are compiled into the server: installing or removing one means
/// rebuilding it, so this list does not change while the server runs. Migration `applied`
/// is `null` here; get one module to read it.
#[utoipa::path(
    get,
    path = "/modules",
    extensions(("x-permissions" = json!(["kernel.modules.read"]))),
    operation_id = "list_modules",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every installed module, in start order.", body = Data<Vec<ModuleInfo>>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.modules.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_modules(
    State(state): State<Arc<CatalogState>>,
    principal: Principal,
    language: Language,
) -> Result<Data<Vec<ModuleInfo>>, Error> {
    principal.require(&MODULES_READ)?;
    let modules = state.catalog.list()?;
    let data = modules.into_iter().map(|info| info.localized(&state.translations, language));
    Ok(Data { data: data.collect() })
}

/// Get one installed module.
///
/// Requires `kernel.modules.read`. The same as an item of the module list, plus whether
/// each migration is applied to the database.
#[utoipa::path(
    get,
    path = "/modules/{name}",
    extensions(("x-permissions" = json!(["kernel.modules.read"]))),
    operation_id = "get_module",
    security(("bearer" = [])),
    params(("name" = String, Path, description = "Module name, e.g. `notes`.")),
    responses(
        (status = 200, description = "The module, with migration status.", body = Data<ModuleInfo>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.modules.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such module; code `kernel.module_not_found`.", body = Problem),
    )
)]
async fn get_module(
    State(state): State<Arc<CatalogState>>,
    principal: Principal,
    Path(name): Path<String>,
    language: Language,
) -> Result<Data<ModuleInfo>, Error> {
    principal.require(&MODULES_READ)?;
    let info = state.catalog.get(&state.database, &name).await?;
    Ok(Data { data: info.localized(&state.translations, language) })
}

// Đầu vào của tool `get_module`.
/// Which module to describe.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModuleNameInput {
    /// Module name, e.g. `notes`.
    #[validate(length(min = 1, max = 64))]
    name: String,
}

/// The registry's MCP resource: `autoxide://kernel/modules`, with the same content as `GET /kernel/modules`.
pub(crate) fn resources(state: CatalogState) -> Vec<crate::mcp::McpResource> {
    vec![
        crate::mcp::McpResource::new(
            "modules",
            "Installed modules",
            "Every module installed in this server, in start order, with what each declares: \
             permissions, events, MCP tools, resources and prompts. The same list as \
             `GET /api/v1/kernel/modules`.",
            "application/json",
            move |_tenant, _principal| {
                let catalog = state.catalog.clone();
                async move {
                    let modules = catalog.list()?;
                    serde_json::to_string_pretty(&Data { data: modules }).map_err(Error::internal)
                }
            },
        )
        .requires(MODULES_READ),
    ]
}

/// The registry's MCP tools — they call exactly the functions REST uses.
pub(crate) fn tools(state: CatalogState) -> Vec<Tool> {
    let list = state.clone();
    let get = state;
    vec![
        Tool::new(
            "list_modules",
            "List the modules installed in this server, in start order, with the permissions, \
             events, jobs, schedules, MCP tools, deprecated endpoints and migrations each \
             declares. Use it to learn what this server can do.",
            move |_tenant: crate::tenancy::TenantContext, _principal: Principal, _: NoInput| {
                let catalog = list.catalog.clone();
                async move { catalog.list() }
            },
        )
        .requires(MODULES_READ)
        .read_only(),
        Tool::new(
            "get_module",
            "Describe one installed module, like an item of `kernel.list_modules`, plus whether \
             each of its migrations is applied to the database.",
            move |_tenant: crate::tenancy::TenantContext,
                  _principal: Principal,
                  input: ModuleNameInput| {
                let state = get.clone();
                async move { state.catalog.get(&state.database, &input.name).await }
            },
        )
        .requires(MODULES_READ)
        .read_only(),
    ]
}
