//! Bộ đăng ký module: nhận danh sách module của binary, kiểm tên, gom permission, event,
//! bản dịch, cấu hình, migration, rồi khởi động từng module theo thứ tự đăng ký. Hợp đồng
//! với module (trait `Module`, `StartContext`) ở `module.rs`.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;

use crate::access::PermissionCatalog;
use crate::config::{ConfigReader, Invalid};
use crate::database::Database;
use crate::event::{Dispatcher, EventCatalog, Subscription};
use crate::i18n::Translations;
use crate::job::{JobHandler, Worker};
use crate::migration::{MigrationError, Migrator, ModuleMigrations};
use crate::privacy::PersonalDataExport;

use super::{Apis, Module, StartContext, module_name_problems};

/// Lỗi trong việc đăng ký module.
#[derive(Debug)]
pub struct ModuleError(String);

impl ModuleError {
    /// Nội dung lỗi, không kèm tiền tố `modules:`.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "modules: {}", self.0)
    }
}

impl std::error::Error for ModuleError {}

/// Tập module đã đăng ký, theo thứ tự đăng ký.
pub struct Modules {
    // `Box<dyn Module>`: mỗi module là một kiểu khác nhau; `dyn` cho phép để chung
    // một danh sách và gọi qua trait.
    modules: Vec<Box<dyn Module>>,
    permissions: PermissionCatalog,
    events: EventCatalog,
    translations: Translations,
    /// Chuông đánh thức bộ phát event, dùng chung cho mọi `Outbox`.
    wake: Arc<tokio::sync::Notify>,
    /// Các trait module đã công bố; có sau [`start`](Self::start).
    apis: Apis,
    /// Phần dữ liệu cá nhân các module đăng ký lúc khởi động.
    personal_data: PersonalDataExport,
    /// Loại đối tượng các module đăng ký lúc khởi động.
    resources: crate::resource::ResourceDirectory,
    /// Event vòng đời các module khai (`Module::resource_events`), đã kiểm.
    resource_events: Vec<crate::resource::ResourceEvent>,
    /// Bộ hỏi tài khoản đã xác thực (một module đăng ký lúc khởi động, D-202).
    verification: crate::access::PostingVerification,
    /// Sổ module đã cài (module `kernel` phục vụ); điền ở cuối [`start`](Self::start).
    catalog: crate::module_catalog::ModuleCatalog,
    /// Cờ theo người dùng của module `kernel`, phục vụ dưới `/api/v1/me` (D-207).
    user_flags: crate::user_flags::Handle,
    /// Cấu hình khoá gốc (module `kernel` điền lúc `configure`), cho upgrade dữ liệu.
    keys: crate::secret::KeyConfig,
    /// Sổ database (bước 33); có sau [`start`](Self::start).
    tenancy: Option<crate::tenancy::DatabaseRegistry>,
}

