//! Module nội bộ `kernel`: luôn được đăng ký đầu tiên (trong [`crate::module::Modules::new`]),
//! giữ những bảng mà cả chương trình dùng chung, không thuộc module nghiệp vụ nào — hiện là
//! `kernel_secrets` (khoá ký, [`crate::secret`]). Nó cũng đọc và công bố URL công khai của API
//! ([`crate::public_url`]), phục vụ sổ module đã cài ([`crate::module_catalog`], D-155) và cờ
//! theo người dùng dưới `/api/v1/me` ([`crate::user_flags`], D-207). Là
//! một module như mọi module khác nên luật sở hữu bảng, `autoxide backup` / `copy` và `migrate` áp cho nó y như vậy.

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

/// Tên của module nội bộ; module khác không được dùng tên này.
pub(crate) const NAME: &str = "kernel";

pub(crate) struct KernelModule {
    /// Giá trị `AUTOXIDE_SECRET_KEY` và `AUTOXIDE_SECRET_KEY_PREVIOUS`; điền lúc `configure`, dùng
    /// chung với `Migrator`.
    keys: crate::secret::KeyConfig,
    /// Giá trị `AUTOXIDE_HTTP_PUBLIC_URL`; có sau `configure`.
    public_url: PublicUrl,
    /// Sổ module, do `Modules::start` điền sau khi mọi module khởi động.
    catalog: ModuleCatalog,
    /// Outbox của kernel; có sau `start` (event `kernel.job_retried`).
    outbox: Option<crate::event::Outbox>,
    /// Quyền dùng theo gói (bước 35); có sau `start` (sổ điều hướng lọc theo đó).
    entitlements: Option<crate::entitlement::EntitlementsHandle>,
    /// Cảnh báo vận hành (bước kiểm của mọi module, giữ kết quả một phút).
    alerts: Arc<crate::ops::alerts::Alerts>,
    /// Cấu hình cho giao diện của mọi module (D-271), điền cùng lúc với sổ module.
    client_configs: crate::ops::client_config::ClientConfigs,
    /// Database của chương trình; có sau `start` (để đọc trạng thái migration).
    database: Option<crate::database::Database>,
    /// `AUTOXIDE_USER_FLAGS_MAX`; có sau `configure`.
    max_flags: u64,
    /// Bộ dựng mẫu dùng chung (`AUTOXIDE_TEMPLATE_MAX_BYTES`, D-351); công bố lúc `start`.
    templates: crate::template::TemplateEngine,
    /// Service cờ theo người dùng, dùng chung với `Modules::routes`; điền lúc `start`.
    flags: user_flags::Handle,
    /// Sổ cấu hình theo khách của mọi module (điền khi dựng `Modules`).
    tenant_settings: crate::tenancy::settings::Catalog,
    /// Service cấu hình theo khách; có sau `start`.
    settings: Option<Arc<crate::tenancy::settings::SettingsService>>,
    /// Sổ database; có sau `start` (nhân sự nền tảng đặt cấu hình của khách khác).
    registry: Option<crate::tenancy::DatabaseRegistry>,
    /// Bộ gieo dữ liệu cho khách mới của mọi module.
    seeds: crate::tenancy::admin::Seeds,
    /// Service quản lý khách; có sau `start`.
    admin: Option<Arc<crate::tenancy::TenantAdmin>>,
    /// Bộ tra DNS cho tên miền riêng (điền sau khi mọi module khởi động).
    dns: crate::tenancy::domains::DnsSlot,
    /// Cây tổ chức (bước 34) để gắn tên miền với nút (điền sau khi mọi module khởi động).
    orgs: crate::tenancy::domains::OrgSlot,
    /// Service tên miền riêng của khách; có sau `start`.
    domains: Option<Arc<crate::tenancy::TenantDomains>>,
    /// Thứ việc dời khách cần từ mọi module (điền sau khi mọi module khởi động).
    move_setup: crate::tenancy::moves::MoveSetup,
    /// Service dời khách giữa các placement; có sau `start`.
    moves: Option<Arc<crate::tenancy::TenantMoves>>,
    /// Xuất dữ liệu khách (bước 35, D-328); có sau `start`.
    exports: Option<Arc<crate::tenancy::TenantExports>>,
    /// Migrate placement bằng API (D-334); có sau `start`.
    migration_runs: Option<Arc<crate::tenancy::migration_runs::MigrationRuns>>,
    /// `AUTOXIDE_TENANCY_EXPORT_RETENTION_DAYS`.
    export_days: i64,
    /// `AUTOXIDE_JOBS_FILE_RETENTION_DAYS` (bước 51 P16).
    job_file_days: u32,
    /// Hàng đợi job của kernel (xuất danh sách); có sau `start`.
    jobs: Option<crate::job::JobQueue>,
    /// Sổ đối tượng và sổ permission (ghi chú trên bản ghi, bước 51 P13); có sau `start`.
    resources: Option<(crate::resource::ResourceDirectory, crate::access::PermissionCatalog)>,
    /// `AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS` (đọc ở cấu hình kernel): dời khách chờ request đang
    /// chạy xong trong chừng này.
    request_timeout: std::time::Duration,
    /// Bản dịch của mọi module (dịch mô tả permission, event trong sổ module); có sau `start`.
    translations: crate::i18n::Translations,
}

