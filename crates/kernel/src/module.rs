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

pub use crate::ops::client_config::{ClientSection, TenantClientSection};
use jiff::SignedDuration;
use std::any::{Any, TypeId};
use std::collections::{BTreeSet, HashMap};
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

pub mod navigation;
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
    /// module dùng cấu hình chung. Kết nối tới đối tác (máy chủ SMTP, cổng thanh toán…) khai
    /// `.group(CONNECTION_GROUP)` và đặt tên `<module>.connection.<loại>.<trường>`; đổi tên một cấu
    /// hình thì khai tên cũ bằng `.formerly(…)`. Mặc định rỗng.
    fn tenant_settings(&self) -> Vec<crate::tenancy::TenantSetting> {
        Vec::new()
    }

    /// Cài đặt kế thừa theo cây tổ chức của module (bước 34): tên `<module>.<tên>`, mặc định, bộ
    /// kiểm. Module `organizations` lưu giá trị theo nút và phục vụ; nút không đặt thì dùng giá trị
    /// của nút cấp trên gần nhất. Mặc định rỗng.
    fn org_settings(&self) -> Vec<crate::tenancy::OrgSetting> {
        Vec::new()
    }

    /// Gieo dữ liệu ban đầu của module cho một khách mới (vd role `admin`, dòng khoá) — gọi sau
    /// `start`, khi tạo khách (`autoxide tenant create`, `POST /api/v1/kernel/tenants`). Phải chạy
    /// lại được. Mặc định không gieo gì.
    fn seed_tenant(&self) -> Option<std::sync::Arc<dyn crate::tenancy::TenantSeed>> {
        None
    }

    /// Dữ liệu mẫu của module cho khách mới được cấp phát theo gói có `sample_data` (bước 35): vài
    /// bản ghi để khách thấy module dùng thế nào. Chỉ chạy cho module gói bật. Phải chạy lại được và
    /// không tạo trùng. Mặc định không có.
    fn sample_data(&self) -> Option<std::sync::Arc<dyn crate::tenancy::TenantSeed>> {
        None
    }

    /// Bảng cấp khách hàng của module phải GIỮ khi xoá hẳn một khách (D-337): chứng từ, sổ kế toán,
    /// hoá đơn lưu ít nhất 10 năm (Luật Kế toán 88/2015, Điều 41; D-190). Mặc định rỗng: dữ liệu
    /// của module bị xoá cùng khách.
    fn retained_tables(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// Loại dữ liệu cá nhân module giữ và thời hạn lưu của từng loại (bước 37, D-360). Gọi sau
    /// `configure` (thời hạn đọc từ cấu hình của module); kernel gom thành
    /// [`RetentionCatalog`](crate::privacy::RetentionCatalog), `compliance` công khai ở sổ hoạt động
    /// xử lý. Mặc định rỗng.
    fn retention(&self) -> Vec<crate::privacy::RetentionPolicy> {
        Vec::new()
    }

    /// Bảng mặt điều khiển của module (bước 33, D-253): bảng không có `tenant_id` vì chung cả nền
    /// tảng, chỉ có dữ liệu ở database điều khiển. Mọi bảng khác của module (trừ `<m>_migrations`)
    /// là bảng cấp khách hàng và phải có `tenant_id` — testkit và `cargo xtask check` kiểm.
    /// Mặc định rỗng: module nghiệp vụ hầu như không có bảng loại này.
    fn control_plane_tables(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// Tính năng bật / tắt theo gói của module (`<module>.<tên>`, bước 35). Module tự hỏi bằng
    /// [`EntitlementsHandle::require_feature`](crate::entitlement::EntitlementsHandle) trước việc
    /// cần tính năng. Mặc định rỗng.
    fn features(&self) -> Vec<crate::entitlement::Feature> {
        Vec::new()
    }

    /// Hạn mức theo gói của module (`<module>.<tên>`, bước 35): module giữ chỗ / trả lại qua
    /// [`EntitlementsHandle`](crate::entitlement::EntitlementsHandle). Mặc định rỗng.
    fn limits(&self) -> Vec<crate::entitlement::Limit> {
        Vec::new()
    }

    /// Module nền không bao giờ bị gói chặn (đăng nhập, file, audit…). Mặc định `false`: module bị
    /// gói chi phối — chưa bật cho khách thì route, tool MCP, job, event của nó bị chặn.
    fn always_enabled(&self) -> bool {
        false
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

    /// Resource MCP của module (bước 40 P2): tài liệu chỉ đọc cho agent — hướng dẫn, JSON Schema
    /// của một loại bản ghi. Gọi sau `start`. Path khai ngắn (`guide`); kernel ghép URI
    /// `autoxide://<module>/<path>`. Mặc định: không có.
    fn mcp_resources(&self) -> Vec<crate::mcp::McpResource> {
        Vec::new()
    }

    /// Prompt MCP của module (bước 40 P2): mẫu lời nhắc cho việc hay làm. Gọi sau `start`. Tên
    /// khai ngắn (`draft_post`); kernel thêm tiền tố `<module>.`. Mặc định: không có.
    fn prompts(&self) -> Vec<crate::mcp::Prompt> {
        Vec::new()
    }

    /// Một câu tiếng Anh nói module làm gì, cho `/llms.txt` và sổ module (bước 40 P2). Mặc định:
    /// rỗng.
    fn description(&self) -> &'static str {
        ""
    }

    /// Bước kiểm bản cài đặt cho `autoxide doctor` (D-166): nơi lưu ghi được không, máy chủ
    /// ngoài có trả lời không, thiếu dữ liệu ban đầu không… Gọi sau `start`. Tên khai ngắn
    /// (`storage`); kernel thêm tiền tố `<module>.`. Bước kiểm chỉ đọc, hoặc dọn thứ đã ghi thử.
    fn checks(&self) -> Vec<crate::doctor::Check> {
        Vec::new()
    }

    /// Nhóm phân hệ và mục menu của module cho giao diện (bước 51 P1). Kernel gom lúc khởi động
    /// (nhóm trùng phải khai giống hệt) và lọc theo khách, gói, permission ở
    /// `GET /api/v1/kernel/navigation`. Mặc định: không có.
    fn navigation(&self) -> navigation::Navigation {
        navigation::Navigation::default()
    }

    /// Danh sách lý do của module (bước 51 P18, D-223): tên `<module>.<việc>`, mã mặc định kèm nhãn
    /// tiếng Anh. Khách đổi nhãn, bật / tắt, thêm mã riêng qua `/api/v1/kernel/reasons`. Mặc định:
    /// không có.
    fn reason_lists(&self) -> Vec<crate::ops::reasons::ReasonList> {
        Vec::new()
    }

    /// Câu khách đổi được (bước 51 P21, D-235): thuật ngữ và câu hướng tới người dùng cuối, khoá
    /// `<module>.<…>`, câu tiếng Anh mặc định. Mã lỗi và câu của lỗi không bao giờ đổi được. Khách
    /// đổi qua `/api/v1/kernel/texts`; module đọc bằng `TextOverrides::text`. Mặc định: không có.
    fn overridable_texts(&self) -> Vec<crate::ops::texts::OverridableText> {
        Vec::new()
    }

    /// Loại bản ghi của module nhận trường tự định nghĩa (bước 51 P19, D-235). Module lưu giá trị
    /// trên bản ghi (cột `custom_fields`), kiểm bằng `CustomFields::validate`. Mặc định: không có.
    fn custom_field_targets(&self) -> Vec<crate::ops::custom_fields::CustomFieldTarget> {
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

    /// Giá trị cấu hình không phải secret mà giao diện cần (ngôn ngữ, giới hạn tải lên…), phục vụ
    /// ở `GET /api/v1/kernel/client-config` dưới tên module (D-271). Endpoint không cần đăng
    /// nhập: chỉ đưa giá trị ai xem cũng được. Gọi một lần sau `start` của mọi module.
    fn client_config(&self) -> Result<Option<ClientSection>, Error> {
        Ok(None)
    }

    /// Các route REST của module, kèm tài liệu OpenAPI của chúng. Gọi sau `start`.
    /// Core gắn chúng dưới `/api/v1/<tên module>`: route khai là `/` và `/{id}`
    /// của module `notes` thành `/api/v1/notes` và `/api/v1/notes/{id}`.
    fn routes(&self) -> OpenApiRouter {
        OpenApiRouter::new()
    }

    /// Số liệu báo cáo các event của module sinh ra (bước 38, D-20261118-step38-report-facts):
    /// mỗi [`FactSource`](crate::report::FactSource) nói một event của CHÍNH module (`<module>.…`,
    /// đã khai ở [`events`](Self::events)) cho ra số liệu nào. Module `reporting` nghe các event đó
    /// và đổ số liệu về kho báo cáo gộp của cây khách hàng. Kernel kiểm khai báo lúc khởi động.
    /// Mặc định: không có.
    fn facts(&self) -> Vec<crate::report::FactSource> {
        Vec::new()
    }

    /// Cách từng bảng cấp khách của module đi theo khi tách một nhánh tổ chức thành khách riêng
    /// hay gộp hai khách (bước 38, D-20261118-step38-relocation): theo nút, theo dòng cha, chép
    /// theo dòng trỏ tới, cả khách, phải rỗng hay module tự làm
    /// ([`TablePolicy`](crate::tenancy::TablePolicy)); kèm cột secret mã hoá theo khách. Mặc định:
    /// suy từ cột (có `org_id` → theo nút; hàng đợi → phải rỗng; còn lại → ở lại, cảnh báo trong kế
    /// hoạch) — test của binary chặn bảng không khai.
    fn relocation(&self) -> crate::tenancy::Relocation {
        crate::tenancy::Relocation::inferred()
    }

    /// Loại sự kiện phân tích truy cập của module (bước 57, D-220): website gửi loại `browser`, loại
    /// `server` rút từ event của chính module. Module `analytics` chỉ nhận loại đã khai. Kernel kiểm
    /// khai báo lúc khởi động. Mặc định: không có.
    fn analytics_events(&self) -> Vec<crate::analytics::AnalyticsEvent> {
        Vec::new()
    }

    /// Phễu module khai (bước 57), vd `orders.checkout`: các bước là loại sự kiện đã khai (của bất
    /// kỳ module nào). Mặc định: không có.
    fn funnels(&self) -> Vec<crate::analytics::Funnel> {
        Vec::new()
    }

    /// Mẫu luật tự động hoá dựng sẵn của module (bước 58): id `<module>.<tên>`, thân luật theo định
    /// dạng của module `automations`. Gọi sau `start`. Mặc định: không có.
    fn automation_templates(&self) -> Vec<crate::automation::AutomationTemplate> {
        Vec::new()
    }

    /// Sao cấu hình của module vào sandbox mới (bước 38, D-232): job `kernel.sandbox_copy` gọi theo
    /// thứ tự đăng ký. Chỉ cấu hình, không bao giờ dữ liệu cá nhân, giao dịch hay secret. Gọi sau
    /// `start`. Mặc định: không sao gì.
    fn sandbox_copy(&self) -> Option<Arc<dyn crate::tenancy::sandbox::SandboxCopy>> {
        None
    }

    /// Loại bản ghi của module đẩy được từ sandbox sang khách thật thành bản nháp (bước 38, D-232):
    /// `kind` là `<module>.<loại>`. Gọi sau `start`. Mặc định: không có.
    fn promotable(&self) -> Vec<crate::tenancy::sandbox::Promotable> {
        Vec::new()
    }

    /// Loại bản ghi app ngoại tuyến đồng bộ được (bước 69 P2, D-20261119-step69-sync): module có
    /// thì thêm `sync::changes_migration` vào migration, ghi thay đổi bằng `StartContext::sync_log`
    /// trong transaction ghi, và phục vụ `GET <đường dẫn>/changes`. Kernel kiểm khai báo lúc khởi
    /// động. Mặc định: không có.
    fn sync_resources(&self) -> Vec<crate::sync::SyncResource> {
        Vec::new()
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
    automations: &'a crate::automation::AutomationDirectory,
    sync_keep: SignedDuration,
    /// Bọc hợp đồng đối tác để sandbox luôn dùng bộ giả (bước 38, D-232).
    partners: &'a crate::tenancy::sandbox::PartnerGuard,
    /// Sổ sao / đẩy sandbox của mọi module, điền sau khi mọi module khởi động.
    sandbox: &'a crate::tenancy::sandbox::SandboxCatalog,
}

impl StartContext<'_> {
    /// Sổ database (bước 33): chế độ của bản cài và database của từng khách. Service lấy database
    /// của dữ liệu cấp khách hàng qua [`TenantContext`](crate::tenancy::TenantContext), không qua
    /// [`database`](Self::database).
    pub fn tenancy(&self) -> crate::tenancy::DatabaseRegistry {
        self.tenancy.clone()
    }

    /// Cài đặt theo nút mà mọi module đã khai (`Module::org_settings`), cho module lưu giá trị.
    pub fn org_settings(&self) -> crate::tenancy::OrgSettingCatalog {
        self.org_settings.clone()
    }

    /// Quyền dùng theo khách (bước 35): hỏi module / tính năng có bật không, giữ chỗ hạn mức. Có từ
    /// module khởi động đầu tiên; trước khi `plans` (hay module cung cấp khác) khởi động thì "không
    /// giới hạn".
    pub fn entitlements(&self) -> crate::entitlement::EntitlementsHandle {
        self.entitlements.clone()
    }

    /// Đăng ký bộ đếm thật cho hạn mức `limit` của module này (job đối soát sửa số lệch).
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

    /// Nơi module ghi và đọc luồng thay đổi cho app ngoại tuyến (bước 69 P2): bảng
    /// `<module>_changes`, thời hạn giữ theo `AUTOXIDE_SYNC_RETENTION_DAYS`.
    pub fn sync_log(&self) -> crate::sync::SyncLog {
        crate::sync::SyncLog::new(self.module, self.sync_keep)
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

    /// Sổ thời hạn lưu mọi module khai (`Module::retention`), đủ cả module đăng ký sau.
    pub fn retention_catalog(&self) -> crate::privacy::RetentionCatalog {
        self.retention.clone()
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

    /// Đăng ký nguồn việc của hộp việc (bước 51 P2); id phải là `<module>.<việc>`.
    pub fn work_source(
        &mut self,
        source: Arc<dyn crate::ops::work::WorkSource>,
    ) -> Result<(), Error> {
        self.work.register(self.module, source)
    }

    /// Đăng ký nguồn tìm xuyên phân hệ (bước 51 P3); loại phải là `<module>.<loại>`.
    pub fn finder_source(
        &mut self,
        source: Arc<dyn crate::ops::find::FinderSource>,
    ) -> Result<(), Error> {
        self.finders.register(self.module, source)
    }

    /// Góp nguồn gợi ý tìm cho website (D-201); loại phải là `<module>.<loại>`.
    pub fn suggest_source(
        &mut self,
        source: Arc<dyn crate::ops::suggest::SuggestSource>,
    ) -> Result<(), Error> {
        self.suggestions.register(self.module, source)
    }

    /// Sổ nguồn gợi ý (dùng chung): module phục vụ endpoint gợi ý giữ nó. Nguồn của module đăng
    /// ký sau vẫn có mặt lúc hỏi.
    pub fn suggest_sources(&self) -> crate::ops::suggest::SuggestSources {
        self.suggestions.clone()
    }

    /// Khai loại tài liệu tìm được của module (bước 40 P7): `kind.kind` là `<module>.<loại>`,
    /// `source` đọc tài liệu từ bảng của module. Khi bản ghi đổi, gọi `SearchIndex::enqueue` trong
    /// transaction của thay đổi (lấy bằng `context.require::<SearchIndex>()`); job đồng bộ sau commit.
    pub fn search_documents(
        &mut self,
        kind: crate::search::SearchKind,
        source: Arc<dyn crate::search::SearchSource>,
    ) -> Result<(), Error> {
        self.require::<crate::search::SearchIndex>()?.register(self.module, kind, source)
    }

    /// Thêm bộ máy tìm tên `name` (module bộ máy ngoài như Meilisearch, D-220); được dùng khi
    /// `AUTOXIDE_SEARCH_BACKEND = name`. Module nghiệp vụ không phải sửa gì.
    pub fn search_backend(
        &mut self,
        name: &str,
        backend: Arc<dyn crate::search::SearchBackend>,
    ) -> Result<(), Error> {
        self.require::<crate::search::SearchIndex>()?.add_backend(name, backend)
    }

    /// Đăng ký số liệu tóm tắt (bước 51 P4); id phải là `<module>.<số liệu>`, phải có định nghĩa.
    pub fn summary_metric(
        &mut self,
        metric: Arc<dyn crate::ops::summary::SummaryMetric>,
    ) -> Result<(), Error> {
        self.metrics.register(self.module, metric)
    }

    /// Sổ số liệu tóm tắt của mọi module (bước 51 P4): đọc số liệu của module khác qua hợp đồng
    /// chung (cảnh báo bất thường của `automations`, bước 58). Sổ có cả số liệu module đăng ký sau.
    pub fn summary_metrics(&self) -> crate::ops::summary::SummaryMetrics {
        self.metrics.clone()
    }

    /// Đăng ký danh sách xuất được (bước 51 P16); `resource` phải là id danh sách
    /// `<module>.<operation id>` của P7.
    pub fn export_source(
        &mut self,
        source: Arc<dyn crate::ops::list_exports::ExportSource>,
    ) -> Result<(), Error> {
        self.exports.register(self.module, source)
    }

    /// Đăng ký hành động hàng loạt (bước 51 P8) trên một danh sách `<module>.<operation id>`.
    pub fn bulk_action(
        &mut self,
        action: Arc<dyn crate::ops::bulk::BulkAction>,
    ) -> Result<(), Error> {
        self.bulk.register(self.module, action)
    }

    /// Đăng ký bộ nhận dạng mã (bước 51 P9) cho `GET /kernel/resolve`; loại `<module>.<loại>`.
    pub fn code_resolver(
        &mut self,
        resolver: Arc<dyn crate::ops::resolve::CodeResolver>,
    ) -> Result<(), Error> {
        self.resolvers.register(self.module, resolver)
    }

    /// Đăng ký bước thiết lập (bước 51 P10) cho `GET /kernel/setup`; id `<module>.<bước>`, nhãn
    /// tiếng Anh dịch qua `text:` của module.
    pub fn setup_step(&mut self, step: Arc<dyn crate::ops::setup::SetupStep>) -> Result<(), Error> {
        self.setup.register(self.module, step)
    }

    /// Danh sách lý do của mọi module (bước 51 P18): service kiểm mã người dùng gửi bằng
    /// `ReasonCodes::ensure`.
    pub fn reason_codes(&self) -> crate::ops::reasons::ReasonCodes {
        self.reasons.clone()
    }

    /// Đăng ký cách thử một kết nối ra ngoài (`<module>.connection.<loại>`) cho
    /// `POST /api/v1/kernel/settings/{name}/test`.
    pub fn connection_probe(
        &mut self,
        probe: Arc<dyn crate::ops::connections::ConnectionProbe>,
    ) -> Result<(), Error> {
        self.probes.register(self.module, probe)
    }

    /// Cho gắn nhãn nội bộ (bước 51 P20) lên loại bản ghi `kind` của module (phải có trong sổ đối
    /// tượng); `write`: permission ghi bản ghi — cần để gắn / gỡ nhãn.
    pub fn taggable(&mut self, kind: &'static str, write: Permission) -> Result<(), Error> {
        self.taggables.register(self.module, kind, write)
    }

    /// Khai bản ghi của module trỏ tới bản ghi khác (bước 51, D-267): "đang dùng ở đâu".
    pub fn reference_source(&mut self, source: Arc<dyn crate::ops::usages::ReferenceSource>) {
        self.references.register(self.module, source);
    }

    /// Trường tự định nghĩa (bước 51 P19): kiểm giá trị, ghi index lọc, dựng điều kiện lọc.
    pub fn custom_fields(&self) -> crate::ops::custom_fields::CustomFields {
        self.custom_fields.clone()
    }

    /// Câu khách đã đổi (bước 51 P21): tra trước câu mặc định.
    pub fn text_overrides(&self) -> crate::ops::texts::TextOverrides {
        self.texts.clone()
    }

    /// Ghi lần dùng / lỗi cuối của kết nối ra ngoài (hiện ở `GET /api/v1/kernel/settings`).
    pub fn connection_status(&self) -> crate::ops::connections::ConnectionStatus {
        crate::ops::connections::ConnectionStatus
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
    /// Mỗi trait chỉ có một bên công bố; công bố lần hai là lỗi. Hợp đồng đối tác của kernel
    /// (thanh toán, SMS, mail…) được kernel bọc để sandbox luôn dùng bộ giả (D-232).
    //
    // `T: ?Sized`: cho phép `T` là `dyn Trait` (kiểu không có kích thước cố định).
    pub fn provide<T>(&mut self, api: Arc<T>) -> Result<(), Error>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        let api = self.partners.wrap(api);
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

    /// Góp một bộ cài đặt cho trait mà NHIỀU module cùng cung cấp (bước 36), vd mỗi module bộ nối
    /// đối tác góp một `dyn PaymentGateway`: `context.contribute::<dyn PaymentGateway>(Arc::new(vnpay))?`.
    /// Khác [`provide`](Self::provide) (đúng một bên công bố): bao nhiêu module góp cũng được, mỗi
    /// module góp mỗi trait tối đa một lần.
    pub fn contribute<T>(&mut self, api: Arc<T>) -> Result<(), Error>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        let api = self.partners.wrap(api);
        self.apis.contribute(self.module, api)
    }

    /// Mọi bộ cài đặt của `T` mà các module đăng ký TRƯỚC đã góp, theo thứ tự đăng ký; chưa ai góp
    /// thì rỗng (bên dùng tự quyết đó có phải lỗi không).
    pub fn contributions<T>(&self) -> Vec<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.apis.contributions()
    }

    /// Mọi nguồn số liệu báo cáo (`Module::facts`) của mọi module đã đăng ký — kể cả module đăng ký
    /// sau — kèm tên module, theo thứ tự đăng ký (bước 38). Module `reporting` đọc để nghe đúng các
    /// event đó.
    pub fn fact_sources(&self) -> Vec<(&'static str, crate::report::FactSource)> {
        self.facts.sources()
    }

    /// Mọi loại sự kiện phân tích và phễu của mọi module đã đăng ký — kể cả module đăng ký sau
    /// (bước 57). Module `analytics` đọc để biết nhận sự kiện nào.
    pub fn analytics_catalog(&self) -> crate::analytics::AnalyticsCatalog {
        self.analytics.clone()
    }

    /// Sổ tool MCP và mẫu luật tự động hoá của mọi module (bước 58), điền sau khi MỌI module đã
    /// khởi động: giữ tay cầm lúc `start`, chỉ đọc khi phục vụ.
    pub fn automation_directory(&self) -> crate::automation::AutomationDirectory {
        self.automations.clone()
    }

    /// Sổ sao / đẩy sandbox của mọi module (chỉ kernel dùng); đọc khi phục vụ, không lúc `start`.
    pub(crate) fn sandbox_catalog(&self) -> crate::tenancy::sandbox::SandboxCatalog {
        self.sandbox.clone()
    }

    /// Bộ phân phối dữ liệu khách cha → khách con của module này (bước 38): ghi bản phân phối
    /// trong transaction của khách cha, job `<module>.distribute` áp vào từng khách con. Xem
    /// [`crate::tenancy::distribution`].
    pub fn distributor(&self) -> crate::tenancy::distribution::Distributor {
        crate::tenancy::distribution::Distributor::new(
            self.module,
            self.tenancy.clone(),
            self.entitlements.clone(),
        )
    }
}

/// Một bộ góp: module góp và `Arc<T>` cất dưới dạng `dyn Any`.
type Contribution = (&'static str, Box<dyn Any + Send + Sync>);

/// Sổ các trait đã được công bố, tra theo kiểu của trait.
#[derive(Default)]
struct Apis {
    // Khoá là "danh tính" của kiểu `T` (`TypeId`); giá trị là một `Arc<T>` cất dưới
    // dạng `dyn Any` vì mỗi mục một kiểu khác nhau. Lấy ra thì ép kiểu ngược lại
    // có kiểm tra (`downcast_ref`) — sai kiểu trả `None` chứ không hỏng bộ nhớ.
    provided: HashMap<TypeId, (&'static str, Box<dyn Any + Send + Sync>)>,
    /// `(module, module cung cấp)` mỗi lần một module `require` trait của module khác: sổ module
    /// và `plans` biết module nào cần module nào (bước 35). `Mutex` vì `require` chỉ mượn `&self`.
    required: std::sync::Mutex<BTreeSet<(&'static str, &'static str)>>,
    /// Trait nhiều bên cùng góp (`contribute`): theo kiểu, danh sách `(module, Arc<T>)` theo thứ tự góp.
    contributed: HashMap<TypeId, Vec<Contribution>>,
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

    fn contribute<T>(&mut self, module: &'static str, api: Arc<T>) -> Result<(), Error>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        let entries = self.contributed.entry(TypeId::of::<T>()).or_default();
        if entries.iter().any(|(owner, _)| *owner == module) {
            return Err(Error::internal(format!(
                "module `{module}` contributes `{}` twice; contribute each contract once",
                std::any::type_name::<T>()
            )));
        }
        entries.push((module, Box::new(api)));
        Ok(())
    }

    fn contributions<T>(&self) -> Vec<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.contributed
            .get(&TypeId::of::<T>())
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|(_, api)| api.downcast_ref::<Arc<T>>().cloned())
                    .collect()
            })
            .unwrap_or_default()
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

    /// Module đã công bố `T`, nếu có.
    fn provider_of<T>(&self) -> Option<&'static str>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.provided.get(&TypeId::of::<T>()).map(|(module, _)| *module)
    }

    /// Các cặp `(module, module cung cấp)` đã ghi.
    fn required(&self) -> BTreeSet<(&'static str, &'static str)> {
        self.required.lock().map(|required| required.clone()).unwrap_or_default()
    }

    fn require<T>(&self, module: &'static str) -> Result<Arc<T>, Error>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        if let Some(provider) = self.provider_of::<T>()
            && provider != module
            && let Ok(mut required) = self.required.lock()
        {
            required.insert((module, provider));
        }
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