impl Modules {
    /// Kiểm tên rồi nhận các module. Tên sai là lỗi của người viết module, báo
    /// ngay lúc khởi động.
    ///
    /// Hai module nội bộ luôn được thêm vào đầu danh sách: `kernel` (bảng dùng chung của
    /// chương trình, vd khoá ký) và `antispam` (chống spam, D-119). Module nào cũng `require`
    /// được thứ chúng công bố.
    pub fn new(modules: Vec<Box<dyn Module>>) -> Result<Self, ModuleError> {
        let mut problems = Vec::new();
        // `me`: tiền tố `/api/v1/me` của các endpoint "của tôi" do kernel phục vụ.
        for reserved in [crate::builtin::NAME, crate::antispam::module::NAME, crate::user_flags::ME]
        {
            if modules.iter().any(|module| module.name() == reserved) {
                problems.push(format!("module name `{reserved}` is reserved for the kernel"));
            }
        }
        let catalog = crate::module_catalog::ModuleCatalog::default();
        let user_flags = crate::user_flags::Handle::default();
        let keys = crate::secret::KeyConfig::default();
        let settings = crate::tenancy::settings::Catalog::default();
        let mut all: Vec<Box<dyn Module>> = vec![
            Box::new(crate::builtin::KernelModule::new(
                catalog.clone(),
                user_flags.clone(),
                keys.clone(),
                settings.clone(),
            )),
            Box::new(crate::antispam::module::AntispamModule::default()),
        ];
        all.extend(modules);
        let modules = all;
        let names: Vec<&str> = modules.iter().map(|module| module.name()).collect();
        problems.extend(module_name_problems(&names));
        let declared = modules.iter().map(|module| (module.name(), module.permissions())).collect();
        let permissions = PermissionCatalog::new(declared).unwrap_or_else(|errors| {
            problems.extend(errors);
            PermissionCatalog::default()
        });
        let declared = modules.iter().map(|module| (module.name(), module.events())).collect();
        let events = EventCatalog::new(declared).unwrap_or_else(|errors| {
            problems.extend(errors);
            EventCatalog::default()
        });
        let resource_events = checked_resource_events(&modules, &events, &mut problems);
        let mut seen = Vec::new();
        let mut declared_settings = Vec::new();
        for module in &modules {
            let declared = module.tenant_settings();
            problems.extend(crate::tenancy::settings::declaration_problems(
                module.name(),
                &declared,
                &mut seen,
            ));
            declared_settings.extend(declared);
        }
        settings.fill(declared_settings);
        for module in &modules {
            for deprecation in module.deprecations() {
                problems.extend(deprecation.problems(module.name()));
            }
        }
        let declared =
            modules.iter().map(|module| (module.name(), module.translations())).collect();
        let translations = Translations::new(declared).unwrap_or_else(|errors| {
            problems.extend(errors);
            Translations::default()
        });
        if problems.is_empty() {
            Ok(Modules {
                modules,
                permissions,
                events,
                translations,
                wake: Arc::new(tokio::sync::Notify::new()),
                personal_data: PersonalDataExport::default(),
                resources: crate::resource::ResourceDirectory::default(),
                resource_events,
                verification: crate::access::PostingVerification::default(),
                apis: Apis::default(),
                catalog,
                user_flags,
                keys,
                tenancy: None,
            })
        } else {
            Err(ModuleError(problems.join("; ")))
        }
    }

