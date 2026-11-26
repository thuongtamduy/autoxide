//! The internal `kernel` module: always registered first (in [`crate::module::Modules::new`]), it holds the
//! tables the whole program shares that belong to no business module — currently `kernel_secrets` (signing
//! keys, [`crate::secret`]). It also reads and publishes the API's public URL ([`crate::public_url`]), serves
//! the registry of installed modules ([`crate::module_catalog`], D-155) and the per-user flags under
//! `/api/v1/me` ([`crate::user_flags`], D-207). Being a module like any other, the table ownership rule,
//! `autoxide backup` / `copy` and `migrate` apply to it just the same.

use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;

use crate::access::Permission;
use crate::config::{ConfigReader, Invalid};
use crate::error::Error;
use crate::migration::prelude::*;
use crate::module::{Module, StartContext};
use crate::module_catalog::{self, CatalogState, ModuleCatalog};
use crate::public_url::PublicUrl;
use crate::secret::SecretKey;
use crate::user_flags::{self, UserFlags};

/// Name of the internal module; other modules must not use this name.
pub(crate) const NAME: &str = "kernel";

pub(crate) struct KernelModule {
    /// Values of `AUTOXIDE_SECRET_KEY` and `AUTOXIDE_SECRET_KEY_PREVIOUS`; filled at `configure`, shared
    /// with `Migrator`.
    keys: crate::secret::KeyConfig,
    /// Value of `AUTOXIDE_HTTP_PUBLIC_URL`; available after `configure`.
    public_url: PublicUrl,
    /// Module registry, filled by `Modules::start` after all modules have started.
    catalog: ModuleCatalog,
    /// The kernel's outbox; available after `start` (event `kernel.job_retried`).
    outbox: Option<crate::event::Outbox>,
    /// Plan-based entitlements (step 35); available after `start` (the navigation registry filters by it).
    entitlements: Option<crate::entitlement::EntitlementsHandle>,
    /// Operational alerts (the checks of all modules, results kept for one minute).
    alerts: Arc<crate::ops::alerts::Alerts>,
    /// Configuration for the UI of all modules (D-271), filled together with the module registry.
    client_configs: crate::ops::client_config::ClientConfigs,
    /// The program's database; available after `start` (to read migration status).
    database: Option<crate::database::Database>,
    /// `AUTOXIDE_USER_FLAGS_MAX`; available after `configure`.
    max_flags: u64,
    /// Shared template builder (`AUTOXIDE_TEMPLATE_MAX_BYTES`, D-351); published at `start`.
    templates: crate::template::TemplateEngine,
    /// Per-user flags service, shared with `Modules::routes`; filled at `start`.
    flags: user_flags::Handle,
    /// Registry of per-tenant settings of all modules (filled when `Modules` is built).
    tenant_settings: crate::tenancy::settings::Catalog,
    /// Per-tenant settings service; available after `start`.
    settings: Option<Arc<crate::tenancy::settings::SettingsService>>,
    /// Database registry; available after `start` (platform staff set the settings of other tenants).
    registry: Option<crate::tenancy::DatabaseRegistry>,
    /// Per-tenant provider accounts (step 70 P7): read connection settings, route webhooks.
    accounts: Option<crate::provider::ProviderAccounts>,
    /// Seeder for new tenants, covering all modules.
    seeds: crate::tenancy::admin::Seeds,
    /// Tenant management service; available after `start`.
    admin: Option<Arc<crate::tenancy::TenantAdmin>>,
    /// DNS resolver for custom domains (filled after all modules have started).
    dns: crate::tenancy::domains::DnsSlot,
    /// Organization tree (step 34) for attaching domains to nodes (filled after all modules have started).
    orgs: crate::tenancy::domains::OrgSlot,
    /// Service for a tenant's custom domains; available after `start`.
    domains: Option<Arc<crate::tenancy::TenantDomains>>,
    /// What moving a tenant needs from all modules (filled after all modules have started).
    move_setup: crate::tenancy::moves::MoveSetup,
    /// Service that moves tenants between placements; available after `start`.
    moves: Option<Arc<crate::tenancy::TenantMoves>>,
    /// Split / merge tenants (step 38): settings, service, routes, jobs.
    relocations: crate::tenancy::relocate::Wiring,
    /// Test tenants (sandbox, step 38, D-232).
    sandboxes: crate::tenancy::sandbox::Wiring,
    /// Tenant data export (step 35, D-328); available after `start`.
    exports: Option<Arc<crate::tenancy::TenantExports>>,
    /// Placement migration through the API (D-334); available after `start`.
    migration_runs: Option<Arc<crate::tenancy::migration_runs::MigrationRuns>>,
    /// `AUTOXIDE_TENANCY_EXPORT_RETENTION_DAYS`.
    export_days: i64,
    /// `AUTOXIDE_JOBS_FILE_RETENTION_DAYS` (step 51 P16).
    job_file_days: u32,
    /// The kernel's job queue (list export); available after `start`.
    jobs: Option<crate::job::JobQueue>,
    /// Event stream settings (step 40 P5); filled at `configure`.
    stream_config: crate::event::stream::StreamConfig,
    /// SSE event stream hub; available after `start`.
    stream: Option<crate::event::StreamHub>,
    /// `AUTOXIDE_SEARCH_BACKEND` (step 40 P7); filled at `configure`.
    search_backend: String,
    /// Registry of searchable document types and the search engine; available after `start`.
    search: Option<crate::search::SearchIndex>,
    /// Object registry and permission registry (notes on records, step 51 P13); available after `start`.
    resources: Option<(crate::resource::ResourceDirectory, crate::access::PermissionCatalog)>,
    /// `AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS` (read in the kernel settings): moving a tenant waits this long
    /// for running requests to finish.
    request_timeout: std::time::Duration,
    /// Translations of all modules (translating permission and event descriptions in the module registry); available after `start`.
    translations: crate::i18n::Translations,
    /// Scheduled backup (step 39): settings, and the runner available after `start`.
    backup_config: crate::backup::scheduled::BackupConfig,
    backups: Option<Arc<crate::backup::scheduled::ScheduledBackups>>,
    /// Modules that have job / event tables, for `GET /kernel/ops/backlog` (step 39).
    backlog: crate::job::backlog::BacklogModules,
}

