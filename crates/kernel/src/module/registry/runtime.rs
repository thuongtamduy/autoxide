//! Phần của bộ đăng ký gom khai báo của mọi module cho chương trình chạy: endpoint deprecated, tool,
//! resource và prompt MCP, worker, bộ phát event, task, route, migrator… (tách khỏi `mod.rs` theo
//! trách nhiệm, chỉ di chuyển code).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;

use crate::event::{Dispatcher, EventCatalog, Subscription};
use crate::i18n::Translations;
use crate::job::{JobHandler, Worker};
use crate::migration::{MigrationError, Migrator, ModuleMigrations};

use super::{ModuleError, Modules};

impl Modules {
    /// Endpoint deprecated của mọi module: `(tên module, khai báo)`. Không gồm tool deprecated
    /// (chúng đi theo tool, xem [`tools`](Self::tools)).
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

    /// Tool MCP của mọi module, tên đầy đủ `<module>.<tên>`. Gọi sau
    /// [`start`](Self::start). Khai báo sai (tên sai dạng hay trùng, đầu vào không phải
    /// object, permission không ai khai) là lỗi khởi động.
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

    /// Resource MCP của mọi module, URI đầy đủ `autoxide://<module>/<path>` (bước 40 P2). Gọi sau
    /// [`start`](Self::start). Khai báo sai (path sai dạng hay trùng, thiếu khai quyền, permission
    /// không ai khai) là lỗi khởi động.
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

    /// Prompt MCP của mọi module, tên đầy đủ `<module>.<tên>` (bước 40 P2). Gọi sau
    /// [`start`](Self::start). Khai báo sai (tên sai dạng hay trùng, `uses` trỏ tới tool không có
    /// trong `Module::tools` của cùng module) là lỗi khởi động.
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

    /// Tên các module hạng thử nghiệm đang chạy — cho lớp HTTP đánh dấu operation của chúng.
    pub fn experimental(&self) -> Vec<&'static str> {
        self.modules
            .iter()
            .filter(|module| module.stability() == crate::module::Stability::Experimental)
            .map(|module| module.name())
            .collect()
    }

    /// Tên và mô tả một câu (`Module::description`) của mọi module, theo thứ tự đăng ký — cho
    /// `/llms.txt`.
    pub fn descriptions(&self) -> Vec<(&'static str, &'static str)> {
        self.modules.iter().map(|module| (module.name(), module.description())).collect()
    }

    /// Bảng mặt điều khiển của mọi module (kể cả `kernel`), theo thứ tự đăng ký.
    pub fn control_plane_tables(&self) -> Vec<&'static str> {
        self.modules.iter().flat_map(|module| module.control_plane_tables()).collect()
    }

    /// Giới hạn thân request riêng của mọi module: `(tên module, khai báo)`.
    pub fn body_limits(&self) -> Vec<(&'static str, crate::http::BodyLimit)> {
        self.modules
            .iter()
            .flat_map(|module| {
                let name = module.name();
                module.body_limits().into_iter().map(move |limit| (name, limit))
            })
            .collect()
    }

    /// Bản dịch thông báo lỗi của kernel và mọi module.
    pub fn translations(&self) -> &Translations {
        &self.translations
    }

    /// Mọi loại event của các module đã đăng ký.
    pub fn events(&self) -> &EventCatalog {
        &self.events
    }

    /// Worker chạy job của mọi module trên mọi database của sổ database, `concurrency` job cùng
    /// lúc mỗi module. Gọi sau [`start`](Self::start). Khai báo sai (loại job không thuộc module
    /// khai, hai bộ chạy cho một loại, lịch cho loại job không ai chạy) là lỗi khởi động.
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

    /// Bộ cập nhật gauge tồn đọng (bước 39): bảng job và bảng event của mọi module, trên mọi
    /// database của sổ database. Gọi sau [`start`](Self::start).
    pub fn backlog_monitor(&self) -> Result<crate::job::backlog::BacklogMonitor, ModuleError> {
        let databases = self.started()?;
        let (jobs, events) = (self.job_modules(), self.events.modules());
        Ok(crate::job::backlog::BacklogMonitor::new(databases, jobs, events))
    }

    /// Module có job (và bảng job), theo thứ tự đăng ký.
    pub(super) fn job_modules(&self) -> Vec<&'static str> {
        let with_jobs = self.modules.iter().filter(|module| !module.jobs().is_empty());
        with_jobs.map(|module| module.name()).collect()
    }

    /// Bộ phát event: đọc bảng event của mọi module phát event trên mọi database của sổ
    /// database, giao cho các đăng ký nhận của mọi module. Gọi sau [`start`](Self::start). Đăng
    /// ký nhận một event không ai khai là lỗi khởi động.
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
        Ok(Dispatcher::new(databases, self.events.modules(), subscriptions, Arc::clone(&self.wake))
            .with_entitlements(self.entitlements.clone()))
    }

    /// Bước kiểm của từng module kèm tên module, theo thứ tự đăng ký (`autoxide doctor`). Gọi
    /// sau [`start`](Self::start).
    pub fn checks(&self) -> Vec<(&'static str, crate::doctor::Check)> {
        self.modules
            .iter()
            .flat_map(|module| module.checks().into_iter().map(|check| (module.name(), check)))
            .collect()
    }

    /// Lệnh quản trị của mọi module, tên đầy đủ `<module>.<lệnh>`, theo thứ tự đăng ký. Gọi sau
    /// [`start`](Self::start). Tên sai dạng, trùng, thiếu mô tả hay tham số trùng tên là lỗi.
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

    /// Loại bản ghi đồng bộ được của mọi module (bước 69 P2): `(tên module, khai báo)`, theo thứ
    /// tự đăng ký.
    pub fn sync_resources(&self) -> Vec<(&'static str, crate::sync::SyncResource)> {
        crate::sync::declared(&self.modules)
    }

    /// Route của từng module kèm tên module, theo thứ tự đăng ký. Gọi sau
    /// [`start`](Self::start).
    /// Ngoài route của từng module còn có `("me", …)`: endpoint "của tôi" do kernel phục vụ dưới
    /// `/api/v1/me` (cờ theo người dùng, D-207) — `me` là tên dành riêng, không module nào dùng.
    pub fn routes(&self) -> Vec<(&'static str, OpenApiRouter)> {
        let mut routes: Vec<(&'static str, OpenApiRouter)> =
            self.modules.iter().map(|module| (module.name(), module.routes())).collect();
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

    /// Migrator chạy migration của mọi module.
    /// Quản lý placement trên `registry` (lệnh `autoxide placement`, `migrate --all`, kiểm định kỳ
    /// của `serve`): đưa khoá gốc của chương trình vào sổ (mở URL placement đã mã hoá) và dùng
    /// migration của mọi module.
    pub fn placements(
        &self,
        registry: &crate::tenancy::DatabaseRegistry,
    ) -> Result<crate::tenancy::Placements, MigrationError> {
        registry.use_secret_key(self.keys.key_for(registry.control()));
        Ok(crate::tenancy::Placements::new(registry.clone(), self.migrator()?))
    }

    pub fn migrator(&self) -> Result<Migrator, MigrationError> {
        let modules = self
            .modules
            .iter()
            .map(|module| {
                ModuleMigrations::new(module.name(), module.migrations())
                    .with_upgrades(module.upgrades())
            })
            .collect();
        Ok(Migrator::new(modules)?.with_keys(self.keys.clone()))
    }
}
