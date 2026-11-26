//! The module registry: takes the binary's module list, checks names, collects permissions, events,
//! translations, configuration and migrations, then starts each module in registration order. The contract
//! with modules (the `Module` trait, `StartContext`) is in `module.rs`.

mod sandbox;

use jiff::SignedDuration;
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

/// An error while registering modules.
#[derive(Debug)]
pub struct ModuleError(String);

impl ModuleError {
    pub(super) fn new(message: String) -> Self {
        ModuleError(message)
    }

    /// The error content, without the `modules:` prefix.
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

/// The set of registered modules, in registration order.
pub struct Modules {
    // `Box<dyn Module>`: mỗi module là một kiểu khác nhau; `dyn` cho phép để chung
    // một danh sách và gọi qua trait.
    modules: Vec<Box<dyn Module>>,
    permissions: PermissionCatalog,
    events: EventCatalog,
    translations: Translations,
    /// The bell that wakes the event dispatcher, shared by every `Outbox`.
    wake: Arc<tokio::sync::Notify>,
    /// The traits modules have published; available after [`start`](Self::start).
    apis: Apis,
    /// The personal-data parts modules register at startup.
    personal_data: PersonalDataExport,
    retention: crate::privacy::RetentionCatalog,
    /// The object kinds modules register at startup.
    resources: crate::resource::ResourceDirectory,
    /// Lifecycle events modules declare (`Module::resource_events`), already checked.
    resource_events: Vec<crate::resource::ResourceEvent>,
    /// The verified-account checker (one module registers it at startup, D-202).
    verification: crate::access::PostingVerification,
    /// The registry of installed modules (served by the `kernel` module); filled in at the end of
    /// [`start`](Self::start).
    catalog: crate::module_catalog::ModuleCatalog,
    /// Configuration for the UI of every module (D-271), filled in after `start`.
    client_configs: crate::ops::client_config::ClientConfigs,
    /// Per-user flags of the `kernel` module, served under `/api/v1/me` (D-207).
    user_flags: crate::user_flags::Handle,
    /// The root key configuration (filled by the `kernel` module at `configure`), for data upgrades.
    keys: crate::secret::KeyConfig,
    /// The seeder of initial data for new tenants of every module; filled in after every module has
    /// started.
    seeds: crate::tenancy::admin::Seeds,
    /// The per-node settings of every module (step 34).
    org_settings: crate::tenancy::OrgSettingCatalog,
    /// Per-tenant entitlements (step 35): the feature / quota registry and the handle to the provider.
    entitlements: crate::entitlement::EntitlementsHandle,
    /// The DNS lookup for a tenant's custom domains: the one a module publishes (`dyn DnsTxtLookup`),
    /// otherwise the operating system's resolver; filled in after every module has started.
    dns: crate::tenancy::domains::DnsSlot,
    /// The organization tree a module publishes (step 34), filled in after every module has started.
    orgs: crate::tenancy::domains::OrgSlot,
    /// What moving a tenant needs (modules, control-plane tables, migrations); filled in after every module
    /// has started.
    moves: crate::tenancy::moves::MoveSetup,
    /// Modules that have job / event tables, for the backlog metrics over REST (step 39); filled in after
    /// startup.
    backlog: crate::job::backlog::BacklogModules,
    /// The database registry (step 33); available after [`start`](Self::start).
    tenancy: Option<crate::tenancy::DatabaseRegistry>,
    /// The report fact sources of every module (step 38), already checked.
    facts: crate::report::FactCatalog,
    /// The analytics event kinds and funnels of every module (step 57), already checked.
    analytics: crate::analytics::AnalyticsCatalog,
    /// The registry of tools and rule templates for automations (step 58); filled in after every module has
    /// started.
    automations: crate::automation::AutomationDirectory,
    /// The sandbox copy / push registry (step 38, D-232).
    sandbox: crate::tenancy::sandbox::SandboxCatalog,
    /// The retention period of the sync change stream (`AUTOXIDE_SYNC_RETENTION_DAYS`, step 69 P2).
    sync_keep: SignedDuration,
    /// Modules present in the binary but not enabled by `AUTOXIDE_MODULES` (`Modules::select`).
    pub(super) inactive: Vec<super::InactiveModule>,
    /// The turned-off modules themselves (dormant mode, step 70 P9): their declarations (tables, migrations,
    /// split / merge policy) and [`Module::dormant`](super::Module::dormant).
    pub(super) dormant: Vec<Box<dyn super::Module>>,
    /// The account-deletion handlers turned-off modules registered in `dormant`, filled at `start`.
    dormant_subscriptions: Vec<(&'static str, crate::event::Subscription)>,
    /// `(module enabled along, module that needs it)` (`Modules::select`).
    pub(super) implied: Vec<(&'static str, &'static str)>,
    /// The verbatim `AUTOXIDE_MODULES` the installer wrote, if any.
    pub(super) selection_text: Option<String>,
}

impl Modules {
    /// Checks names then accepts the modules. A bad name is the module author's error, reported immediately
    /// at startup.
    ///
    /// Two internal modules are always added at the head of the list: `kernel` (the program's shared
    /// tables, e.g. signing keys) and `antispam` (spam protection, D-119). Any module can `require` what
    /// they publish.
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
        // Sổ khối (bước 67): tên, schema props, tính năng của chính module.
        catalog.blocks().fill(
            modules
                .iter()
                .map(|module| (module.name(), module.features(), module.blocks()))
                .collect(),
            entitlements.clone(),
            &mut problems,
        );
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
        // Bảng control-plane chỉ là bảng của chính module: không khai bảng của module khác để mở đường đọc
        // không lọc theo khách (chốt `global` của `db`). Bảng có `tenant_id` vẫn được là control-plane
        // (D-253: sổ thuê bao, tên miền của khách), nhưng record `global` trên nó bị `check_records` chặn.
        for module in &modules {
            for table in module.control_plane_tables() {
                problems.extend(crate::db::control_plane_prefix_problem(module.name(), table));
            }
        }
        if problems.is_empty() {
            // Sổ bảng control-plane cho chốt lúc chạy của `db`: record `global` chỉ dùng được trên các bảng này.
            crate::db::register_control_plane(
                modules.iter().flat_map(|module| module.control_plane_tables()),
            );
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
                sandbox: Default::default(),
                sync_keep: SignedDuration::from_hours(24 * 30),
                inactive: Vec::new(),
                dormant: Vec::new(),
                dormant_subscriptions: Vec::new(),
                implied: Vec::new(),
                selection_text: None,
            })
        } else {
            Err(ModuleError(problems.join("; ")))
        }
    }

    /// The names of the modules, in registration order.
    pub fn names(&self) -> Vec<&'static str> {
        self.modules.iter().map(|module| module.name()).collect()
    }

    /// The number of periodic schedules (`Module::schedules`) of each module, in registration order - for
    /// tests and tools to compare against the number of schedules actually declared instead of a
    /// hand-written constant.
    pub fn schedule_counts(&self) -> Vec<(&'static str, usize)> {
        self.modules.iter().map(|module| (module.name(), module.schedules().len())).collect()
    }

    pub fn is_empty(&self) -> bool {
        // Hai module nội bộ `kernel` và `antispam` luôn có, không tính.
        self.modules.len() <= 2
    }

    /// Lets each module read its configuration. A failing module does not stop other modules from reading -
    /// so every error is reported at once.
    pub fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        let mut result = crate::sync::read_retention(reader).map(|keep| self.sync_keep = keep);
        for module in &mut self.modules {
            if module.configure(reader).is_err() {
                result = Err(Invalid);
            }
        }
        // Module đang tắt (chế độ "nghỉ") cũng đọc cấu hình của nó — vd `files` cần biết kho để xoá nội dung file
        // khi tài khoản bị xoá. Đọc ở reader riêng: cấu hình sai của module tắt chỉ là cảnh báo, không chặn khởi
        // động; phần "nghỉ" của nó tự xử lý khi thiếu cấu hình.
        for module in &mut self.dormant {
            let mut side = reader.sibling();
            if module.configure(&mut side).is_err() {
                let problems: Vec<String> =
                    side.problems().iter().map(ToString::to_string).collect();
                tracing::warn!(
                    module = module.name(),
                    problems = %problems.join("; "),
                    "a turned-off module cannot read its configuration; its dormant part may not run"
                );
            }
        }
        result
    }

    /// Checks the records every module declares (`Module::records`) against the live schema of `database`
    /// (migrations applied), and that the control-plane tables of turned-off modules are their own. Call it after the migrations and before serving; a mismatch is an error listing
    /// every problem.
    pub async fn check_records(&self, database: &Database) -> Result<(), ModuleError> {
        let mut problems = Vec::new();
        // Module đang tắt không qua bước dựng `Modules`: kiểm tiền tố bảng control-plane của nó ở đây.
        for module in &self.dormant {
            for table in module.control_plane_tables() {
                problems.extend(crate::db::control_plane_prefix_problem(module.name(), table));
            }
        }
        for module in &self.modules {
            let records = module.records();
            if records.is_empty() {
                continue;
            }
            problems.extend(
                crate::db::record_problems(
                    database,
                    module.name(),
                    &records,
                    &module.control_plane_tables(),
                )
                .await
                .map_err(ModuleError)?,
            );
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(ModuleError(format!(
                "records do not match the database schema: {}",
                problems.join("; ")
            )))
        }
    }

    /// Starts each module in registration order, in single-tenant mode on `database`. If a module fails, it
    /// stops and reports that module's name.
    pub fn start(&mut self, database: &Database) -> Result<(), ModuleError> {
        self.start_in(&crate::tenancy::DatabaseRegistry::single(database.clone()))
    }

    /// Like [`start`](Self::start), with the database registry `registry` (the installation's mode, the
    /// placements): the program's database is its control database.
    pub fn start_in(
        &mut self,
        registry: &crate::tenancy::DatabaseRegistry,
    ) -> Result<(), ModuleError> {
        let database = registry.control();
        let work = self.catalog.work_sources();
        let finders = self.catalog.finder_sources();
        let suggestions = self.catalog.suggest_sources();
        let metrics = self.catalog.summary_metrics();
        let exports = self.catalog.export_sources();
        let bulk = self.catalog.bulk_actions();
        let resolvers = self.catalog.code_resolvers();
        let setup = self.catalog.setup_steps();
        let reasons = self.catalog.reason_codes();
        let blocks = self.catalog.blocks();
        let probes = self.catalog.connection_probes();
        let taggables = self.catalog.taggables();
        let texts = self.catalog.text_overrides();
        let custom_fields = self.catalog.custom_fields();
        let references = self.catalog.references();
        let mut apis = Apis::default();
        let partners = crate::tenancy::sandbox::PartnerGuard::new(registry.clone());
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
        // Sổ thời hạn lưu được liệt kê theo trang với khoá `<module>` + tên (`compliance` retention): hai mục
        // trùng tên trong một module làm trang đó lỗi 500 mỗi lần đọc — chặn ngay lúc khởi động, nói rõ chỗ sai.
        let mut seen = std::collections::BTreeSet::new();
        for (module, policy) in self.retention.all() {
            if !seen.insert((*module, policy.name)) {
                return Err(ModuleError(format!(
                    "module `{module}` declares the retention policy `{}` twice \
                     (`Module::retention`): each name must be unique within the module",
                    policy.name
                )));
            }
        }
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
                suggestions: &suggestions,
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
                blocks: &blocks,
                automations: &self.automations,
                sync_keep: self.sync_keep,
                partners: &partners,
                sandbox: &self.sandbox,
            };
            module.start(&mut context).map_err(|error| {
                ModuleError(format!("module `{name}` failed to start: {error}"))
            })?;
        }
        let missing = blocks.missing_sources();
        if !missing.is_empty() {
            return Err(ModuleError(format!(
                "blocks {missing:?} are declared `with_data` but their module never registers \
                 block data (`StartContext::block_data`)"
            )));
        }
        apis.fill_later();
        // Module đang tắt (chế độ "nghỉ", bước 70 P9): chỉ phần dữ liệu cá nhân, sau khi mọi module bật
        // đã khởi động — chúng chỉ thấy thứ module bật công bố.
        let (subscriptions, shared_storage) = super::dormant::start(
            &self.dormant,
            &apis,
            registry,
            &self.translations,
            &self.personal_data,
            self.sync_keep,
        )?;
        self.dormant_subscriptions = subscriptions;
        self.dns.fill(apis.get::<dyn crate::tenancy::DnsTxtLookup>());
        partners.fill(apis.get::<dyn crate::tenancy::sandbox::SandboxRecipients>());
        self.orgs.fill(apis.get::<dyn crate::tenancy::OrgNodes>());
        self.catalog.history().fill(apis.get::<dyn crate::resource::RecordHistory>());
        self.catalog.text_model().fill(apis.get::<dyn crate::ai::TextModel>());
        self.catalog.signal_topics().fill(apis.contributions::<dyn crate::event::SignalTopics>());
        self.entitlements.fill(apis.get::<dyn crate::entitlement::Entitlements>());
        self.entitlements.catalog().fill(apis.required(), counters);
        // Nơi lưu chung và bộ mở nơi lưu riêng của placement (module `files` công bố).
        // Không module bật nào công bố kho: kho mà module tắt (`files` "nghỉ") tự mở để xoá dữ liệu cá nhân
        // cũng là kho của kernel (xoá khách, bản xuất của khách) — không để file của khách nằm lại.
        let (storage, opener) = match apis.get::<dyn crate::storage::Storage>() {
            Some(storage) => (Some(storage), apis.get::<dyn crate::storage::StorageOpener>()),
            None => {
                shared_storage.map_or((None, None), |(storage, opener)| (Some(storage), opener))
            }
        };
        registry.use_storage(storage, opener);
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
        self.fill_sandbox()?;
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
        // Module đang tắt vẫn thuộc vòng đời khách (chế độ "nghỉ", bước 70 P9): khai báo bảng của nó
        // đi cùng module đang bật.
        let every = || self.modules.iter().chain(&self.dormant);
        let retained = every().flat_map(|module| module.retained_tables()).collect();
        let mut control_plane = self.control_plane_tables();
        control_plane.extend(self.dormant.iter().flat_map(|module| module.control_plane_tables()));
        // Chính sách tách / gộp (bước 38): bảng thuộc chính module, không khai hai lần.
        let policies = crate::tenancy::relocate::Policies::collect(
            every().map(|module| (module.name(), module.relocation())).collect(),
        )
        .map_err(|problems| ModuleError(problems.join("; ")))?;
        let dormant = super::dormant::migrators(&self.dormant, &self.keys, registry)?;
        self.moves.fill(crate::tenancy::moves::Setup {
            modules: self.names(),
            control_plane,
            migrator,
            placements,
            retained,
            policies,
            dormant,
        });
        self.backlog.fill(self.job_modules(), self.events.modules());
        Ok(())
    }

    /// Uses `lookup` instead of the operating system's DNS resolver when verifying a tenant's custom
    /// domains. Call before [`start`](Self::start) (the testkit uses a fake, with no real DNS queries);
    /// later calls and any one a module publishes afterwards are ignored.
    pub fn use_dns(&self, lookup: Arc<dyn crate::tenancy::DnsTxtLookup>) {
        self.dns.fill(Some(lookup));
    }

    /// Per-tenant entitlements (step 35): the enforcement layer used by HTTP, MCP, the worker and the event
    /// dispatcher.
    pub fn entitlements(&self) -> crate::entitlement::EntitlementsHandle {
        self.entitlements.clone()
    }

    /// The database registry used at startup; `None` before [`start`](Self::start).
    pub fn tenancy(&self) -> Option<crate::tenancy::DatabaseRegistry> {
        self.tenancy.clone()
    }

    /// The database registry, available after [`start`](Self::start).
    fn started(&self) -> Result<crate::tenancy::DatabaseRegistry, ModuleError> {
        self.tenancy.clone().ok_or_else(|| {
            ModuleError("start the modules (`Modules::start`) before building the worker".into())
        })
    }

    /// The trait `T` a module published at startup, if any. Used in the binary to fetch the traits the core
    /// needs (e.g. `dyn Authenticator`). Call after [`start`](Self::start).
    pub fn api<T>(&self) -> Option<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.apis.get()
    }

    /// The retention registry of every module (complete after `start`).
    pub fn retention(&self) -> crate::privacy::RetentionCatalog {
        self.retention.clone()
    }

    /// The registry of personal data the modules registered at [`start`](Self::start).
    pub fn personal_data(&self) -> PersonalDataExport {
        self.personal_data.clone()
    }

    /// The block registry of every module (step 67): tests and tools read block data the way `content`
    /// does.
    pub fn blocks(&self) -> crate::blocks::Blocks {
        self.catalog.blocks()
    }

    /// The object registry the modules registered (call after [`start`](Self::start)).
    pub fn resources(&self) -> crate::resource::ResourceDirectory {
        self.resources.clone()
    }

    /// Every permission of the registered modules.
    pub fn permissions(&self) -> &PermissionCatalog {
        &self.permissions
    }
}