impl KernelModule {
    /// What `GET /kernel/ops/backlog` needs; available after `start`.
    fn backlog_state(&self) -> Option<crate::job::backlog_api::BacklogState> {
        let registry = self.registry.clone()?;
        Some(crate::job::backlog_api::BacklogState { registry, modules: self.backlog.clone() })
    }

    pub(crate) fn new(
        catalog: ModuleCatalog,
        client_configs: crate::ops::client_config::ClientConfigs,
        flags: user_flags::Handle,
        keys: crate::secret::KeyConfig,
        tenant_settings: crate::tenancy::settings::Catalog,
        slots: crate::tenancy::Slots,
        backlog: crate::job::backlog::BacklogModules,
    ) -> Self {
        let crate::tenancy::Slots { seeds, dns, orgs, moves: move_setup } = slots;
        KernelModule {
            dns,
            orgs,
            domains: None,
            move_setup,
            moves: None,
            relocations: Default::default(),
            sandboxes: Default::default(),
            exports: None,
            migration_runs: None,
            export_days: 30,
            job_file_days: 7,
            jobs: None,
            stream_config: Default::default(),
            stream: None,
            search_backend: crate::search::DATABASE_BACKEND.to_owned(),
            search: None,
            resources: None,
            request_timeout: std::time::Duration::from_secs(30),
            keys,
            tenant_settings,
            settings: None,
            registry: None,
            accounts: None,
            seeds,
            admin: None,
            public_url: PublicUrl::default(),
            alerts: Arc::new(crate::ops::alerts::Alerts::new(catalog.clone())),
            entitlements: None,
            outbox: None,
            catalog,
            client_configs,
            database: None,
            max_flags: 500,
            templates: crate::template::TemplateEngine::default(),
            flags,
            translations: crate::i18n::Translations::default(),
            backup_config: crate::backup::scheduled::BackupConfig::default(),
            backups: None,
            backlog,
        }
    }

    /// Tenant data export, only when there is a file storage (module `files`): with no place to write the
    /// export there is no route, tool, job or schedule for it.
    fn live_exports(&self) -> Option<&Arc<crate::tenancy::TenantExports>> {
        let storage =
            self.registry.as_ref().is_some_and(|registry| registry.shared_storage().is_some());
        self.exports.as_ref().filter(|_| storage)
    }

    fn catalog_state(&self) -> Option<CatalogState> {
        let database = self.database.clone()?;
        Some(CatalogState {
            catalog: self.catalog.clone(),
            database,
            translations: self.translations.clone(),
        })
    }

    /// Available after `start` (needs plan entitlements).
    fn navigation_state(&self) -> Option<crate::ops::navigation::NavigationState> {
        Some(crate::ops::navigation::NavigationState {
            catalog: self.catalog.clone(),
            entitlements: self.entitlements.clone()?,
            translations: self.translations.clone(),
        })
    }
}

