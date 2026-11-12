//! Hợp đồng module: những gì một module khai với core, và [`Modules`] — tập
//! module đã đăng ký của chương trình.
//!
//! Một module là một crate trong `modules/<tên>`. Nó mở ra ngoài đúng hai thứ
//! (`cargo xtask check` kiểm):
//!
//! - `pub fn module() -> impl Module` — để binary đăng ký;
//! - `pub mod api` — trait và kiểu dữ liệu cho module khác dùng.
//!
//! Binary đăng ký mọi module ở MỘT chỗ (`crates/app/src/modules.rs`). Core không
//! biết tên module nào: nó chỉ làm việc qua trait [`Module`], nên thêm một module
//! không phải sửa core.
//!
//! Hợp đồng lớn dần theo lộ trình: mỗi phần của core (route, permission, event,
//! job, tool MCP) thêm một hàm vào [`Module`], luôn kèm cài đặt mặc định "không
//! có gì" — module cũ không phải sửa khi hợp đồng thêm hàm mới.

use std::any::{Any, TypeId};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use sea_orm_migration::MigrationTrait;
use utoipa_axum::router::OpenApiRouter;

use crate::access::{Permission, PermissionCatalog};
use crate::config::{ConfigReader, Invalid};
use crate::database::Database;
use crate::error::Error;
use crate::event::{Dispatcher, EventCatalog, EventInfo, Outbox, Subscription};
use crate::i18n::Translations;
use crate::job::{JobHandler, JobQueue, JobRegistration, Schedule, Worker};
use crate::migration::{MigrationError, Migrator, ModuleMigrations};
use crate::privacy::{PersonalData, PersonalDataExport};

/// Những gì một module khai với core.
///
/// ```
/// use autoxide_kernel::config::{ConfigReader, Invalid, Setting};
/// use autoxide_kernel::module::Module;
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
///     fn name(&self) -> &'static str {
///         "notes"
///     }
///
///     fn configure(&mut self, reader: &mut ConfigReader) -> Result<(), Invalid> {
///         self.page_size = reader.get(&PAGE_SIZE)?;
///         Ok(())
///     }
/// }
///
/// // Hàm duy nhất crate của module mở cho binary.
/// pub fn module() -> impl Module {
///     NotesModule::default()
/// }
/// ```
//
// `Send + Sync`: module được dùng từ nhiều thread của máy chủ. Trình biên dịch
// từ chối module chứa thứ không an toàn khi chia sẻ giữa các thread.
pub trait Module: Send + Sync {
    /// Tên module: chữ thường, số và `_`, bắt đầu bằng chữ. Cũng là tiền tố của
    /// mọi bảng (`<tên>_…`) và mọi biến cấu hình (`AUTOXIDE_<TÊN>_…`) của module.
    fn name(&self) -> &'static str;

