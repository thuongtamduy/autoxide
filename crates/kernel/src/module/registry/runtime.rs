//! The part of the registry that collects every module's declarations for the running program: deprecated
//! endpoints, MCP tools, resources and prompts, worker, event dispatcher, tasks, routes, migrator... (split
//! out of `mod.rs` by responsibility, code moved only).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;

use crate::event::{Dispatcher, EventCatalog, Subscription};
use crate::i18n::Translations;
use crate::job::{JobHandler, Worker};
use crate::migration::{MigrationError, Migrator, ModuleMigrations};

use super::{ModuleError, Modules};

impl Modules {
    /// Deprecated endpoints of every module: `(module name, declaration)`. Does not include deprecated tools
    /// (they go with the tool, see [`tools`](Self::tools)).
    pub fn deprecations(&self) -> Vec<(&'static str, crate::http::Deprecation)> {
        self.modules
            .iter()
            .flat_map(|module| {
                let name = module.name();
                module
                    .deprecations()
                    .into_iter()
                    .filter(|deprecation| !deprecation.is_tool())
                    .map(move |deprecation| (name, deprecation))
            })
            .collect()
    }

    /// MCP tools of every module, full name `<module>.<name>`. Call after
    /// [`start`](Self::start). A wrong declaration (a badly formed or duplicate name, an input that is not an
    /// object, a permission nobody declares) is a startup error.
    pub fn tools(&self) -> Result<Vec<crate::mcp::Tool>, ModuleError> {
        let mut tools = Vec::new();
        let mut problems = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for module in &self.modules {
            let name = module.name();
            let declared = module.tools();
            let short: Vec<String> = declared.iter().map(|tool| tool.name().to_owned()).collect();
            let deprecated: Vec<crate::http::Deprecation> = module
                .deprecations()
                .into_iter()
                .filter(|deprecation| deprecation.is_tool())
                .collect();
            // Tool deprecated và tool dùng thay phải có trong `tools()` của chính module.
            for deprecation in &deprecated {
                for (role, tool) in
                    [("deprecation", deprecation.tool), ("successor", deprecation.successor)]
                {
                    if let Some(tool) = tool
                        && !short.iter().any(|known| known == tool)
                    {
                        problems.push(format!(
                            "module `{name}` declares a tool {role} `{tool}`, but it has no such \
                             tool in Module::tools"
                        ));
                    }
                }
            }
            for tool in declared {
                let experimental = module.stability() == crate::module::Stability::Experimental;
                let mut tool = tool.qualify(name).experimental(experimental);
                if let Some(deprecation) = deprecated.iter().find(|deprecation| {
                    deprecation.tool.is_some_and(|short| tool.name() == format!("{name}.{short}"))
                }) {
                    tool = tool.deprecated(crate::mcp::ToolDeprecation {
                        since: deprecation.since.to_owned(),
                        sunset: deprecation.sunset.map(str::to_owned),
                        successor: deprecation
                            .successor
                            .map(|successor| format!("{name}.{successor}")),
                    });
                }
                problems.extend(tool.problems(name));
                for permission in tool.permission().into_iter().chain(tool.alternative()) {
                    if !self.permissions.contains(permission.name()) {
                        problems.push(format!(
                            "tool `{}` requires permission `{}`, which no module declares",
                            tool.name(),
                            permission.name()
                        ));
                    }
                }
                if !seen.insert(tool.name().to_owned()) {
                    problems.push(format!("tool `{}` is declared twice", tool.name()));
                }
                tools.push(tool);
            }
        }
        if problems.is_empty() { Ok(tools) } else { Err(ModuleError(problems.join("; "))) }
    }

