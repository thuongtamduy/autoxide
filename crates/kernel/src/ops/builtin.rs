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
    /// Cấu hình cho giao diện của mọi module (D-271), điền cùng lúc với sổ module.
    client_configs: crate::ops::client_config::ClientConfigs,
    /// Database của chương trình; có sau `start` (để đọc trạng thái migration).
    database: Option<crate::database::Database>,
    /// `AUTOXIDE_USER_FLAGS_MAX`; có sau `configure`.
    max_flags: u64,
    /// Service cờ theo người dùng, dùng chung với `Modules::routes`; điền lúc `start`.
    flags: user_flags::Handle,
    /// Sổ cấu hình theo khách của mọi module (điền khi dựng `Modules`).
    tenant_settings: crate::tenancy::settings::Catalog,
    /// Service cấu hình theo khách; có sau `start`.
    settings: Option<Arc<crate::tenancy::settings::SettingsService>>,
    /// Bộ gieo dữ liệu cho khách mới của mọi module.
    seeds: crate::tenancy::admin::Seeds,
    /// Service quản lý khách; có sau `start`.
    admin: Option<Arc<crate::tenancy::TenantAdmin>>,
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
        seeds: crate::tenancy::admin::Seeds,
    ) -> Self {
        KernelModule {
            keys,
            tenant_settings,
            settings: None,
            seeds,
            admin: None,
            public_url: PublicUrl::default(),
            catalog,
            client_configs,
            database: None,
            max_flags: 500,
            flags,
            translations: crate::i18n::Translations::default(),
        }
    }

    fn catalog_state(&self) -> Option<CatalogState> {
        let database = self.database.clone()?;
        Some(CatalogState {
            catalog: self.catalog.clone(),
            database,
            translations: self.translations.clone(),
        })
    }
}

impl Module for KernelModule {
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
        ]
    }

    fn control_plane_tables(&self) -> Vec<&'static str> {
        // Khoá ký gốc của bản cài và sổ khách / vị trí dữ liệu / tên miền (D-253).
        vec!["kernel_secrets", "kernel_tenants", "kernel_placements", "kernel_tenant_domains"]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        let secret = crate::secret::read_setting(reader);
        let previous = crate::secret::read_previous(reader);
        let public_url = crate::public_url::read_setting(reader);
        let max_flags = user_flags::read_setting(reader);
        self.keys.set(secret?, previous?);
        self.public_url = public_url?;
        self.max_flags = max_flags?;
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
        ]
    }

    fn events(&self) -> Vec<crate::event::EventInfo> {
        vec![
            crate::tenancy::settings::SETTING_CHANGED,
            crate::tenancy::admin::TENANT_CREATED,
            crate::tenancy::admin::TENANT_UPDATED,
        ]
    }

    fn tenant_settings(&self) -> Vec<crate::tenancy::TenantSetting> {
        vec![crate::tenancy::PUBLIC_URL]
    }

    fn routes(&self) -> OpenApiRouter {
        let catalog = self.catalog_state().map_or_else(OpenApiRouter::new, module_catalog::routes);
        let mut routes =
            catalog.merge(crate::ops::client_config::routes(self.client_configs.clone()));
        if let Some(settings) = &self.settings {
            routes = routes.merge(crate::tenancy::settings::routes(settings.clone()));
        }
        if let Some(admin) = &self.admin {
            routes = routes.merge(crate::tenancy::admin::routes(admin.clone()));
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

    fn tools(&self) -> Vec<crate::mcp::Tool> {
        let mut tools = self.catalog_state().map_or_else(Vec::new, module_catalog::tools);
        if let Some(settings) = &self.settings {
            tools.extend(crate::tenancy::settings::tools(settings.clone()));
        }
        if let Some(admin) = &self.admin {
            tools.extend(crate::tenancy::admin::tools(admin.clone()));
        }
        tools
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        self.database = Some(context.database().clone());
        self.translations = context.translations().clone();
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
        ));
        self.settings = Some(settings.clone());
        let admin = Arc::new(crate::tenancy::TenantAdmin::new(
            context.tenancy().clone(),
            self.seeds.clone(),
            context.outbox()?,
        ));
        self.admin = Some(admin.clone());
        context.provide::<crate::tenancy::TenantAdmin>(admin)?;
        context.provide::<dyn crate::tenancy::TenantSettings>(settings)?;
        context.provide::<SecretKey>(Arc::new(key))?;
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