    /// Các migration của module, theo thứ tự áp dụng. Đã commit thì không sửa —
    /// thêm migration mới vào cuối.
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        Vec::new()
    }

    /// Đọc cấu hình riêng của module và giữ lại trong chính nó. Gọi một lần lúc
    /// khởi động, trước mọi việc khác. Lỗi ghi vào `reader`, được báo chung một
    /// lượt với lỗi cấu hình của kernel và các module khác.
    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        // `let _ =`: tham số không dùng tới trong cài đặt mặc định.
        let _ = reader;
        Ok(())
    }

    /// Khởi động module: dựng service của nó (với database lấy từ `context`),
    /// công bố trait cho module khác dùng ([`StartContext::provide`]) và lấy trait
    /// của module khác ([`StartContext::require`]). Gọi một lần, sau `configure`,
    /// theo thứ tự đăng ký — module phải đăng ký SAU các module nó cần.
    ///
    /// Module giữ những gì đã dựng trong chính nó để [`routes`](Self::routes) dùng.
    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let _ = context;
        Ok(())
    }

    /// Các permission của module. Tên viết `<tên module>.<việc>`; core kiểm lúc
    /// khởi động. Permission đã phát hành là hợp đồng: đổi tên thì role đang có
    /// permission cũ mất quyền.
    fn permissions(&self) -> Vec<Permission> {
        Vec::new()
    }

    /// Các event module phát. Có event thì module phải có bảng `<tên>_events` (thêm
    /// `autoxide_kernel::event::outbox_migration` vào migration) và ghi event bằng
    /// [`StartContext::outbox`].
    fn events(&self) -> Vec<EventInfo> {
        Vec::new()
    }

    /// Các event module muốn nhận (của mọi module, kể cả chính nó). Gọi sau `start`.
    fn subscriptions(&self) -> Vec<Subscription> {
        Vec::new()
    }

    /// Bộ chạy cho các loại job của module (`<tên>.<việc>`). Có job thì module phải có
    /// bảng `<tên>_jobs` (thêm `autoxide_kernel::job::jobs_migration` vào migration).
    /// Gọi sau `start`.
    fn jobs(&self) -> Vec<JobRegistration> {
        Vec::new()
    }

    /// Lịch định kỳ của module: mỗi lịch đưa một job (loại do chính module chạy) vào
    /// hàng đợi sau mỗi khoảng. Gọi sau `start`.
    fn schedules(&self) -> Vec<Schedule> {
        Vec::new()
    }

    /// Endpoint deprecated của module: kernel đánh dấu trong OpenAPI và gắn header
    /// `Deprecation` / `Sunset` / `Link` vào response (D-059).
    fn deprecations(&self) -> Vec<crate::http::Deprecation> {
        Vec::new()
    }

    /// Tool MCP của module (D-068). Gọi sau `start` (tool thường gọi vào service dựng
    /// lúc đó). Tên khai ngắn (`create_note`); kernel thêm tiền tố `<module>.`.
    fn tools(&self) -> Vec<crate::mcp::Tool> {
        Vec::new()
    }

    /// Route cần thân request lớn hơn giới hạn chung (`AUTOXIDE_HTTP_BODY_LIMIT_BYTES`),
    /// vd tải file lên. Gọi sau `configure` (giới hạn có thể lấy từ cấu hình của module).
    fn body_limits(&self) -> Vec<crate::http::BodyLimit> {
        Vec::new()
    }

    /// Bản dịch thông báo lỗi của module: `(ngôn ngữ, nội dung JSON)`, thường là
    /// `vec![("vi", include_str!("../locales/vi.json"))]`. Mỗi `code` lỗi của module
    /// phải có bản dịch (`cargo xtask check` kiểm).
    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }

    /// Các route REST của module, kèm tài liệu OpenAPI của chúng. Gọi sau `start`.
    /// Core gắn chúng dưới `/api/v1/<tên module>`: route khai là `/` và `/{id}`
    /// của module `notes` thành `/api/v1/notes` và `/api/v1/notes/{id}`.
    fn routes(&self) -> OpenApiRouter {
        OpenApiRouter::new()
    }
}

/// Những gì một module nhận được lúc khởi động.
pub struct StartContext<'a> {
    module: &'static str,
    database: &'a Database,
    permissions: &'a PermissionCatalog,
    events: &'a EventCatalog,
    translations: &'a Translations,
    wake: &'a Arc<tokio::sync::Notify>,
    apis: &'a mut Apis,
    personal_data: &'a PersonalDataExport,
    resources: &'a crate::resource::ResourceDirectory,
}