    /// MCP resources of every module, full URI `autoxide://<module>/<path>` (step 40 P2). Call after
    /// [`start`](Self::start). A wrong declaration (a badly formed or duplicate path, a missing permission
    /// declaration, a permission nobody declares) is a startup error.
    pub fn mcp_resources(&self) -> Result<Vec<crate::mcp::McpResource>, ModuleError> {
        let mut resources = Vec::new();
        let mut problems = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for module in &self.modules {
            let name = module.name();
            for resource in module.mcp_resources() {
                let resource = resource.qualify(name);
                problems.extend(resource.problems(name));
                if let Some(permission) = resource.permission()
                    && !self.permissions.contains(permission.name())
                {
                    problems.push(format!(
                        "resource `{}` requires permission `{}`, which no module declares",
                        resource.uri(),
                        permission.name()
                    ));
                }
                if !seen.insert(resource.uri().to_owned()) {
                    problems.push(format!("resource `{}` is declared twice", resource.uri()));
                }
                resources.push(resource);
            }
        }
        if problems.is_empty() { Ok(resources) } else { Err(ModuleError(problems.join("; "))) }
    }

    /// MCP prompts of every module, full name `<module>.<name>` (step 40 P2). Call after
    /// [`start`](Self::start). A wrong declaration (a badly formed or duplicate name, `uses` pointing to a
    /// tool not present in the same module's `Module::tools`) is a startup error.
    pub fn prompts(&self) -> Result<Vec<crate::mcp::Prompt>, ModuleError> {
        let mut prompts = Vec::new();
        let mut problems = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for module in &self.modules {
            let name = module.name();
            let declared = module.prompts();
            if declared.is_empty() {
                continue;
            }
            let tools = module.tools();
            for prompt in declared {
                let mut used = Vec::new();
                for short in prompt.uses_short() {
                    match tools.iter().find(|tool| tool.name() == *short) {
                        Some(tool) => used.push(tool.clone().qualify(name)),
                        None => problems.push(format!(
                            "prompt `{name}.{}` uses tool `{short}`, but module `{name}` has no \
                             such tool in Module::tools",
                            prompt.name()
                        )),
                    }
                }
                let prompt = prompt.qualify(name, used);
                problems.extend(prompt.problems(name));
                if !seen.insert(prompt.name().to_owned()) {
                    problems.push(format!("prompt `{}` is declared twice", prompt.name()));
                }
                prompts.push(prompt);
            }
        }
        if problems.is_empty() { Ok(prompts) } else { Err(ModuleError(problems.join("; "))) }
    }