impl Module for KernelModule {
    fn navigation(&self) -> crate::module::navigation::Navigation {
        use crate::module::navigation::{MobilePlacement, NavGroup, NavItem, Navigation};
        use crate::tenancy::{PLACEMENTS_READ, TENANTS_READ};
        Navigation::default()
            .group(NavGroup::new("home", "Home", "home", 10))
            .group(NavGroup::new("settings", "Settings", "gear", 110))
            .group(NavGroup::new("platform", "Platform", "server", 200).platform())
            .item(
                NavItem::new("home.dashboard", "home", "Overview", 10)
                    .mobile(MobilePlacement::Primary, 10),
            )
            .item(
                NavItem::new("home.work", "home", "Work and activity", 20)
                    .counter("work")
                    .mobile(MobilePlacement::Primary, 20),
            )
            .item(NavItem::new("settings.general", "settings", "General", 10).permissions(&[
                crate::ops::module_catalog::MODULES_READ,
                crate::tenancy::SETTINGS_MANAGE,
            ]))
            .item(
                NavItem::new("platform.tenants", "platform", "Tenants", 10)
                    .permissions(&[TENANTS_READ]),
            )
            .item(
                NavItem::new("platform.fleet", "platform", "Databases", 40)
                    .permissions(&[PLACEMENTS_READ])
                    .mobile(MobilePlacement::Hidden, 40),
            )
            .item(
                NavItem::new("platform.health", "platform", "Health", 50)
                    .permissions(&[TENANTS_READ]),
            )
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(CreateSecrets),
            Box::new(user_flags::CreateUserFlags),
            // Bảng mặt điều khiển của phần đa khách, gieo khách mặc định (bước 33, D-253).
            Box::new(crate::tenancy::schema::CreateTenancy),
            user_flags::adopt_tenancy(),
            // Cấu hình theo khách (D-258) và bảng event của kernel (`kernel.setting_changed`).
            Box::new(crate::tenancy::settings::CreateTenantSettings),
            crate::event::outbox_migration("m0006_create_events", NAME),
            crate::event::outbox_actor_migration("m0007_add_event_actor", NAME),
            crate::tenancy::migration::adopt(
                "m0008_tenancy_events",
                vec![crate::event::tenant_table(NAME)],
            ),
            // Hàng đợi job của kernel (`kernel.verify_domains`, D-190), theo khách từ đầu.
            crate::job::jobs_migration("m0009_create_jobs", NAME),
            crate::tenancy::migration::adopt(
                "m0010_tenancy_jobs",
                vec![crate::job::tenant_table(NAME)],
            ),
            // Lượt dời khách giữa các placement (mặt điều khiển).
            Box::new(crate::tenancy::moves::CreateTenantMoves),
            // Số lần kiểm lại hằng ngày liên tiếp không thấy bản ghi TXT của tên miền đã xác minh.
            Box::new(crate::tenancy::domains::AddCheckFailures),
            // Khách chỉ đọc, tách khỏi tạm khoá (đối chiếu thiết kế 2, A-3.11).
            Box::new(crate::tenancy::admin::AddReadOnly),
            // Cấu trúc cây tổ chức trong mỗi khách (bước 34): nút và bảng tổ tiên – con cháu.
            Box::new(crate::tenancy::org_tree::CreateOrgTree),
            // Đường người làm vào khách (bước 34, D-310).
            crate::event::outbox_access_migration("k0001_event_access", NAME),
            // Xuất toàn bộ dữ liệu một khách (bước 35, D-328).
            Box::new(crate::tenancy::exports::CreateTenantExports),
            // Dãy số tuần tự không nhảy số (bước 36): cấp khách hàng, có ở mọi database dữ liệu.
            Box::new(crate::numbering::CreateSequences),
            // Lượt migrate placement bằng API (D-334, mặt điều khiển).
            Box::new(crate::tenancy::migration_runs::CreateMigrationRuns),
            // Job người dùng theo dõi được (bước 51 P15): cấp khách hàng, mọi database dữ liệu.
            Box::new(crate::job::runs::CreateJobRuns),
            // View đã lưu của danh sách (bước 51 P7): cấp khách hàng, mọi database dữ liệu.
            Box::new(crate::ops::views::CreateSavedViews),
            // File của job (bản xuất danh sách, bước 51 P16).
            Box::new(crate::job::runs::AddJobFiles),
            // Nơi lưu dữ liệu theo khách (bước 37, D-360): nước của placement, yêu cầu của khách.
            Box::new(crate::tenancy::residency::AddDataResidency),
            // Hoãn / giao việc của hộp việc (bước 51 P10).
            Box::new(crate::ops::work_states::CreateWorkStates),
            // Ghi chú trên mọi bản ghi (bước 51 P13).
            Box::new(crate::ops::records::CreateRecordNotes),
            // Yêu cầu duyệt lệnh của agent (bước 51 P12).
            Box::new(crate::ops::approvals::CreateApprovalRequests),
            // Danh sách lý do theo khách (bước 51 P18).
            Box::new(crate::ops::reasons::CreateReasonCodes),
            // Lần dùng / lỗi cuối của kết nối ra ngoài (bước 51).
            Box::new(crate::ops::connections::CreateConnectionStatus),
            // Nhãn nội bộ gắn bản ghi (bước 51 P20).
            Box::new(crate::ops::tags::CreateTags),
            // Câu chữ khách đổi (bước 51 P21).
            Box::new(crate::ops::texts::CreateTextOverrides),
            // Trường tự định nghĩa (bước 51 P19).
            Box::new(crate::ops::custom_fields::CreateCustomFields),
            // Ghim của người dùng (bước 52 mục 4).
            Box::new(crate::ops::pins::CreatePins),
            // Nhật ký request theo khách (bước 52 mục 1).
            Box::new(crate::ops::request_log::CreateRequestLog),
            // Bảng đếm rate limit theo API key dùng chung mọi máy chủ (bước 40 P6, D-375).
            crate::throttle::throttle_migration("m0032_create_throttle", NAME),
            crate::tenancy::migration::adopt(
                "m0033_tenancy_throttle",
                vec![crate::throttle::tenant_table(NAME)],
            ),
            // Chỉ mục tìm kiếm mặc định (bước 40 P7): cấp khách hàng, mọi database dữ liệu.
            Box::new(crate::search::database::CreateSearchDocs),
            // Lượt tách / gộp khách (bước 38, mặt điều khiển).
            Box::new(crate::tenancy::relocate::CreateRelocations),
            // Sổ các lượt backup theo lịch (bước 39, mặt điều khiển).
            Box::new(crate::backup::scheduled::CreateBackups),
            // Khách thử nghiệm (bước 38, D-232): cột sandbox của `kernel_tenants`.
            Box::new(crate::tenancy::sandbox::AddSandbox),
            // Hộp thư giả của sandbox (bước 38, D-232): cấp khách hàng, mọi database dữ liệu.
            Box::new(crate::tenancy::sandbox::CreateSandboxMessages),
            // Mỗi lúc một lượt migrate placement: unique `active_key` (rà đồng thời mục 31).
            Box::new(crate::tenancy::migration_runs::AddMigrationRunActiveKey),
            // Nhiều khách cùng chờ một tên miền, ai xác minh trước được (P-4, D-20261125-domain-claims).
            Box::new(crate::tenancy::domains::ShareClaims),
            // Index cho danh sách "của tôi" có phân trang: ghim, view đã lưu (bước 70 P9).
            Box::new(crate::ops::list_indexes::AddListIndexes),
            // Index cho danh sách của kernel có phân trang ở cụm 3: job, trường tự định nghĩa (bước 70 P9).
            Box::new(crate::ops::list_indexes::AddListIndexesKernel),
        ]
    }

    fn relocation(&self) -> crate::tenancy::Relocation {
        crate::tenancy::relocate::kernel_relocation()
    }

    fn control_plane_tables(&self) -> Vec<&'static str> {
        // Khoá ký gốc của bản cài và sổ khách / vị trí dữ liệu / tên miền (D-253).
        vec![
            "kernel_secrets",
            "kernel_tenants",
            "kernel_placements",
            "kernel_tenant_domains",
            "kernel_tenant_moves",
            "kernel_tenant_exports",
            "kernel_migration_runs",
            "kernel_relocations",
            "kernel_backups",
        ]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        let secret = match crate::secret::read_setting(reader) {
            // Chế độ `multi` bắt buộc khoá gốc ngoài database (bước 33): nhiều khách chung một bản
            // cài thì khoá nằm trong database không bảo vệ được bản backup của khách nào.
            Ok(None) if multi_tenant(reader) => reader.reject(
                &crate::secret::SECRET_KEY,
                "is required when AUTOXIDE_TENANCY_MODE=multi: secrets of every tenant are \
                 encrypted with it, and a key kept in the database does not protect a backup. \
                 Generate one with `openssl rand -base64 48` and back it up apart from the database",
            ),
            other => other,
        };
        let previous = crate::secret::read_previous(reader);
        let public_url = crate::public_url::read_setting(reader);
        let max_flags = user_flags::read_setting(reader);
        let template_bytes = crate::template::read_setting(reader);
        let export_days = reader.get_with(&crate::tenancy::exports::EXPORT_RETENTION_DAYS, |raw| {
            crate::config::parse_in_range(raw, 1i64, 365)
        });
        let ui_texts = reader.get_with(&crate::ops::texts::UI_MAX, |raw| {
            crate::config::parse_in_range(raw, 1u64, 100_000)
        });
        let tenant_reasons = reader.get_with(&crate::ops::reasons::TENANT_REASONS_MAX, |raw| {
            crate::config::parse_in_range(raw, 1u64, 10_000)
        });
        let stream_config = crate::event::stream::StreamConfig::read(reader);
        let relocation_drain = crate::tenancy::relocate::Wiring::read(reader);
        let sandbox = crate::tenancy::sandbox::Wiring::read(reader);
        let search_backend = reader.get_with(&crate::search::SEARCH_BACKEND, |raw| {
            let name = raw.trim();
            let valid = !name.is_empty()
                && name.len() <= 32
                && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if valid {
                Ok(name.to_owned())
            } else {
                Err("must be `database` or the name of a search engine module (lowercase letters, \
                     digits and `_`)"
                    .to_owned())
            }
        });
        let job_file_days = reader
            .get_with(&crate::ops::list_exports::FILE_RETENTION_DAYS, |raw| {
                crate::config::parse_in_range(raw, 1u32, 365)
            });
        let backup_config = crate::backup::scheduled::BackupConfig::read(reader);
        let provider_bytes = crate::contracts::provider::read_setting(reader);
        if let Some(seconds) = request_timeout(reader) {
            self.request_timeout = std::time::Duration::from_secs(seconds);
        }
        self.backup_config = backup_config?;
        crate::contracts::provider::set_max_response_bytes(provider_bytes?);
        self.keys.set(secret?, previous?);
        self.public_url = public_url?;
        self.max_flags = max_flags?;
        self.templates = crate::template::TemplateEngine::new(template_bytes?);
        self.export_days = export_days?;
        self.job_file_days = job_file_days?;
        self.stream_config = stream_config?;
        self.search_backend = search_backend?;
        self.relocations.set(relocation_drain?);
        self.sandboxes.set(sandbox?);
        self.catalog.text_overrides().set_ui_max(ui_texts?);
        self.catalog.reason_codes().set_tenant_max(tenant_reasons?);
        Ok(())
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![
            module_catalog::MODULES_READ,
            TENANTS_OPERATE,
            crate::tenancy::SETTINGS_MANAGE,
            crate::tenancy::TENANTS_READ,
            crate::tenancy::TENANTS_MANAGE,
            crate::tenancy::PLACEMENTS_READ,
            crate::tenancy::DOMAINS_MANAGE,
            crate::tenancy::TENANTS_MOVE,
            crate::tenancy::TENANTS_RELOCATE,
            crate::tenancy::RELOCATIONS_REQUEST,
            crate::tenancy::SANDBOXES_READ,
            crate::tenancy::SANDBOXES_MANAGE,
            crate::tenancy::DATA_EXPORT,
            crate::tenancy::TENANTS_DELETE,
            crate::job::runs::JOBS_READ,
            crate::ops::views::VIEWS_SHARE,
            crate::ops::reasons::REASONS_MANAGE,
            crate::ops::tags::TAGS_MANAGE,
            crate::ops::texts::TEXTS_MANAGE,
            crate::ops::custom_fields::CUSTOM_FIELDS_MANAGE,
            crate::job::runs::JOBS_MANAGE,
            crate::ops::request_log::DEVTOOLS_READ,
            crate::ops::request_log::DEVTOOLS_MANAGE,
            crate::job::backlog_api::OPS_READ,
        ]
    }

    fn events(&self) -> Vec<crate::event::EventInfo> {
        vec![
            crate::tenancy::settings::SETTING_CHANGED,
            crate::job::runs::JOB_RETRIED,
            crate::job::runs::JOB_FINISHED,
            crate::ops::records::RECORD_NOTE_ADDED,
            crate::ops::approvals::APPROVAL_DECIDED,
            crate::ops::tags::RECORD_TAGS_CHANGED,
            crate::tenancy::admin::TENANT_CREATED,
            crate::tenancy::admin::TENANT_UPDATED,
            crate::tenancy::domains::DOMAIN_ADDED,
            crate::tenancy::domains::DOMAIN_VERIFIED,
            crate::tenancy::domains::DOMAIN_REVOKED,
            crate::tenancy::domains::DOMAIN_FAILED,
            crate::tenancy::domains::DOMAIN_UPDATED,
            crate::tenancy::moves::TENANT_MOVED,
            crate::tenancy::relocate::RELOCATION_STARTED,
            crate::tenancy::relocate::RELOCATION_FINISHED,
            crate::tenancy::relocate::RELOCATION_FAILED,
            crate::tenancy::erasure::TENANT_DELETED,
            crate::tenancy::sandbox::SANDBOX_CREATED,
            crate::tenancy::sandbox::SANDBOX_EXTENDED,
            crate::tenancy::sandbox::SANDBOX_DELETED,
            crate::tenancy::sandbox::SANDBOX_PROMOTED,
            crate::tenancy::exports::TENANT_EXPORT_REQUESTED,
            crate::tenancy::exports::TENANT_EXPORT_ACCESSED,
        ]
    }

    fn limits(&self) -> Vec<crate::entitlement::Limit> {
        vec![
            crate::entitlement::API_REQUESTS,
            crate::http::tenant_rate_limit::API_RATE,
            crate::ops::custom_fields::CUSTOM_FIELDS_LIMIT,
            crate::tenancy::SANDBOXES_LIMIT,
        ]
    }

    fn tenant_settings(&self) -> Vec<crate::tenancy::TenantSetting> {
        vec![
            crate::tenancy::PUBLIC_URL,
            crate::tenancy::BUSINESS_MODEL,
            crate::ops::tags::TAGS_RESTRICTED,
            crate::tenancy::sandbox::WEBHOOK_URLS,
        ]
    }

    fn routes(&self) -> OpenApiRouter {
        let catalog = self.catalog_state().map_or_else(OpenApiRouter::new, module_catalog::routes);
        let mut routes = catalog
            .merge(crate::ops::client_config::routes(self.client_configs.clone()))
            .merge(crate::ops::whats_new::routes())
            .merge(crate::ops::alerts::routes(self.alerts.clone()));
        if let Some(state) = self.backlog_state() {
            routes = routes.merge(crate::job::backlog_api::routes(state));
        }
        if let Some(state) = self.navigation_state() {
            routes = routes.merge(crate::ops::navigation::routes(state));
        }
        if let Some(outbox) = &self.outbox {
            routes = routes.merge(crate::job::runs::routes(crate::job::runs::JobsState {
                outbox: outbox.clone(),
            }));
        }
        if let Some(outbox) = &self.outbox {
            routes = routes.merge(crate::ops::approvals::routes(
                crate::ops::approvals::ApprovalsState { outbox: outbox.clone() },
            ));
        }
        routes = routes.merge(crate::ops::request_log::routes());
        if let Some(hub) = &self.stream {
            routes = routes.merge(crate::http::stream::routes(hub.clone()));
            routes =
                routes.merge(crate::http::ws::routes(hub.clone(), self.catalog.signal_topics()));
        }
        routes = routes.merge(crate::ops::filter_parse::routes(
            crate::ops::filter_parse::FilterParseState {
                lists: self.catalog.lists(),
                model: self.catalog.text_model(),
            },
        ));
        routes = routes.merge(crate::ops::custom_fields::routes(
            crate::ops::custom_fields::CustomFieldsState {
                fields: self.catalog.custom_fields(),
                entitlements: self.entitlements.clone().unwrap_or_default(),
            },
        ));
        routes = routes.merge(crate::ops::texts::routes(crate::ops::texts::TextsState {
            overrides: self.catalog.text_overrides(),
            translations: self.translations.clone(),
        }));
        routes = routes.merge(crate::blocks::routes(crate::blocks::BlocksState {
            blocks: self.catalog.blocks(),
            translations: self.translations.clone(),
        }));
        routes = routes.merge(crate::ops::connections::routes(
            crate::ops::connections::ConnectionsState { probes: self.catalog.connection_probes() },
        ));
        if let Some((_, permissions)) = &self.resources {
            routes = routes.merge(crate::ops::reasons::routes(crate::ops::reasons::ReasonsState {
                codes: self.catalog.reason_codes(),
                permissions: permissions.clone(),
                translations: self.translations.clone(),
            }));
        }
        if let (Some(outbox), Some((resources, _))) = (&self.outbox, &self.resources) {
            routes = routes.merge(crate::ops::tags::routes(crate::ops::tags::TagsState {
                taggables: self.catalog.taggables(),
                resources: resources.clone(),
                outbox: outbox.clone(),
                settings: self.settings.clone().map(|settings| {
                    let settings: Arc<dyn crate::tenancy::settings::TenantSettings> = settings;
                    settings
                }),
            }));
        }
        if let (Some(outbox), Some((resources, permissions))) = (&self.outbox, &self.resources) {
            routes = routes.merge(crate::ops::records::routes(crate::ops::records::RecordsState {
                resources: resources.clone(),
                permissions: permissions.clone(),
                outbox: outbox.clone(),
                history: self.catalog.history(),
                references: self.catalog.references(),
            }));
        }
        if let (Some(jobs), Some(entitlements), Some(registry)) =
            (&self.jobs, &self.entitlements, &self.registry)
        {
            routes = routes.merge(crate::ops::list_exports::routes(
                crate::ops::list_exports::ExportsState {
                    sources: self.catalog.export_sources(),
                    lists: self.catalog.lists(),
                    jobs: jobs.clone(),
                    entitlements: entitlements.clone(),
                    registry: registry.clone(),
                    fields: self.catalog.custom_fields(),
                },
            ));
            routes = routes.merge(crate::ops::bulk::routes(crate::ops::bulk::BulkState {
                actions: self.catalog.bulk_actions(),
                sources: self.catalog.export_sources(),
                lists: self.catalog.lists(),
                jobs: jobs.clone(),
                entitlements: entitlements.clone(),
            }));
        }
        if let Some(entitlements) = &self.entitlements {
            let state = crate::ops::work::WorkState::new(
                self.catalog.work_sources(),
                entitlements.clone(),
                self.translations.clone(),
            );
            routes = routes.merge(crate::ops::work::routes(state));
            let find = crate::ops::find::FindState {
                sources: self.catalog.finder_sources(),
                entitlements: entitlements.clone(),
            };
            routes = routes.merge(crate::ops::find::routes(find));
            routes = routes.merge(crate::ops::setup::routes(crate::ops::setup::SetupState {
                steps: self.catalog.setup_steps(),
                entitlements: entitlements.clone(),
                translations: self.translations.clone(),
                settings: self.settings.clone().map(|settings| {
                    let settings: Arc<dyn crate::tenancy::settings::TenantSettings> = settings;
                    settings
                }),
            }));
            routes = routes.merge(crate::ops::resolve::routes(crate::ops::resolve::ResolveState {
                resolvers: self.catalog.code_resolvers(),
                entitlements: entitlements.clone(),
            }));
            let summary = crate::ops::summary::SummaryState {
                metrics: self.catalog.summary_metrics(),
                entitlements: entitlements.clone(),
                translations: self.translations.clone(),
            };
            routes = routes.merge(crate::ops::summary::routes(summary));
        }
        if let (Some(settings), Some(registry)) = (&self.settings, &self.registry) {
            routes =
                routes.merge(crate::tenancy::settings::routes(settings.clone(), registry.clone()));
        }
        if let Some(accounts) = &self.accounts {
            routes = routes.merge(crate::provider::routes(accounts.clone()));
        }
        if let Some(admin) = &self.admin {
            routes = routes.merge(crate::tenancy::admin::routes(admin.clone()));
        }
        if let Some(domains) = &self.domains {
            routes = routes.merge(crate::tenancy::domains::routes(domains.clone()));
        }
        if let Some(moves) = &self.moves {
            routes = routes.merge(crate::tenancy::moves::routes(moves.clone()));
        }
        if let Some(exports) = self.live_exports() {
            routes = routes.merge(crate::tenancy::exports::routes(exports.clone()));
        }
        if let Some(runs) = &self.migration_runs {
            routes = routes.merge(crate::tenancy::migration_runs::routes(runs.clone()));
        }
        routes = routes.merge(self.relocations.routes());
        routes = routes.merge(self.sandboxes.routes());
        routes
    }

    fn client_config(&self) -> Result<Option<crate::module::ClientSection>, Error> {
        let section = crate::ops::client_config::KernelClientConfig {
            public_url: self.public_url.base().ok().map(str::to_owned),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        crate::module::ClientSection::new(&section).map(Some)
    }

    fn subscriptions(&self) -> Vec<crate::event::Subscription> {
        // Tài khoản bị xoá: xoá cờ của người đó.
        self.flags.get().map_or_else(Vec::new, |flags| {
            vec![crate::event::Subscription::new(crate::privacy::USER_DELETED, flags)]
        })
    }

    fn description(&self) -> &'static str {
        "The core: tenants and their databases, domains, settings, the module catalog, \
         navigation, background jobs and the shared tools of every module."
    }

    fn mcp_resources(&self) -> Vec<crate::mcp::McpResource> {
        self.catalog_state().map_or_else(Vec::new, module_catalog::resources)
    }

    fn tools(&self) -> Vec<crate::mcp::Tool> {
        let mut tools = self.catalog_state().map_or_else(Vec::new, module_catalog::tools);
        if let Some(state) = self.backlog_state() {
            tools.extend(crate::job::backlog_api::tools(state));
        }
        if let Some(state) = self.navigation_state() {
            tools.extend(crate::ops::navigation::tools(state));
        }
        if let Some(settings) = &self.settings {
            tools.extend(crate::tenancy::settings::tools(settings.clone()));
            tools.extend(crate::ops::connections::tools(self.catalog.connection_probes()));
        }
        if let Some(accounts) = &self.accounts {
            tools.extend(crate::provider::tools(accounts.clone()));
        }
        if let Some(admin) = &self.admin {
            tools.extend(crate::tenancy::admin::tools(admin.clone()));
        }
        if let Some(domains) = &self.domains {
            tools.extend(crate::tenancy::domains::tools(domains.clone()));
        }
        if let Some(moves) = &self.moves {
            tools.extend(crate::tenancy::moves::tools(moves.clone()));
        }
        if let Some(exports) = self.live_exports() {
            tools.extend(crate::tenancy::exports::tools(exports.clone()));
        }
        if let Some(runs) = &self.migration_runs {
            tools.extend(crate::tenancy::migration_runs::tools(runs.clone()));
        }
        tools.extend(crate::ops::request_log::tools());
        tools.extend(self.sandboxes.tools());
        tools.extend(crate::ops::custom_fields::tools(self.catalog.custom_fields()));
        if let Some(outbox) = &self.outbox {
            tools.extend(crate::ops::approvals::tools(crate::ops::approvals::ApprovalsState {
                outbox: outbox.clone(),
            }));
        }
        tools
    }

    fn jobs(&self) -> Vec<crate::job::JobRegistration> {
        let mut jobs = Vec::new();
        if let Some(search) = &self.search {
            jobs.push(crate::job::JobRegistration::new(
                crate::search::SEARCH_SYNC,
                Arc::new(crate::search::SyncRunner(search.clone())),
            ));
        }
        if let Some(registry) = &self.registry {
            jobs.push(crate::job::JobRegistration::new(
                crate::ops::list_exports::EXPORT_LIST,
                Arc::new(crate::ops::list_exports::ExportRunner {
                    sources: self.catalog.export_sources(),
                    registry: registry.clone(),
                    retention_days: self.job_file_days,
                }),
            ));
            jobs.push(crate::job::JobRegistration::new(
                crate::ops::bulk::BULK,
                Arc::new(crate::ops::bulk::BulkRunner { actions: self.catalog.bulk_actions() }),
            ));
            jobs.push(crate::job::JobRegistration::new(
                crate::ops::list_exports::CLEAN_FILES,
                Arc::new(crate::ops::list_exports::CleanRunner { registry: registry.clone() }),
            ));
        }
        if let Some(domains) = &self.domains {
            jobs.push(crate::job::JobRegistration::new(
                crate::tenancy::domains::VERIFY_DOMAINS,
                Arc::new(crate::tenancy::domains::VerifyDomains(domains.clone())),
            ));
        }
        if let Some(moves) = &self.moves {
            jobs.push(crate::job::JobRegistration::new(
                crate::tenancy::moves::MOVE_TENANT,
                Arc::new(crate::tenancy::moves::MoveTenantRunner(moves.clone())),
            ));
        }
        if let Some(exports) = self.live_exports() {
            jobs.push(crate::job::JobRegistration::new(
                crate::tenancy::exports::EXPORT_TENANT,
                Arc::new(crate::tenancy::exports::ExportRunner(exports.clone())),
            ));
            jobs.push(crate::job::JobRegistration::new(
                crate::tenancy::exports::CLEAN_EXPORTS,
                Arc::new(crate::tenancy::exports::CleanRunner(exports.clone())),
            ));
        }
        if let Some(runs) = &self.migration_runs {
            jobs.push(crate::job::JobRegistration::new(
                crate::tenancy::migration_runs::MIGRATE_PLACEMENTS,
                Arc::new(crate::tenancy::migration_runs::MigrateRunner(runs.clone())),
            ));
        }
        jobs.extend(self.relocations.jobs());
        if let Some(backups) = &self.backups {
            jobs.push(
                crate::job::JobRegistration::new(
                    crate::backup::scheduled::BACKUP_JOB,
                    Arc::new(crate::backup::scheduled::BackupRunner(backups.clone())),
                )
                .timeout(crate::backup::scheduled::BACKUP_TIMEOUT),
            );
        }
        jobs.extend(self.sandboxes.jobs());
        jobs
    }

    fn checks(&self) -> Vec<crate::doctor::Check> {
        match &self.registry {
            Some(registry) => vec![
                crate::doctor::Check::new(
                    "backup_age",
                    "Every database has a recent scheduled backup.",
                    Arc::new(crate::backup::scheduled::BackupAge {
                        registry: registry.clone(),
                        config: self.backup_config,
                    }),
                ),
                crate::doctor::Check::new(
                    "placements",
                    "Every placement (tenant database) has its migrations applied.",
                    crate::tenancy::placement_health(registry.clone()),
                ),
            ],
            None => Vec::new(),
        }
    }

    fn tasks(&self) -> Vec<crate::task::Task> {
        match (&self.search, &self.registry) {
            (Some(search), Some(registry)) => {
                vec![crate::search::reindex_task(search.clone(), registry.clone())]
            }
            _ => Vec::new(),
        }
    }

    fn schedules(&self) -> Vec<crate::job::Schedule> {
        // Tên miền riêng chỉ có ở chế độ `multi`: chế độ `single` không thêm job nào mỗi 10 phút.
        let mut schedules = match &self.domains {
            Some(domains) if domains.multi_tenant() => vec![crate::tenancy::domains::schedule()],
            _ => Vec::new(),
        };
        if self.live_exports().is_some() {
            schedules.push(crate::tenancy::exports::schedule());
        }
        if self.registry.as_ref().is_some_and(|registry| registry.shared_storage().is_some()) {
            schedules.push(crate::ops::list_exports::schedule());
        }
        // Không có nơi lưu file (module `files`) thì không có chỗ ghi: `autoxide doctor` báo.
        if self.registry.as_ref().is_some_and(|registry| registry.shared_storage().is_some())
            && let Some(schedule) = self.backup_config.schedule()
        {
            schedules.push(schedule);
        }
        let multi = self.domains.as_ref().is_some_and(|domains| domains.multi_tenant());
        schedules.extend(self.sandboxes.schedules(multi));
        schedules
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        self.jobs = Some(context.jobs());
        self.resources = Some((context.resources(), context.permissions().clone()));
        self.database = Some(context.database().clone());
        self.translations = context.translations().clone();
        self.entitlements = Some(context.entitlements());
        self.outbox = Some(context.outbox()?);
        // Stream event SSE (bước 40 P5): một hub mỗi tiến trình; `serve` đóng nó khi tắt.
        let hub = crate::event::StreamHub::new(self.stream_config.clone(), context.events());
        self.stream = Some(hub.clone());
        context.provide::<crate::event::StreamHub>(Arc::new(hub))?;
        // Chỉ mục tìm kiếm thay được (bước 40 P7): module khai loại tài liệu, bộ máy theo cấu hình.
        let search = crate::search::SearchIndex::new(&self.search_backend);
        self.search = Some(search.clone());
        context.provide::<crate::search::SearchIndex>(Arc::new(search))?;
        // Yêu cầu duyệt của agent vào hộp việc của người quyết được (bước 51 P12).
        context.work_source(Arc::new(crate::ops::approvals::ApprovalsWork))?;
        let flags = Arc::new(UserFlags::new(self.max_flags));
        context.personal_data(flags.clone())?;
        self.flags.fill(flags);
        let key = self.keys.key_for(context.database());
        // Sổ database mở URL placement đã mã hoá bằng khoá gốc này.
        context.tenancy().use_secret_key(key.clone());
        let settings = Arc::new(crate::tenancy::settings::SettingsService::new(
            self.tenant_settings.clone(),
            key.clone(),
            context.outbox()?,
            std::time::Duration::from_secs(context.tenancy().config().cache_seconds),
        ));
        self.registry = Some(context.tenancy().clone());
        self.settings = Some(settings.clone());
        let accounts = crate::provider::ProviderAccounts::new(
            settings.clone(),
            context.tenancy().clone(),
            key.clone(),
            self.public_url.clone(),
        );
        self.accounts = Some(accounts.clone());
        context.provide::<crate::provider::ProviderAccounts>(Arc::new(accounts))?;
        let admin = Arc::new(
            crate::tenancy::TenantAdmin::new(
                context.tenancy().clone(),
                self.seeds.clone(),
                context.outbox()?,
            )
            .with_entitlements(context.entitlements())
            .with_setup(self.move_setup.clone()),
        );
        self.admin = Some(admin.clone());
        self.sandboxes.start(context, admin.clone())?;
        context.provide::<crate::tenancy::TenantAdmin>(admin)?;
        let platform_host =
            self.public_url.base().ok().and_then(crate::tenancy::domains::host_of_url);
        let domains = Arc::new(crate::tenancy::TenantDomains::new(
            context.tenancy().clone(),
            context.outbox()?,
            self.dns.clone(),
            self.orgs.clone(),
            platform_host.clone(),
        ));
        self.domains = Some(domains.clone());
        // Link riêng của khách trong mail chỉ trỏ được tới host khách sở hữu (D-20261125-tenant-link-templates).
        let trust = crate::tenancy::settings::LinkTrust::new(
            Some(context.tenancy().clone()),
            platform_host,
        );
        settings.use_link_trust(trust.clone());
        let links = crate::tenancy::SiteLinks::new(settings.clone()).with_trust(trust);
        context.provide::<crate::tenancy::SiteLinks>(Arc::new(links))?;
        context.provide::<crate::tenancy::TenantDomains>(domains)?;
        let moves = Arc::new(crate::tenancy::TenantMoves::new(
            context.tenancy().clone(),
            self.move_setup.clone(),
            context.outbox()?,
            context.jobs(),
            self.request_timeout,
            context.translations().clone(),
        ));
        self.moves = Some(moves.clone());
        context.provide::<crate::tenancy::TenantMoves>(moves)?;
        self.relocations.start(
            context,
            self.move_setup.clone(),
            key.clone(),
            self.seeds.clone(),
            self.request_timeout,
        )?;
        let exports = Arc::new(crate::tenancy::TenantExports::new(
            context.tenancy().clone(),
            self.move_setup.clone(),
            context.jobs(),
            context.outbox()?,
            self.export_days,
        ));
        self.exports = Some(exports.clone());
        context.provide::<crate::tenancy::TenantExports>(exports)?;
        let backups = Arc::new(crate::backup::scheduled::ScheduledBackups::new(
            context.tenancy().clone(),
            self.move_setup.clone(),
            self.backup_config,
        ));
        self.backups = Some(backups.clone());
        context.provide::<crate::backup::scheduled::ScheduledBackups>(backups)?;
        let runs = Arc::new(crate::tenancy::migration_runs::MigrationRuns::new(
            context.tenancy().clone(),
            self.move_setup.clone(),
            context.jobs(),
        ));
        self.migration_runs = Some(runs.clone());
        context.provide::<crate::tenancy::PlacementMigrationQueue>(Arc::new(
            crate::tenancy::PlacementMigrationQueue::new(runs),
        ))?;
        context.provide::<dyn crate::tenancy::TenantSettings>(settings)?;
        context.provide::<SecretKey>(Arc::new(key))?;
        context.provide::<crate::template::TemplateEngine>(Arc::new(self.templates.clone()))?;
        context.provide::<PublicUrl>(Arc::new(self.public_url.clone()))
    }
}