impl StartContext<'_> {
    /// Database của chương trình (hai pool đọc / ghi). `Database` clone rẻ: module
    /// giữ một bản trong service của mình.
    pub fn database(&self) -> &Database {
        self.database
    }

    /// Mọi permission của mọi module đã đăng ký (vd để kiểm role chỉ cấp permission
    /// có thật).
    pub fn permissions(&self) -> &PermissionCatalog {
        self.permissions
    }

    /// Bản dịch của kernel và mọi module (vd để điền mẫu mail theo ngôn ngữ).
    pub fn translations(&self) -> &Translations {
        self.translations
    }

    /// Mọi loại event của mọi module đã đăng ký (vd để kiểm tên event client gửi lên).
    pub fn events(&self) -> &EventCatalog {
        self.events
    }

    /// Hàng đợi job của module (bảng `<tên>_jobs`).
    pub fn jobs(&self) -> JobQueue {
        JobQueue::new(self.module)
    }

    /// Nơi module ghi event của mình. Lỗi nếu module không khai event nào
    /// (`Module::events`).
    pub fn outbox(&self) -> Result<Outbox, Error> {
        if !self.events.modules().contains(&self.module) {
            return Err(Error::internal(format!(
                "module `{}` asks for an outbox but declares no events in `Module::events`",
                self.module
            )));
        }
        Ok(Outbox::with_wake(self.module, Arc::clone(self.wake)))
    }

    /// Đăng ký phần dữ liệu cá nhân module giữ, để có trong bản xuất dữ liệu của người
    /// dùng (xem [`crate::privacy`]). Mỗi module đăng ký nhiều nhất một lần.
    pub fn personal_data(&mut self, exporter: Arc<dyn PersonalData>) -> Result<(), Error> {
        self.personal_data.register(self.module, exporter)
    }

    /// Sổ dữ liệu cá nhân của mọi module, cho module quản lý tài khoản xuất dữ liệu. Sổ
    /// có cả phần các module khởi động sau đăng ký.
    pub fn personal_data_export(&self) -> PersonalDataExport {
        self.personal_data.clone()
    }

    /// Đăng ký loại đối tượng `kind` (`<module>.<loại>`) của module này, kèm bộ tra (xem
    /// [`crate::resource`]).
    pub fn resource_type(
        &mut self,
        kind: &'static str,
        resolver: Arc<dyn crate::resource::ResourceResolver>,
    ) -> Result<(), Error> {
        self.resources.register(self.module, kind, resolver)
    }

    /// Sổ đối tượng của mọi module, để tra đối tượng của module khác. Sổ có cả các loại do
    /// module khởi động sau đăng ký.
    pub fn resources(&self) -> crate::resource::ResourceDirectory {
        self.resources.clone()
    }

    /// Công bố một trait cho module khác dùng. `T` là kiểu `dyn Trait` khai trong
    /// `api` của module: `context.provide::<dyn UserDirectory>(Arc::new(service))?`.
    ///
    /// Mỗi trait chỉ có một bên công bố; công bố lần hai là lỗi.
    //
    // `T: ?Sized`: cho phép `T` là `dyn Trait` (kiểu không có kích thước cố định).
    pub fn provide<T>(&mut self, api: Arc<T>) -> Result<(), Error>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.apis.provide(self.module, api)
    }

    /// Lấy một trait do module đăng ký TRƯỚC công bố:
    /// `let users = context.require::<dyn UserDirectory>()?;`
    pub fn require<T>(&self) -> Result<Arc<T>, Error>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.apis.require(self.module)
    }
}

/// Sổ các trait đã được công bố, tra theo kiểu của trait.
#[derive(Default)]
struct Apis {
    // Khoá là "danh tính" của kiểu `T` (`TypeId`); giá trị là một `Arc<T>` cất dưới
    // dạng `dyn Any` vì mỗi mục một kiểu khác nhau. Lấy ra thì ép kiểu ngược lại
    // có kiểm tra (`downcast_ref`) — sai kiểu trả `None` chứ không hỏng bộ nhớ.
    provided: HashMap<TypeId, (&'static str, Box<dyn Any + Send + Sync>)>,
}

impl Apis {
    fn provide<T>(&mut self, module: &'static str, api: Arc<T>) -> Result<(), Error>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        let contract = std::any::type_name::<T>();
        if let Some((owner, _)) = self.provided.get(&TypeId::of::<T>()) {
            return Err(Error::internal(format!(
                "module `{module}` provides `{contract}`, but module `{owner}` already does; \
                 a contract has exactly one provider"
            )));
        }
        self.provided.insert(TypeId::of::<T>(), (module, Box::new(api)));
        Ok(())
    }

    fn get<T>(&self) -> Option<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.provided
            .get(&TypeId::of::<T>())
            .and_then(|(_, api)| api.downcast_ref::<Arc<T>>())
            .cloned()
    }

