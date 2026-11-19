//! Bộ đăng ký module: nhận danh sách module của binary, kiểm tên, gom permission, event,
//! bản dịch, cấu hình, migration, rồi khởi động từng module theo thứ tự đăng ký. Hợp đồng
//! với module (trait `Module`, `StartContext`) ở `module.rs`.

use std::fmt;
use std::sync::Arc;

use crate::access::PermissionCatalog;
use crate::config::{ConfigReader, Invalid};
use crate::database::Database;
use crate::event::EventCatalog;
use crate::i18n::Translations;
use crate::privacy::PersonalDataExport;

use super::{Apis, Module, StartContext, module_name_problems};

mod runtime;

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
    retention: crate::privacy::RetentionCatalog,
    /// Loại đối tượng các module đăng ký lúc khởi động.
    resources: crate::resource::ResourceDirectory,
    /// Event vòng đời các module khai (`Module::resource_events`), đã kiểm.
    resource_events: Vec<crate::resource::ResourceEvent>,
    /// Bộ hỏi tài khoản đã xác thực (một module đăng ký lúc khởi động, D-202).
    verification: crate::access::PostingVerification,
    /// Sổ module đã cài (module `kernel` phục vụ); điền ở cuối [`start`](Self::start).
    catalog: crate::module_catalog::ModuleCatalog,
    /// Cấu hình cho giao diện của mọi module (D-271), điền sau `start`.
    client_configs: crate::ops::client_config::ClientConfigs,
    /// Cờ theo người dùng của module `kernel`, phục vụ dưới `/api/v1/me` (D-207).
    user_flags: crate::user_flags::Handle,
    /// Cấu hình khoá gốc (module `kernel` điền lúc `configure`), cho upgrade dữ liệu.
    keys: crate::secret::KeyConfig,
    /// Bộ gieo dữ liệu cho khách mới của mọi module; điền sau khi mọi module khởi động.
    seeds: crate::tenancy::admin::Seeds,
    /// Cài đặt theo nút của mọi module (bước 34).
    org_settings: crate::tenancy::OrgSettingCatalog,
    /// Quyền dùng theo khách (bước 35): sổ tính năng / hạn mức và tay cầm tới bên cung cấp.
    entitlements: crate::entitlement::EntitlementsHandle,
    /// Bộ tra DNS cho tên miền riêng của khách: bản một module công bố (`dyn DnsTxtLookup`), không
    /// thì bộ tra của hệ điều hành; điền sau khi mọi module khởi động.
    dns: crate::tenancy::domains::DnsSlot,
    /// Cây tổ chức một module công bố (bước 34), điền sau khi mọi module khởi động.
    orgs: crate::tenancy::domains::OrgSlot,
    /// Thứ việc dời khách cần (module, bảng mặt điều khiển, migration); điền sau khi mọi module
    /// khởi động.
    moves: crate::tenancy::moves::MoveSetup,
    /// Module có bảng job / event, cho số đo tồn đọng qua REST (bước 39); điền sau khi khởi động.
    backlog: crate::job::backlog::BacklogModules,
    /// Sổ database (bước 33); có sau [`start`](Self::start).
    tenancy: Option<crate::tenancy::DatabaseRegistry>,
    /// Nguồn số liệu báo cáo của mọi module (bước 38), đã kiểm.
    facts: crate::report::FactCatalog,
    /// Loại sự kiện phân tích và phễu của mọi module (bước 57), đã kiểm.
    analytics: crate::analytics::AnalyticsCatalog,
    /// Sổ tool và mẫu luật cho tự động hoá (bước 58); điền sau khi mọi module khởi động.
    automations: crate::automation::AutomationDirectory,
    /// Thời hạn giữ luồng thay đổi đồng bộ (`AUTOXIDE_SYNC_RETENTION_DAYS`, bước 69 P2).
    sync_keep: chrono::Duration,
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
        // Danh sách lý do (bước 51 P18): kiểm khai báo, giữ trong sổ cho kernel và module đọc.
        let mut reason_lists = Vec::new();
        for module in &modules {
            let lists = module.reason_lists();
            crate::ops::reasons::check(module.name(), &lists, &mut problems);
            reason_lists.extend(lists);
        }
        catalog.reason_codes().fill(reason_lists);
        // Câu khách đổi được (bước 51 P21).
        let mut texts = Vec::new();
        for module in &modules {
            let declared = module.overridable_texts();
            crate::ops::texts::check(module.name(), &declared, &mut problems);
            texts.extend(declared);
        }
        catalog.text_overrides().fill(texts);
        // Loại bản ghi nhận trường tự định nghĩa (bước 51 P19).
        catalog
            .custom_fields()
            .fill(modules.iter().flat_map(|module| module.custom_field_targets()).collect());
        let client_configs = crate::ops::client_config::ClientConfigs::default();
        let user_flags = crate::user_flags::Handle::default();
        let keys = crate::secret::KeyConfig::default();
        let settings = crate::tenancy::settings::Catalog::default();
        let seeds = crate::tenancy::admin::Seeds::default();
        let dns = crate::tenancy::domains::DnsSlot::default();
        let orgs = crate::tenancy::domains::OrgSlot::default();
        let moves = crate::tenancy::moves::MoveSetup::default();
        let backlog = crate::job::backlog::BacklogModules::default();
        let mut all: Vec<Box<dyn Module>> = vec![
            Box::new(crate::builtin::KernelModule::new(
                catalog.clone(),
                client_configs.clone(),
                user_flags.clone(),
                keys.clone(),
                settings.clone(),
                crate::tenancy::Slots {
                    seeds: seeds.clone(),
                    dns: dns.clone(),
                    orgs: orgs.clone(),
                    moves: moves.clone(),
                },
                backlog.clone(),
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
        // Event stream được phải trỏ tới permission có thật (bước 40 P5).
        for (module, event) in events.streamable() {
            if let Some(permission) = event.stream_permission()
                && !permissions.contains(permission.name())
            {
                problems.push(format!(
                    "event `{}` of module `{module}` is streamable with permission `{}`, which no \
                     module declares",
                    event.name(),
                    permission.name()
                ));
            }
        }
        let resource_events = checked_resource_events(&modules, &events, &mut problems);
        // Nguồn số liệu báo cáo (bước 38): chỉ event của chính module, đã khai.
        let declared = modules.iter().map(|module| (module.name(), module.facts())).collect();
        let facts = crate::report::FactCatalog::new(declared, &events).unwrap_or_else(|errors| {
            problems.extend(errors);
            crate::report::FactCatalog::default()
        });
        // Loại sự kiện phân tích và phễu (bước 57): sự kiện phía máy chủ chỉ nghe event của chính
        // module.
        let declared = modules
            .iter()
            .map(|module| (module.name(), module.analytics_events(), module.funnels()))
            .collect();
        let analytics =
            crate::analytics::AnalyticsCatalog::new(declared, &events).unwrap_or_else(|errors| {
                problems.extend(errors);
                crate::analytics::AnalyticsCatalog::default()
            });
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
        let mut seen = Vec::new();
        let mut declared_org_settings = Vec::new();
        for module in &modules {
            let declared = module.org_settings();
            problems.extend(crate::tenancy::org_settings::declaration_problems(
                module.name(),
                &declared,
                &mut seen,
            ));
            declared_org_settings.extend(declared);
        }
        let org_settings = crate::tenancy::OrgSettingCatalog::new(declared_org_settings);
        let declared = modules
            .iter()
            .map(|module| {
                (module.name(), module.always_enabled(), module.features(), module.limits())
            })
            .collect();
        let (entitlement_catalog, entitlement_problems) =
            crate::entitlement::EntitlementCatalog::build(declared);
        problems.extend(entitlement_problems);
        let entitlements = crate::entitlement::EntitlementsHandle::new(entitlement_catalog);
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
        let automations = crate::automation::AutomationDirectory::new(entitlements.clone());
        problems.extend(crate::sync::module_problems(&modules, &permissions));
        if problems.is_empty() {
            Ok(Modules {
                modules,
                permissions,
                events,
                translations,
                wake: Arc::new(tokio::sync::Notify::new()),
                personal_data: PersonalDataExport::default(),
                retention: crate::privacy::RetentionCatalog::default(),
                resources: crate::resource::ResourceDirectory::default(),
                resource_events,
                verification: crate::access::PostingVerification::default(),
                apis: Apis::default(),
                catalog,
                client_configs,
                user_flags,
                keys,
                seeds,
                org_settings,
                entitlements,
                dns,
                orgs,
                moves,
                backlog,
                tenancy: None,
                facts,
                analytics,
                automations,
                sync_keep: chrono::Duration::days(30),
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
        let mut result = crate::sync::read_retention(reader).map(|keep| self.sync_keep = keep);
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
        let work = self.catalog.work_sources();
        let finders = self.catalog.finder_sources();
        let metrics = self.catalog.summary_metrics();
        let exports = self.catalog.export_sources();
        let bulk = self.catalog.bulk_actions();
        let resolvers = self.catalog.code_resolvers();
        let setup = self.catalog.setup_steps();
        let reasons = self.catalog.reason_codes();
        let probes = self.catalog.connection_probes();
        let taggables = self.catalog.taggables();
        let texts = self.catalog.text_overrides();
        let custom_fields = self.catalog.custom_fields();
        let references = self.catalog.references();
        let mut apis = Apis::default();
        let mut counters = std::collections::BTreeMap::new();
        // Gọi sau `configure` của mọi module: thời hạn đọc từ cấu hình.
        self.retention = crate::privacy::RetentionCatalog::new(
            self.modules
                .iter()
                .flat_map(|module| {
                    let name = module.name();
                    module.retention().into_iter().map(move |policy| (name, policy))
                })
                .collect(),
        );
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
                retention: &self.retention,
                resources: &self.resources,
                work: &work,
                finders: &finders,
                metrics: &metrics,
                exports: &exports,
                bulk: &bulk,
                resolvers: &resolvers,
                setup: &setup,
                reasons: &reasons,
                probes: &probes,
                taggables: &taggables,
                texts: &texts,
                custom_fields: &custom_fields,
                references: &references,
                resource_events: &self.resource_events,
                verification: &self.verification,
                tenancy: registry,
                org_settings: &self.org_settings,
                entitlements: &self.entitlements,
                counters: &mut counters,
                facts: &self.facts,
                analytics: &self.analytics,
                automations: &self.automations,
                sync_keep: self.sync_keep,
            };
            module.start(&mut context).map_err(|error| {
                ModuleError(format!("module `{name}` failed to start: {error}"))
            })?;
        }
        self.dns.fill(apis.get::<dyn crate::tenancy::DnsTxtLookup>());
        self.orgs.fill(apis.get::<dyn crate::tenancy::OrgNodes>());
        self.catalog.history().fill(apis.get::<dyn crate::resource::RecordHistory>());
        self.catalog.text_model().fill(apis.get::<dyn crate::ai::TextModel>());
        self.entitlements.fill(apis.get::<dyn crate::entitlement::Entitlements>());
        self.entitlements.catalog().fill(apis.required(), counters);
        // Nơi lưu chung và bộ mở nơi lưu riêng của placement (module `files` công bố).
        registry.use_storage(
            apis.get::<dyn crate::storage::Storage>(),
            apis.get::<dyn crate::storage::StorageOpener>(),
        );
        self.apis = apis;
        self.tenancy = Some(registry.clone());
        // Sổ điều hướng (bước 51 P1): nhóm trùng phải khai giống hệt, mục không trùng.
        let navigation = super::navigation::collect(
            self.modules.iter().map(|module| (module.name(), module.navigation())).collect(),
        )
        .map_err(|problems| ModuleError(problems.join("; ")))?;
        self.catalog.fill_navigation(navigation);
        // Sau khi MỌI module đã khởi động: tool, job, lịch của module chỉ có từ lúc này.
        self.catalog.fill(&self.modules, self.entitlements.catalog());
        // Tool khai sai thì chương trình dừng khi dựng endpoint MCP (cùng `tools()`); ở đây chỉ để
        // sổ rỗng.
        let mut templates = Vec::new();
        for module in &self.modules {
            for template in module.automation_templates() {
                if !template.id.starts_with(&format!("{}.", module.name()))
                    || templates.iter().any(|known: &crate::automation::AutomationTemplate| {
                        known.id == template.id
                    })
                {
                    return Err(ModuleError(format!(
                        "automation template `{}` of module `{}` must be named `{}.<name>` and \
                         be declared once",
                        template.id,
                        module.name(),
                        module.name()
                    )));
                }
                templates.push(template);
            }
        }
        self.automations.fill(self.tools().unwrap_or_default(), templates);
        self.client_configs.fill(&self.modules);
        self.seeds.fill(
            self.modules
                .iter()
                .filter_map(|module| module.seed_tenant().map(|seed| (module.name(), seed)))
                .collect(),
            self.modules
                .iter()
                .filter_map(|module| module.sample_data().map(|seed| (module.name(), seed)))
                .collect(),
        );
        let migrator = self.migrator().map_err(|error| ModuleError(error.to_string()))?;
        let placements = crate::tenancy::Placements::new(
            registry.clone(),
            self.migrator().map_err(|error| ModuleError(error.to_string()))?,
        );
        let retained = self.modules.iter().flat_map(|module| module.retained_tables()).collect();
        // Chính sách tách / gộp (bước 38): bảng thuộc chính module, không khai hai lần.
        let policies = crate::tenancy::relocate::Policies::collect(
            self.modules.iter().map(|module| (module.name(), module.relocation())).collect(),
        )
        .map_err(|problems| ModuleError(problems.join("; ")))?;
        self.moves.fill(
            self.names(),
            self.control_plane_tables(),
            migrator,
            placements,
            retained,
            policies,
        );
        self.backlog.fill(self.job_modules(), self.events.modules());
        Ok(())
    }

    /// Dùng `lookup` thay bộ tra DNS của hệ điều hành khi xác minh tên miền riêng của khách. Gọi
    /// trước [`start`](Self::start) (testkit dùng bộ giả, không hỏi DNS thật); lần gọi sau và bản
    /// một module công bố sau đó bị bỏ qua.
    pub fn use_dns(&self, lookup: Arc<dyn crate::tenancy::DnsTxtLookup>) {
        self.dns.fill(Some(lookup));
    }

    /// Quyền dùng theo khách (bước 35): lớp cưỡng chế của HTTP, MCP, worker, bộ phát event dùng.
    pub fn entitlements(&self) -> crate::entitlement::EntitlementsHandle {
        self.entitlements.clone()
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

    /// Sổ thời hạn lưu của mọi module (đủ sau `start`).
    pub fn retention(&self) -> crate::privacy::RetentionCatalog {
        self.retention.clone()
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
mod tests;