/// Staff of the platform tenant enter another tenant with the `Autoxide-Tenant` header (step 33). Permissions
/// keep the same permission names they already have in the target tenant.
pub(crate) const TENANTS_OPERATE: Permission = Permission::new(
    "kernel.tenants.operate",
    "Act inside another tenant with the header `Autoxide-Tenant` (platform staff only: it works \
     only for accounts of the platform tenant).",
)
.with_risk(crate::access::Risk::SensitiveData);

/// Table `kernel_secrets`: one row per key the program generates itself (name → value).
struct CreateSecrets;

impl MigrationName for CreateSecrets {
    fn name(&self) -> &str {
        "m0001_create_secrets"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateSecrets {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("kernel_secrets")
                    .col(schema::string_len("name", 64).primary_key())
                    .col(schema::text("value"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("kernel_secrets").to_owned()).await
    }
}

/// `AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS`, already read by the kernel settings (read before modules).
fn request_timeout(reader: &ConfigReader<'_>) -> Option<u64> {
    reader
        .values()
        .iter()
        .find(|value| value.setting.name == "AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS")
        .and_then(|value| value.display_value().trim().parse().ok())
}

/// The kernel settings (read before modules) set `AUTOXIDE_TENANCY_MODE=multi`.
fn multi_tenant(reader: &ConfigReader<'_>) -> bool {
    reader.values().iter().any(|value| {
        value.setting.name == "AUTOXIDE_TENANCY_MODE" && value.display_value().trim() == "multi"
    })
}
