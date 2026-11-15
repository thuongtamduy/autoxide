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
use std::sync::Arc;

use sea_orm_migration::MigrationTrait;
use utoipa_axum::router::OpenApiRouter;

use crate::access::{Permission, PermissionCatalog};
use crate::config::{ConfigReader, Invalid};
use crate::database::Database;
use crate::error::Error;
use crate::event::{EventCatalog, EventInfo, Outbox, Subscription};
use crate::i18n::Translations;
use crate::job::{JobQueue, JobRegistration, Schedule};
use crate::privacy::{PersonalData, PersonalDataExport};

mod registry;

pub use registry::{ModuleError, Modules};

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

    /// Upgrade dữ liệu của module (D-257): việc nâng dữ liệu cần cấu hình của chương trình (vd
    /// mã hoá secret cũ). Mặc định rỗng. Xem `autoxide_kernel::migration::DataUpgrade`.
    fn upgrades(&self) -> Vec<Box<dyn crate::migration::DataUpgrade>> {
        Vec::new()
    }

    /// Cấu hình module nhận theo khách (D-258): tên `<module>.<khoá>`, đọc bằng
    /// `context.require::<dyn TenantSettings>()` (`autoxide_kernel::tenancy`). Khách chưa đặt thì
    /// module dùng cấu hình chung. Mặc định rỗng.
    fn tenant_settings(&self) -> Vec<crate::tenancy::TenantSetting> {
        Vec::new()
    }

    /// Bảng mặt điều khiển của module (bước 33, D-253): bảng không có `tenant_id` vì chung cả nền
    /// tảng, chỉ có dữ liệu ở database điều khiển. Mọi bảng khác của module (trừ `<m>_migrations`)
    /// là bảng cấp khách hàng và phải có `tenant_id` — testkit và `cargo xtask check` kiểm.
    /// Mặc định rỗng: module nghiệp vụ hầu như không có bảng loại này.
    fn control_plane_tables(&self) -> Vec<&'static str> {
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

    /// Event vòng đời của các loại đối tượng module sở hữu (D-202): event nào nghĩa là đối tượng
    /// thôi nhận tương tác mới hay đã bị gỡ, để module gắn dữ liệu vào đối tượng (vd `comments`)
    /// đóng / ẩn phần của mình mà không biết module sở hữu. Event phải nằm trong
    /// [`events`](Self::events) của chính module, loại phải là `<tên module>.<loại>`, chủ thể của
    /// event là `<loại>:<id>`. Kernel kiểm lúc khởi động.
    fn resource_events(&self) -> Vec<crate::resource::ResourceEvent> {
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

    /// Bước kiểm bản cài đặt cho `autoxide doctor` (D-166): nơi lưu ghi được không, máy chủ
    /// ngoài có trả lời không, thiếu dữ liệu ban đầu không… Gọi sau `start`. Tên khai ngắn
    /// (`storage`); kernel thêm tiền tố `<module>.`. Bước kiểm chỉ đọc, hoặc dọn thứ đã ghi thử.
    fn checks(&self) -> Vec<crate::doctor::Check> {
        Vec::new()
    }

    /// Lệnh quản trị của module cho `autoxide task` (D-169): việc người vận hành chạy tay,
    /// vd làm lại dữ liệu dẫn xuất sau khi đổi cấu hình. Gọi sau `start`. Tên khai ngắn
    /// (`refresh_variants`); kernel thêm tiền tố `<module>.`.
    fn tasks(&self) -> Vec<crate::task::Task> {
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
    resource_events: &'a [crate::resource::ResourceEvent],
    verification: &'a crate::access::PostingVerification,
    tenancy: &'a crate::tenancy::DatabaseRegistry,
}

impl StartContext<'_> {
    /// Sổ database (bước 33): chế độ của bản cài và database của từng khách. Service lấy database
    /// của dữ liệu cấp khách hàng qua [`TenantContext`](crate::tenancy::TenantContext), không qua
    /// [`database`](Self::database).
    pub fn tenancy(&self) -> crate::tenancy::DatabaseRegistry {
        self.tenancy.clone()
    }

    /// Database của chương trình (database điều khiển: hai pool đọc / ghi). Cho chỗ thật sự của
    /// cả bản cài (vd khoá ký gốc); dữ liệu cấp khách hàng đi qua `TenantContext` (bước 33).
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

    /// Đăng ký bộ tính nhãn tác giả cho loại đối tượng `kind` (của module nào cũng được), với
    /// mọi nhãn `codes` nó có thể trả (xem [`crate::resource`], D-202).
    pub fn badge_source(
        &mut self,
        kind: &'static str,
        codes: &'static [&'static str],
        resolver: Arc<dyn crate::resource::BadgeResolver>,
    ) -> Result<(), Error> {
        self.resources.register_badges(self.module, kind, codes, resolver)
    }

    /// Event vòng đời mọi module đã khai qua [`Module::resource_events`] — có đủ ngay từ module
    /// khởi động đầu tiên.
    pub fn resource_events(&self) -> &[crate::resource::ResourceEvent] {
        self.resource_events
    }

    /// Đăng ký bộ hỏi "tài khoản đã xác thực để đăng nội dung chưa" (NĐ 147/2024, D-202). Chỉ
    /// một module trong chương trình đăng ký.
    pub fn posting_verifier(
        &mut self,
        verifier: Arc<dyn crate::access::PostingVerifier>,
    ) -> Result<(), Error> {
        self.verification.register(self.module, verifier)
    }

    /// Chỗ hỏi tài khoản đã xác thực chưa (thấy cả bộ hỏi do module khởi động sau đăng ký).
    pub fn posting_verification(&self) -> crate::access::PostingVerification {
        self.verification.clone()
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