    /// Names of the experimental-tier modules that are running - for the HTTP layer to mark their operations.
    pub fn experimental(&self) -> Vec<&'static str> {
        self.modules
            .iter()
            .filter(|module| module.stability() == crate::module::Stability::Experimental)
            .map(|module| module.name())
            .collect()
    }

    /// Name and one-sentence description (`Module::description`) of every module, in registration order - for
    /// `/llms.txt`.
    pub fn descriptions(&self) -> Vec<(&'static str, &'static str)> {
        self.modules.iter().map(|module| (module.name(), module.description())).collect()
    }

    /// Control-plane tables of every module (including `kernel`), in registration order.
    pub fn control_plane_tables(&self) -> Vec<&'static str> {
        self.modules.iter().flat_map(|module| module.control_plane_tables()).collect()
    }

    /// Request body limits specific to every module: `(module name, declaration)`.
    pub fn body_limits(&self) -> Vec<(&'static str, crate::http::BodyLimit)> {
        self.modules
            .iter()
            .flat_map(|module| {
                let name = module.name();
                module.body_limits().into_iter().map(move |limit| (name, limit))
            })
            .collect()
    }

    /// Error message translations of the kernel and every module.
    pub fn translations(&self) -> &Translations {
        &self.translations
    }

    /// Every kind of event of the registered modules.
    pub fn events(&self) -> &EventCatalog {
        &self.events
    }

    /// A worker that runs the jobs of every module on every database of the database registry, `concurrency`
    /// jobs at once per module. Call after [`start`](Self::start). A wrong declaration (a job kind not
    /// belonging to the declaring module, two runners for one kind, a schedule for a job kind nobody runs) is
    /// a startup error.
    pub fn worker(&self, concurrency: u64) -> Result<Worker, ModuleError> {
        let databases = self.started()?;
        let mut handlers: HashMap<&'static str, Arc<dyn JobHandler>> = HashMap::new();
        let mut timeouts: HashMap<&'static str, std::time::Duration> = HashMap::new();
        let mut modules = Vec::new();
        let mut schedules = Vec::new();
        let mut problems = Vec::new();
        for module in &self.modules {
            let name = module.name();
            let registrations = module.jobs();
            if !registrations.is_empty() {
                modules.push(name);
            }
            for registration in registrations {
                let kind = registration.kind();
                if crate::job::module_of(kind) != Some(name) {
                    problems.push(format!(
                        "module `{name}` registers job kind `{kind}`; its job kinds must start \
                         with `{name}.`"
                    ));
                }
                if let Some(limit) = registration.time_limit() {
                    timeouts.insert(kind, limit);
                }
                if handlers.insert(kind, registration.handler()).is_some() {
                    problems.push(format!("job kind `{kind}` has two handlers"));
                }
            }
            for schedule in module.schedules() {
                if crate::job::module_of(schedule.name) != Some(name)
                    || crate::job::module_of(schedule.kind) != Some(name)
                {
                    problems.push(format!(
                        "schedule `{}` of module `{name}` must be named `{name}.<name>` and run \
                         a job kind of `{name}`",
                        schedule.name
                    ));
                }
                schedules.push(schedule);
            }
        }
        let mut names = BTreeSet::new();
        for schedule in &schedules {
            if !handlers.contains_key(schedule.kind) {
                problems.push(format!(
                    "schedule `{}` runs job kind `{}`, which has no handler",
                    schedule.name, schedule.kind
                ));
            }
            if !names.insert(schedule.name) {
                problems.push(format!("schedule `{}` is declared twice", schedule.name));
            }
        }
        if !problems.is_empty() {
            return Err(ModuleError(problems.join("; ")));
        }
        Ok(Worker::new(databases, modules, handlers, schedules, concurrency)
            .with_sync(crate::sync::logs(&self.modules, self.sync_keep))
            .with_timeouts(timeouts)
            .with_entitlements(self.entitlements.clone()))
    }

    /// The backlog gauge updater (step 39): the job tables and event tables of every module, on every
    /// database of the database registry. Call after [`start`](Self::start).
    pub fn backlog_monitor(&self) -> Result<crate::job::backlog::BacklogMonitor, ModuleError> {
        let databases = self.started()?;
        let (jobs, events) = (self.job_modules(), self.events.modules());
        Ok(crate::job::backlog::BacklogMonitor::new(databases, jobs, events))
    }

    /// Modules that have jobs (and a job table), in registration order.
    pub(super) fn job_modules(&self) -> Vec<&'static str> {
        let with_jobs = self.modules.iter().filter(|module| !module.jobs().is_empty());
        with_jobs.map(|module| module.name()).collect()
    }

    /// The modules that delete or anonymize their data when an account is deleted (they receive
    /// [`USER_DELETED`](crate::privacy::USER_DELETED)): enabled modules through `Module::subscriptions`,
    /// turned-off modules through `Module::dormant` (D-20261125-dormant-modules). Call after
    /// [`start`](Self::start).
    // Cho test của chương trình (crate `autoxide`); không thuộc hợp đồng.
    #[doc(hidden)]
    pub fn user_deleted_receivers(&self) -> Vec<&'static str> {
        let mut receivers: Vec<&'static str> = self
            .modules
            .iter()
            .filter(|module| {
                module
                    .subscriptions()
                    .iter()
                    .any(|subscription| subscription.event() == crate::privacy::USER_DELETED)
            })
            .map(|module| module.name())
            .collect();
        receivers.extend(self.dormant_subscriptions.iter().map(|(module, _)| *module));
        receivers
    }

    /// The event dispatcher: reads the event tables of every module that emits events on every database of
    /// the database registry, delivering to the subscriptions of every module. Call after
    /// [`start`](Self::start). Subscribing to an event nobody declares is a startup error.
    pub fn dispatcher(&self) -> Result<Dispatcher, ModuleError> {
        let databases = self.started()?;
        let mut subscriptions = Vec::new();
        let mut problems = Vec::new();
        for module in &self.modules {
            for subscription in module.subscriptions() {
                let event = subscription.event();
                // `privacy::USER_DELETED` và `moderation::TARGET_*` là hợp đồng của kernel: module
                // dọn dữ liệu / sở hữu đích được đăng ký nhận chúng kể cả khi chưa cài module quản
                // lý tài khoản / xử lý vụ việc nào.
                let known = event == Subscription::ALL
                    || event == crate::privacy::USER_DELETED
                    || event == crate::moderation::TARGET_RESTRICTED
                    || event == crate::moderation::TARGET_RESTORED
                    || self.events.contains(event);
                if !known && subscription.is_optional() {
                    // Module phát event chưa cài: không lỗi, chỉ ghi lại để người vận hành biết.
                    tracing::info!(
                        module = module.name(),
                        event,
                        "optional subscription to an event no installed module declares"
                    );
                } else if !known {
                    problems.push(format!(
                        "module `{}` subscribes to event `{event}`, which no module declares",
                        module.name()
                    ));
                }
                subscriptions.push((module.name(), subscription));
            }
        }
        if !problems.is_empty() {
            return Err(ModuleError(problems.join("; ")));
        }
        // Module đang tắt chỉ nhận event xoá tài khoản (chế độ "nghỉ", bước 70 P9).
        subscriptions.extend(self.dormant_subscriptions.iter().cloned());
        Ok(Dispatcher::new(databases, self.events.modules(), subscriptions, Arc::clone(&self.wake))
            .with_entitlements(self.entitlements.clone()))
    }

    /// Each module's checks with the module name, in registration order (`autoxide doctor`). Call
    /// after [`start`](Self::start).
    pub fn checks(&self) -> Vec<(&'static str, crate::doctor::Check)> {
        self.modules
            .iter()
            .flat_map(|module| module.checks().into_iter().map(|check| (module.name(), check)))
            .collect()
    }

    /// Admin commands of every module, full name `<module>.<command>`, in registration order. Call after
    /// [`start`](Self::start). A badly formed or duplicate name, a missing description or a duplicate
    /// parameter name is an error.
    pub fn tasks(&self) -> Result<Vec<(String, crate::task::Task)>, ModuleError> {
        let mut found = Vec::new();
        let mut problems = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for module in &self.modules {
            for task in module.tasks() {
                let full = format!("{}.{}", module.name(), task.name);
                if !crate::task::valid_name(task.name) {
                    problems.push(format!(
                        "task `{full}`: the name must use lowercase letters, digits and `_`"
                    ));
                }
                if task.description.trim().is_empty() {
                    problems.push(format!("task `{full}` has no description"));
                }
                let mut arguments = std::collections::BTreeSet::new();
                for argument in &task.arguments {
                    if !crate::task::valid_name(argument.name) || !arguments.insert(argument.name) {
                        problems.push(format!(
                            "task `{full}`: argument `{}` is malformed or declared twice",
                            argument.name
                        ));
                    }
                }
                if !seen.insert(full.clone()) {
                    problems.push(format!("task `{full}` is declared twice"));
                }
                found.push((full, task));
            }
        }
        if !problems.is_empty() {
            return Err(ModuleError(problems.join("; ")));
        }
        Ok(found)
    }

    /// Syncable record kinds of every module (step 69 P2): `(module name, declaration)`, in
    /// registration order.
    pub fn sync_resources(&self) -> Vec<(&'static str, crate::sync::SyncResource)> {
        crate::sync::declared(&self.modules)
    }

    /// The path + method pairs a module serves both through `Module::routes` and `Module::endpoints`, as
    /// `"<module>: <METHOD> <path>"`: the two would collide when merged. Called at the end of `start`.
    pub(crate) fn overlapping_routes(&self) -> Vec<String> {
        const METHODS: [&str; 8] =
            ["get", "put", "post", "delete", "options", "head", "patch", "trace"];
        // Cặp (path, method) mà một router khai trong tài liệu OpenAPI của nó (route nào cũng có tài liệu, luật 6).
        let pairs = |router: &OpenApiRouter| {
            let document = serde_json::to_value(&router.get_openapi().paths).unwrap_or_default();
            let mut found = std::collections::BTreeSet::new();
            if let Some(paths) = document.as_object() {
                for (path, item) in paths {
                    for method in METHODS.iter().filter(|method| item.get(**method).is_some()) {
                        found.insert((path.clone(), method.to_uppercase()));
                    }
                }
            }
            found
        };
        let mut overlaps = Vec::new();
        for module in &self.modules {
            let endpoints = module.endpoints().into_router();
            let endpoints = pairs(&endpoints);
            if endpoints.is_empty() {
                continue;
            }
            for (path, method) in pairs(&module.routes()).intersection(&endpoints) {
                overlaps.push(format!("{}: {method} {path}", module.name()));
            }
        }
        overlaps
    }

    /// Routes of each module with the module name, in registration order. Call after [`start`](Self::start).
    /// Besides each module's routes there is also `("me", ...)`: the "mine" endpoint served by the kernel
    /// under `/api/v1/me` (per-user flags, D-207) - `me` is a reserved name, no module uses it.
    pub fn routes(&self) -> Vec<(&'static str, OpenApiRouter)> {
        let mut routes: Vec<(&'static str, OpenApiRouter)> = self
            .modules
            .iter()
            .map(|module| (module.name(), module.routes().merge(module.endpoints().into_router())))
            .collect();
        // Schema bộ lọc (P7): sinh từ tham số của từng endpoint danh sách, gắn vào OpenAPI và giữ
        // lại để kiểm view đã lưu.
        let mut lists = Vec::new();
        for (name, router) in &mut routes {
            lists.extend(crate::ops::views::annotate(name, router.get_openapi_mut()));
        }
        if let Some(flags) = self.user_flags.get() {
            let mine = crate::ops::my_permissions::MyPermissions {
                catalog: self.permissions.clone(),
                translations: self.translations.clone(),
            };
            let schemas = crate::ops::views::schemas(lists);
            self.catalog.lists().fill(schemas.clone());
            let views = crate::ops::views::ViewsState { lists: schemas };
            let me = crate::user_flags::routes(flags)
                .merge(crate::ops::my_permissions::routes(mine))
                .merge(crate::ops::views::routes(views))
                .merge(crate::ops::pins::routes(crate::ops::pins::PinsState {
                    catalog: self.catalog.clone(),
                    translations: self.translations.clone(),
                }))
                .merge(crate::ops::can::routes(crate::ops::can::CanState {
                    permissions: self.permissions.clone(),
                    entitlements: self.entitlements.clone(),
                }))
                .merge(crate::ops::work_states::routes(crate::ops::work_states::WorkStatesState {
                    sources: self.catalog.work_sources(),
                    entitlements: self.entitlements.clone(),
                }));
            routes.push((crate::user_flags::ME, me));
        }
        routes
    }

    /// The migrator that runs the migrations of every module. Manages placements on `registry` (the
    /// `autoxide placement` command, `migrate --all`, the periodic check of `serve`): puts the program's root
    /// key into the registry (to open encrypted placement URLs) and uses the migrations of every module.
    pub fn placements(
        &self,
        registry: &crate::tenancy::DatabaseRegistry,
    ) -> Result<crate::tenancy::Placements, MigrationError> {
        registry.use_secret_key(self.keys.key_for(registry.control()));
        Ok(crate::tenancy::Placements::new(registry.clone(), self.migrator()?))
    }

    pub fn migrator(&self) -> Result<Migrator, MigrationError> {
        let mut modules: Vec<ModuleMigrations> = self
            .modules
            .iter()
            .map(|module| {
                ModuleMigrations::new(module.name(), module.migrations())
                    .with_upgrades(module.upgrades())
            })
            .collect();
        // Module đang tắt (chế độ "nghỉ"): migration chỉ áp ở database đã có bảng của nó, để phần xuất / xoá dữ
        // liệu cá nhân chạy trên schema mới sau nâng cấp; database mới không bao giờ có bảng của nó.
        modules.extend(self.dormant.iter().map(|module| {
            ModuleMigrations::new(module.name(), module.migrations())
                .with_upgrades(module.upgrades())
                .where_installed()
        }));
        Ok(Migrator::new(modules)?.with_keys(self.keys.clone()))
    }
}