impl KernelModule {
    pub(crate) fn new(
        catalog: ModuleCatalog,
        client_configs: crate::ops::client_config::ClientConfigs,
        flags: user_flags::Handle,
        keys: crate::secret::KeyConfig,
        tenant_settings: crate::tenancy::settings::Catalog,
        slots: crate::tenancy::Slots,
    ) -> Self {
        let crate::tenancy::Slots { seeds, dns, orgs, moves: move_setup } = slots;
        KernelModule {
            dns,
            orgs,
            domains: None,
            move_setup,
            moves: None,
            exports: None,
            migration_runs: None,
            export_days: 30,
            job_file_days: 7,
            jobs: None,
            resources: None,
            request_timeout: std::time::Duration::from_secs(30),
            keys,
            tenant_settings,
            settings: None,
            registry: None,
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
        }
    }

    /// Xuất dữ liệu khách, chỉ khi có nơi lưu file (module `files`): không có nơi để ghi bản xuất
    /// thì không có route, tool, job hay lịch nào của nó.
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

    /// Có sau `start` (cần quyền dùng theo gói).
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
        ]
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
        let job_file_days = reader
            .get_with(&crate::ops::list_exports::FILE_RETENTION_DAYS, |raw| {
                crate::config::parse_in_range(raw, 1u32, 365)
            });
        if let Some(seconds) = request_timeout(reader) {
            self.request_timeout = std::time::Duration::from_secs(seconds);
        }
        self.keys.set(secret?, previous?);
        self.public_url = public_url?;
        self.max_flags = max_flags?;
        self.templates = crate::template::TemplateEngine::new(template_bytes?);
        self.export_days = export_days?;
        self.job_file_days = job_file_days?;
        self.catalog.text_overrides().set_ui_max(ui_texts?);
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
        ]
    }

    fn events(&self) -> Vec<crate::event::EventInfo> {
        vec![
            crate::tenancy::settings::SETTING_CHANGED,
            crate::job::runs::JOB_RETRIED,
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
            crate::tenancy::erasure::TENANT_DELETED,
        ]
    }

    fn limits(&self) -> Vec<crate::entitlement::Limit> {
        vec![crate::entitlement::API_REQUESTS, crate::ops::custom_fields::CUSTOM_FIELDS_LIMIT]
    }

    fn tenant_settings(&self) -> Vec<crate::tenancy::TenantSetting> {
        vec![
            crate::tenancy::PUBLIC_URL,
            crate::tenancy::BUSINESS_MODEL,
            crate::ops::tags::TAGS_RESTRICTED,
        ]
    }

    fn routes(&self) -> OpenApiRouter {
        let catalog = self.catalog_state().map_or_else(OpenApiRouter::new, module_catalog::routes);
        let mut routes = catalog
            .merge(crate::ops::client_config::routes(self.client_configs.clone()))
            .merge(crate::ops::whats_new::routes())
            .merge(crate::ops::alerts::routes(self.alerts.clone()));
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
        if let Some(state) = self.navigation_state() {
            tools.extend(crate::ops::navigation::tools(state));
        }
        if let Some(settings) = &self.settings {
            tools.extend(crate::tenancy::settings::tools(settings.clone()));
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
        jobs
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
        schedules
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        self.jobs = Some(context.jobs());
        self.resources = Some((context.resources(), context.permissions().clone()));
        self.database = Some(context.database().clone());
        self.translations = context.translations().clone();
        self.entitlements = Some(context.entitlements());
        self.outbox = Some(context.outbox()?);
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
        context.provide::<crate::tenancy::TenantAdmin>(admin)?;
        let platform_host =
            self.public_url.base().ok().and_then(crate::tenancy::domains::host_of_url);
        let domains = Arc::new(crate::tenancy::TenantDomains::new(
            context.tenancy().clone(),
            context.outbox()?,
            self.dns.clone(),
            self.orgs.clone(),
            platform_host,
        ));
        self.domains = Some(domains.clone());
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
        let exports = Arc::new(crate::tenancy::TenantExports::new(
            context.tenancy().clone(),
            self.move_setup.clone(),
            context.jobs(),
            self.export_days,
        ));
        self.exports = Some(exports.clone());
        context.provide::<crate::tenancy::TenantExports>(exports)?;
        self.migration_runs = Some(Arc::new(crate::tenancy::migration_runs::MigrationRuns::new(
            context.tenancy().clone(),
            self.move_setup.clone(),
            context.jobs(),
        )));
        context.provide::<dyn crate::tenancy::TenantSettings>(settings)?;
        context.provide::<SecretKey>(Arc::new(key))?;
        context.provide::<crate::template::TemplateEngine>(Arc::new(self.templates.clone()))?;
        context.provide::<PublicUrl>(Arc::new(self.public_url.clone()))
    }
}

/// Nhân sự của khách nền tảng vào khách khác bằng header `Autoxide-Tenant` (bước 33). Quyền
/// giữ nguyên tên permission đã có trong khách đích.
pub(crate) const TENANTS_OPERATE: Permission = Permission::new(
    "kernel.tenants.operate",
    "Act inside another tenant with the header `Autoxide-Tenant` (platform staff only: it works \
     only for accounts of the platform tenant).",
)
.with_risk(crate::access::Risk::SensitiveData);

/// Bảng `kernel_secrets`: mỗi dòng một khoá chương trình tự sinh (tên → giá trị).
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

/// `AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS` mà cấu hình kernel (đọc trước module) đã đọc.
fn request_timeout(reader: &ConfigReader<'_>) -> Option<u64> {
    reader
        .values()
        .iter()
        .find(|value| value.setting.name == "AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS")
        .and_then(|value| value.display_value().trim().parse().ok())
}

/// Cấu hình kernel (đọc trước module) đặt `AUTOXIDE_TENANCY_MODE=multi`.
fn multi_tenant(reader: &ConfigReader<'_>) -> bool {
    reader.values().iter().any(|value| {
        value.setting.name == "AUTOXIDE_TENANCY_MODE" && value.display_value().trim() == "multi"
    })
}
