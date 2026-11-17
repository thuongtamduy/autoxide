//! Sổ module đã cài: cho người quản trị (và AI agent) xem chương trình đang chạy gồm những
//! module nào và mỗi module khai gì — permission, event, job, lịch, tool MCP, endpoint
//! deprecated, migration. Đây là phần API mà một màn hình "quản lý module" ở dự án giao diện
//! dùng (D-155).
//!
//! Sổ chỉ ĐỌC. Module đăng ký lúc biên dịch (`crates/app/src/modules.rs`), nên không có bật /
//! tắt module lúc chạy.
//!
//! Cách dựng: [`Modules::new`](crate::module::Modules::new) tạo một [`ModuleCatalog`] rỗng và
//! đưa một bản cho module nội bộ `kernel`; [`Modules::start`](crate::module::Modules::start)
//! điền sổ sau khi MỌI module đã khởi động (tool, job, lịch chỉ có sau `start`). Route của
//! `kernel` đọc sổ lúc phục vụ request:
//!
//! - `GET /api/v1/kernel/modules` — mọi module, theo thứ tự đăng ký; không chạm database;
//! - `GET /api/v1/kernel/modules/{name}` — một module, kèm trạng thái từng migration (đọc
//!   bảng `<tên>_migrations` của riêng module đó).
//!
//! Cả hai cần permission `kernel.modules.read`. Tool MCP `kernel.list_modules` và
//! `kernel.get_module` gọi cùng hàm.

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

/// Permission xem sổ module.
pub(crate) const MODULES_READ: Permission = Permission::new(
    "kernel.modules.read",
    "See the installed modules and what each declares: permissions, events, jobs, schedules, \
     MCP tools, deprecated endpoints and migration status.",
);

/// Hai module nội bộ kernel luôn cài (không do binary đăng ký).
const INTERNAL: [&str; 2] = [crate::builtin::NAME, crate::antispam::module::NAME];

/// Sổ module, dùng chung giữa `Modules` (bên điền) và module `kernel` (bên đọc).
//
// `Arc<OnceLock<…>>`: hai bên giữ cùng một ô; ô được ghi đúng một lần (sau `start`) rồi chỉ
// đọc, nên đọc từ nhiều thread không cần lock.
#[derive(Clone, Default)]
pub(crate) struct ModuleCatalog {
    entries: Arc<OnceLock<Vec<Entry>>>,
    /// Bước kiểm (`autoxide doctor`) của mọi module, kèm tên module: cảnh báo vận hành
    /// (`GET /kernel/ops/alerts`) chạy lại chúng.
    checks: Arc<OnceLock<Vec<(&'static str, crate::doctor::Check)>>>,
    /// Sổ nguồn việc của hộp việc (bước 51 P2): module đăng ký lúc khởi động.
    work: crate::ops::work::WorkSources,
    /// Sổ nguồn tìm xuyên phân hệ (bước 51 P3).
    finders: crate::ops::find::FinderSources,
    /// Sổ điều hướng đã gom (bước 51 P1).
    navigation: Arc<OnceLock<crate::module::navigation::NavigationCatalog>>,
}

/// Một module trong sổ: phần khai báo (không đổi lúc chạy) và migrator riêng của nó để đọc
/// trạng thái migration.
struct Entry {
    info: ModuleInfo,
    migrator: Option<Migrator>,
}

impl ModuleCatalog {
    /// Điền sổ từ các module đã khởi động. Gọi một lần; lần sau bị bỏ qua.
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

    pub(crate) fn fill_navigation(&self, navigation: crate::module::navigation::NavigationCatalog) {
        let _ = self.navigation.set(navigation);
    }

    /// Sổ điều hướng (rỗng trước khi mọi module khởi động).
    pub(crate) fn navigation(&self) -> Option<&crate::module::navigation::NavigationCatalog> {
        self.navigation.get()
    }

    /// Bước kiểm của mọi module (rỗng trước khi mọi module khởi động).
    pub(crate) fn checks(&self) -> &[(&'static str, crate::doctor::Check)] {
        self.checks.get().map_or(&[], Vec::as_slice)
    }

    fn entries(&self) -> Result<&[Entry], Error> {
        self.entries.get().map(Vec::as_slice).ok_or_else(|| {
            Error::internal("the module catalog is read before every module has started")
        })
    }

    /// Mọi module, theo thứ tự đăng ký. `applied` của migration để `null`.
    pub(crate) fn list(&self) -> Result<Vec<ModuleInfo>, Error> {
        Ok(self.entries()?.iter().map(|entry| entry.info.clone()).collect())
    }

    /// Một module theo tên, kèm trạng thái migration đọc từ database.
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

/// Những gì route và tool của sổ cần.
#[derive(Clone)]
pub(crate) struct CatalogState {
    pub(crate) catalog: ModuleCatalog,
    pub(crate) database: Database,
    pub(crate) translations: crate::i18n::Translations,
}

impl ModuleInfo {
    /// Mô tả permission và event theo `language` (D-264).
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

/// Route của sổ, gắn dưới `/api/v1/kernel`.
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

/// Tool MCP của sổ — gọi đúng các hàm REST dùng.
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