/// Lifecycle events of every module declared, after validation: the event must be declared by that module
/// itself, the kind must belong to that module, and an event is declared only once. Errors are added to
/// `problems`.
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

    /// A module that declares nothing but its name: uses every default implementation of the contract.
    struct Bare(&'static str);

    impl Module for Bare {
        fn name(&self) -> &'static str {
            self.0
        }
    }

    /// A module that reads a variable that has a default.
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

    /// A module that owns object `shop.item` and declares a lifecycle event.
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

    /// A module that reads a required variable.
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

    /// `multi` mode without `AUTOXIDE_SECRET_KEY` set: a configuration error that says plainly what to do;
    /// `single` does not require it.
    #[test]
    fn che_do_multi_bat_buoc_khoa_goc() {
        let check = |pairs: &[(&str, &str)]| {
            let source = ConfigSource::from_pairs(pairs.iter().copied());
            let mut reader = ConfigReader::new(&source);
            let kernel = crate::config::Config::read(&mut reader);
            let mut modules = Modules::new(bare(&["auth"])).unwrap();
            let configured = modules.configure(&mut reader).and(kernel.map(|_| ()));
            reader.finish(configured).map(|_| ()).map_err(|error| error.to_string())
        };
        let error = check(&[("AUTOXIDE_TENANCY_MODE", "multi")]).unwrap_err();
        assert!(error.contains("AUTOXIDE_SECRET_KEY: is required when"), "{error}");
        let key = "k".repeat(40);
        check(&[("AUTOXIDE_TENANCY_MODE", "multi"), ("AUTOXIDE_SECRET_KEY", key.as_str())])
            .unwrap();
        check(&[]).unwrap();
    }

    #[test]
    fn module_khong_khai_gi_thi_khong_co_migration_va_cau_hinh() {
        // Biến của các module nội bộ (`kernel`, `antispam`…) đổi theo kernel; so với một bộ module
        // KHÔNG có module người dùng nào thay vì giữ danh sách tên viết tay: module `auth` rỗng
        // không được thêm biến nào.
        let names_with = |modules: &[&'static str]| {
            let source = ConfigSource::default();
            let mut reader = ConfigReader::new(&source);
            let mut registry = Modules::new(bare(modules)).unwrap();
            assert_eq!(registry.configure(&mut reader), Ok(()));
            let names: Vec<&str> = reader.values().iter().map(|value| value.setting.name).collect();
            (registry, names.into_iter().map(str::to_owned).collect::<Vec<String>>())
        };
        let (_, baseline) = names_with(&[]);
        let (modules, names) = names_with(&["auth"]);
        assert_eq!(names, baseline, "a module that declares nothing must not add settings");
        assert!(modules.migrator().is_ok());
    }
}