    /// Tên các module, theo thứ tự đăng ký.
    pub fn names(&self) -> Vec<&'static str> {
        self.modules.iter().map(|module| module.name()).collect()
    }

    pub fn is_empty(&self) -> bool {
        // Hai module nội bộ `kernel` và `antispam` luôn có, không tính.
        self.modules.len() <= 2
    }

    /// Cho từng module đọc cấu hình của nó. Module nào sai cũng không chặn module
    /// khác đọc — để mọi lỗi được báo cùng lúc.
    pub fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        let mut result = Ok(());
        for module in &mut self.modules {
            if module.configure(reader).is_err() {
                result = Err(Invalid);
            }
        }
        result
    }

    /// Khởi động từng module theo thứ tự đăng ký, chế độ một khách trên `database`. Module nào
    /// hỏng thì dừng và báo tên module đó.
    pub fn start(&mut self, database: &Database) -> Result<(), ModuleError> {
        self.start_in(&crate::tenancy::DatabaseRegistry::single(database.clone()))
    }

    /// Như [`start`](Self::start), với sổ database `registry` (chế độ của bản cài, các
    /// placement): database của chương trình là database điều khiển của nó.
    pub fn start_in(
        &mut self,
        registry: &crate::tenancy::DatabaseRegistry,
    ) -> Result<(), ModuleError> {
        let database = registry.control();
        let mut apis = Apis::default();
        for module in &mut self.modules {
            let name = module.name();
            let mut context = StartContext {
                module: name,
                database,
                permissions: &self.permissions,
                events: &self.events,
                translations: &self.translations,
                wake: &self.wake,
                apis: &mut apis,
                personal_data: &self.personal_data,
                resources: &self.resources,
                resource_events: &self.resource_events,
                verification: &self.verification,
                tenancy: registry,
            };
            module.start(&mut context).map_err(|error| {
                ModuleError(format!("module `{name}` failed to start: {error}"))
            })?;
        }
        self.apis = apis;
        self.tenancy = Some(registry.clone());
        // Sau khi MỌI module đã khởi động: tool, job, lịch của module chỉ có từ lúc này.
        self.catalog.fill(&self.modules);
        Ok(())
    }

    /// Sổ database đã dùng lúc khởi động; `None` trước [`start`](Self::start).
    pub fn tenancy(&self) -> Option<crate::tenancy::DatabaseRegistry> {
        self.tenancy.clone()
    }

    /// Sổ database, có sau [`start`](Self::start).
    fn started(&self) -> Result<crate::tenancy::DatabaseRegistry, ModuleError> {
        self.tenancy.clone().ok_or_else(|| {
            ModuleError("start the modules (`Modules::start`) before building the worker".into())
        })
    }

    /// Trait `T` mà một module đã công bố lúc khởi động, nếu có. Dùng ở binary để
    /// lấy những trait core cần (vd `dyn Authenticator`). Gọi sau [`start`](Self::start).
    pub fn api<T>(&self) -> Option<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.apis.get()
    }

    /// Sổ dữ liệu cá nhân các module đã đăng ký lúc [`start`](Self::start).
    pub fn personal_data(&self) -> PersonalDataExport {
        self.personal_data.clone()
    }

    /// Sổ đối tượng các module đã đăng ký (gọi sau [`start`](Self::start)).
    pub fn resources(&self) -> crate::resource::ResourceDirectory {
        self.resources.clone()
    }

    /// Mọi permission của các module đã đăng ký.
    pub fn permissions(&self) -> &PermissionCatalog {
        &self.permissions
    }

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
                let mut tool = tool.qualify(name);
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
        Ok(Worker::new(databases, modules, handlers, schedules, concurrency))
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
                // `privacy::USER_DELETED` là hợp đồng của kernel: module dọn dữ liệu được
                // đăng ký nhận nó kể cả khi chưa cài module quản lý tài khoản nào.
                let known = event == Subscription::ALL
                    || event == crate::privacy::USER_DELETED
                    || self.events.contains(event);
                if !known {
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
        Ok(Dispatcher::new(databases, self.events.modules(), subscriptions, Arc::clone(&self.wake)))
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

    /// Route của từng module kèm tên module, theo thứ tự đăng ký. Gọi sau
    /// [`start`](Self::start).
    /// Ngoài route của từng module còn có `("me", …)`: endpoint "của tôi" do kernel phục vụ dưới
    /// `/api/v1/me` (cờ theo người dùng, D-207) — `me` là tên dành riêng, không module nào dùng.
    pub fn routes(&self) -> Vec<(&'static str, OpenApiRouter)> {
        let mut routes: Vec<(&'static str, OpenApiRouter)> =
            self.modules.iter().map(|module| (module.name(), module.routes())).collect();
        if let Some(flags) = self.user_flags.get() {
            routes.push((crate::user_flags::ME, crate::user_flags::routes(flags)));
        }
        routes
    }

    /// Migrator chạy migration của mọi module.
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

/// Event vòng đời mọi module khai, sau khi kiểm: event phải do chính module đó khai, loại phải
/// thuộc module đó, một event chỉ khai một lần. Lỗi thêm vào `problems`.
fn checked_resource_events(
    modules: &[Box<dyn Module>],
    events: &EventCatalog,
    problems: &mut Vec<String>,
) -> Vec<crate::resource::ResourceEvent> {
    let mut found: Vec<crate::resource::ResourceEvent> = Vec::new();
    for module in modules {
        let name = module.name();
        let own = |text: &str| {
            text.strip_prefix(name).and_then(|rest| rest.strip_prefix('.')).is_some_and(|rest| {
                !rest.is_empty()
                    && rest
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            })
        };
        for declared in module.resource_events() {
            if !own(declared.kind) {
                problems.push(format!(
                    "module `{name}` declares a lifecycle event for resource type `{}`; it must \
                     be `{name}.<lower_case_name>`",
                    declared.kind
                ));
            }
            if !own(declared.event) || !events.contains(declared.event) {
                problems.push(format!(
                    "module `{name}` declares `{}` as a lifecycle event, but does not declare \
                     that event in `Module::events`",
                    declared.event
                ));
            }
            if found.iter().any(|known| known.event == declared.event) {
                problems.push(format!("lifecycle event `{}` is declared twice", declared.event));
            }
            found.push(declared);
        }
    }
    found
}

impl fmt::Debug for Modules {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Modules").field(&self.names()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigSource, Setting};

    const LIMIT: Setting = Setting::new("AUTOXIDE_NOTES_LIMIT", "Limit.").with_default("10");
    const KEY: Setting = Setting::new("AUTOXIDE_TAGS_KEY", "Key.");

    /// Module không khai gì ngoài tên: dùng mọi cài đặt mặc định của hợp đồng.
    struct Bare(&'static str);

    impl Module for Bare {
        fn name(&self) -> &'static str {
            self.0
        }
    }

    /// Module đọc một biến có mặc định.
    #[derive(Default)]
    struct Notes {
        limit: u32,
    }

    impl Module for Notes {
        fn name(&self) -> &'static str {
            "notes"
        }

        fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
            self.limit = reader.get(&LIMIT)?;
            Ok(())
        }
    }

    /// Module sở hữu đối tượng `shop.item`, khai event vòng đời.
    struct Shop(Vec<crate::resource::ResourceEvent>);

    impl Module for Shop {
        fn name(&self) -> &'static str {
            "shop"
        }

        fn events(&self) -> Vec<crate::event::EventInfo> {
            vec![crate::event::EventInfo::new("shop.item_deleted", "An item was deleted.")]
        }

        fn resource_events(&self) -> Vec<crate::resource::ResourceEvent> {
            self.0.clone()
        }
    }

    #[test]
    fn event_vong_doi_duoc_kiem_luc_khoi_dong() {
        use crate::resource::{ResourceChange, ResourceEvent};
        let good = ResourceEvent::new("shop.item_deleted", "shop.item", ResourceChange::Removed);
        let modules = Modules::new(vec![Box::new(Shop(vec![good]))]).unwrap();
        assert_eq!(modules.resource_events, [good]);
        for bad in [
            ResourceEvent::new("shop.item_gone", "shop.item", ResourceChange::Removed),
            ResourceEvent::new("shop.item_deleted", "media.item", ResourceChange::Removed),
        ] {
            let error = Modules::new(vec![Box::new(Shop(vec![bad]))]).unwrap_err();
            assert!(error.to_string().contains("lifecycle event"), "{error}");
        }
        let error = Modules::new(vec![Box::new(Shop(vec![good, good]))]).unwrap_err();
        assert!(error.to_string().contains("declared twice"), "{error}");
    }

    /// Module đọc một biến bắt buộc.
    struct Tags;

    impl Module for Tags {
        fn name(&self) -> &'static str {
            "tags"
        }

        fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
            reader.get::<String>(&KEY).map(|_| ())
        }
    }

    fn bare(names: &[&'static str]) -> Vec<Box<dyn Module>> {
        names.iter().map(|name| Box::new(Bare(name)) as Box<dyn Module>).collect()
    }

    fn error(names: &[&'static str]) -> String {
        Modules::new(bare(names)).unwrap_err().to_string()
    }

    #[test]
    fn giu_thu_tu_dang_ky() {
        let modules = Modules::new(bare(&["notes", "auth", "tags"])).unwrap();
        assert_eq!(modules.names(), ["kernel", "antispam", "notes", "auth", "tags"]);
        assert!(!modules.is_empty());
        assert_eq!(
            format!("{modules:?}"),
            r#"Modules(["kernel", "antispam", "notes", "auth", "tags"])"#
        );
        assert!(Modules::new(Vec::new()).unwrap().is_empty());
    }

    #[test]
    fn ten_module_sai_dang_bi_tu_choi() {
        for name in ["", "Auth", "1auth", "auth-keys", "auth_", "au th"] {
            let message = error(&[name]);
            assert!(message.contains("must be lowercase letters, digits and `_`"), "{message}");
        }
    }

    #[test]
    fn ten_danh_rieng_cua_database_bi_tu_choi() {
        for name in ["sqlite", "pg"] {
            assert!(error(&[name]).contains("is reserved"), "{name}");
        }
        // Chỉ đúng hai tên đó; tên chứa chúng thì không sao.
        assert!(Modules::new(bare(&["pgqueue", "sqlitex"])).is_ok());
    }

    #[test]
    fn dang_ky_trung_hoac_ten_long_nhau_bi_tu_choi() {
        assert!(error(&["auth", "auth"]).contains("module `auth` is registered twice"));
        assert!(error(&["auth", "auth_keys"]).contains("`auth` and `auth_keys` are ambiguous"));
        // `auth` và `authz`: tiền tố bảng là `auth_` và `authz_`, không đụng nhau.
        assert!(Modules::new(bare(&["auth", "authz"])).is_ok());
    }

    #[test]
    fn moi_loi_ten_duoc_bao_cung_luc() {
        let message = error(&["Auth", "notes", "notes"]);
        assert!(message.starts_with("modules: "), "{message}");
        assert!(message.contains("`Auth`") && message.contains("registered twice"), "{message}");
    }

    #[test]
    fn tung_module_doc_cau_hinh_cua_minh() {
        let source = ConfigSource::from_pairs([("AUTOXIDE_NOTES_LIMIT", "25")]);
        let mut reader = ConfigReader::new(&source);
        let mut notes = Notes::default();

        assert_eq!(notes.configure(&mut reader), Ok(()));

        assert_eq!(notes.limit, 25);
    }

    #[test]
    fn loi_cau_hinh_cua_moi_module_duoc_gom_chung() {
        // `notes` nhận giá trị sai, `tags` thiếu biến bắt buộc: cả hai phải có mặt
        // trong cùng một thông báo, dù `notes` đứng trước và đã hỏng.
        let source = ConfigSource::from_pairs([("AUTOXIDE_NOTES_LIMIT", "many")]);
        let mut reader = ConfigReader::new(&source);
        let mut modules =
            Modules::new(vec![Box::new(Notes::default()), Box::new(Tags), Box::new(Bare("auth"))])
                .unwrap();

        let configured = modules.configure(&mut reader);

        assert_eq!(configured, Err(Invalid));
        let message = reader.finish(configured).unwrap_err().to_string();
        assert!(message.contains("invalid configuration (2 problems):"), "{message}");
        assert!(message.contains("AUTOXIDE_NOTES_LIMIT:"), "{message}");
        assert!(message.contains("AUTOXIDE_TAGS_KEY: is required but not set"), "{message}");
    }

    #[test]
    fn module_khong_khai_gi_thi_khong_co_migration_va_cau_hinh() {
        let source = ConfigSource::default();
        let mut reader = ConfigReader::new(&source);
        let mut modules = Modules::new(bare(&["auth"])).unwrap();

        assert_eq!(modules.configure(&mut reader), Ok(()));
        // Chỉ biến của hai module nội bộ `kernel` và `antispam`.
        let names: Vec<&str> = reader.values().iter().map(|value| value.setting.name).collect();
        let internal = |name: &&str| {
            [
                "AUTOXIDE_SECRET_KEY",
                "AUTOXIDE_SECRET_KEY_PREVIOUS",
                "AUTOXIDE_HTTP_PUBLIC_URL",
                "AUTOXIDE_USER_FLAGS_MAX",
            ]
            .contains(name)
                || name.starts_with("AUTOXIDE_ANTISPAM_")
        };
        assert!(names.iter().all(internal), "{names:?}");
        assert!(modules.migrator().is_ok());
    }
}