    fn require<T>(&self, module: &'static str) -> Result<Arc<T>, Error>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.get().ok_or_else(|| {
            Error::internal(format!(
                "module `{module}` requires `{}`, which no earlier module provides; register \
                     the providing module before `{module}` in crates/app/src/modules.rs",
                std::any::type_name::<T>()
            ))
        })
    }
}

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
    /// Sổ module đã cài (module `kernel` phục vụ); điền ở cuối [`start`](Self::start).
    catalog: crate::module_catalog::ModuleCatalog,
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
        for reserved in [crate::builtin::NAME, crate::antispam::module::NAME] {
            if modules.iter().any(|module| module.name() == reserved) {
                problems.push(format!("module name `{reserved}` is reserved for the kernel"));
            }
        }
        let catalog = crate::module_catalog::ModuleCatalog::default();
        let mut all: Vec<Box<dyn Module>> = vec![
            Box::new(crate::builtin::KernelModule::new(catalog.clone())),
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
                apis: Apis::default(),
                catalog,
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

    /// Khởi động từng module theo thứ tự đăng ký. Module nào hỏng thì dừng và báo
    /// tên module đó.
    pub fn start(&mut self, database: &Database) -> Result<(), ModuleError> {
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
            };
            module.start(&mut context).map_err(|error| {
                ModuleError(format!("module `{name}` failed to start: {error}"))
            })?;
        }
        self.apis = apis;
        // Sau khi MỌI module đã khởi động: tool, job, lịch của module chỉ có từ lúc này.
        self.catalog.fill(&self.modules);
        Ok(())
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

    /// Endpoint deprecated của mọi module: `(tên module, khai báo)`.
    pub fn deprecations(&self) -> Vec<(&'static str, crate::http::Deprecation)> {
        self.modules
            .iter()
            .flat_map(|module| {
                let name = module.name();
                module.deprecations().into_iter().map(move |deprecation| (name, deprecation))
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
            for tool in module.tools() {
                let tool = tool.qualify(name);
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

    /// Worker chạy job của mọi module, `concurrency` job cùng lúc mỗi module. Gọi sau
    /// [`start`](Self::start). Khai báo sai (loại job không thuộc module khai, hai bộ chạy
    /// cho một loại, lịch cho loại job không ai chạy) là lỗi khởi động.
    pub fn worker(&self, database: &Database, concurrency: u64) -> Result<Worker, ModuleError> {
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
        Ok(Worker::new(database.clone(), modules, handlers, schedules, concurrency))
    }

    /// Bộ phát event: đọc bảng event của mọi module phát event, giao cho các đăng ký
    /// nhận của mọi module. Gọi sau [`start`](Self::start). Đăng ký nhận một event
    /// không ai khai là lỗi khởi động.
    pub fn dispatcher(&self, database: &Database) -> Result<Dispatcher, ModuleError> {
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
        Ok(Dispatcher::new(
            database.clone(),
            self.events.modules(),
            subscriptions,
            Arc::clone(&self.wake),
        ))
    }

    /// Route của từng module kèm tên module, theo thứ tự đăng ký. Gọi sau
    /// [`start`](Self::start).
    pub fn routes(&self) -> Vec<(&'static str, OpenApiRouter)> {
        self.modules.iter().map(|module| (module.name(), module.routes())).collect()
    }

    /// Migrator chạy migration của mọi module.
    pub fn migrator(&self) -> Result<Migrator, MigrationError> {
        Migrator::new(
            self.modules
                .iter()
                .map(|module| ModuleMigrations::new(module.name(), module.migrations()))
                .collect(),
        )
    }
}

impl fmt::Debug for Modules {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Modules").field(&self.names()).finish()
    }
}

/// Tên mà database dành riêng làm tiền tố bảng hệ thống: SQLite giữ `sqlite_…`,
/// Postgres giữ `pg_…`. Module mang tên này sẽ tạo bảng không hợp lệ ở một bên.
const RESERVED_NAMES: [&str; 2] = ["sqlite", "pg"];

/// Các vấn đề của một danh sách tên module; rỗng nghĩa là hợp lệ.
pub(crate) fn module_name_problems(names: &[&str]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = BTreeSet::new();
    for name in names {
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

/// Chữ thường, số và `_`; bắt đầu bằng chữ; không kết thúc bằng `_`.
pub(crate) fn is_identifier(text: &str) -> bool {
    text.chars().next().is_some_and(|first| first.is_ascii_lowercase())
        && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !text.ends_with('_')
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
            ["AUTOXIDE_SECRET_KEY", "AUTOXIDE_HTTP_PUBLIC_URL"].contains(name)
                || name.starts_with("AUTOXIDE_ANTISPAM_")
        };
        assert!(names.iter().all(internal), "{names:?}");
        assert!(modules.migrator().is_ok());
    }
}
