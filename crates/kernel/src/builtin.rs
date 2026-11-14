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
    /// Giá trị `AUTOXIDE_SECRET_KEY`; có sau `configure`.
    secret: Option<Vec<u8>>,
    /// Giá trị `AUTOXIDE_HTTP_PUBLIC_URL`; có sau `configure`.
    public_url: PublicUrl,
    /// Sổ module, do `Modules::start` điền sau khi mọi module khởi động.
    catalog: ModuleCatalog,
    /// Database của chương trình; có sau `start` (để đọc trạng thái migration).
    database: Option<crate::database::Database>,
    /// `AUTOXIDE_USER_FLAGS_MAX`; có sau `configure`.
    max_flags: u64,
    /// Service cờ theo người dùng, dùng chung với `Modules::routes`; điền lúc `start`.
    flags: user_flags::Handle,
}

impl KernelModule {
    pub(crate) fn new(catalog: ModuleCatalog, flags: user_flags::Handle) -> Self {
        KernelModule {
            secret: None,
            public_url: PublicUrl::default(),
            catalog,
            database: None,
            max_flags: 500,
            flags,
        }
    }

    fn catalog_state(&self) -> Option<CatalogState> {
        let database = self.database.clone()?;
        Some(CatalogState { catalog: self.catalog.clone(), database })
    }
}

impl Module for KernelModule {
    fn name(&self) -> &'static str {
        NAME
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateSecrets), Box::new(user_flags::CreateUserFlags)]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        let secret = crate::secret::read_setting(reader);
        let public_url = crate::public_url::read_setting(reader);
        let max_flags = user_flags::read_setting(reader);
        self.secret = secret?;
        self.public_url = public_url?;
        self.max_flags = max_flags?;
        Ok(())
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![module_catalog::MODULES_READ]
    }

    fn routes(&self) -> OpenApiRouter {
        self.catalog_state().map_or_else(OpenApiRouter::new, module_catalog::routes)
    }

    fn subscriptions(&self) -> Vec<crate::event::Subscription> {
        // Tài khoản bị xoá: xoá cờ của người đó.
        self.flags.get().map_or_else(Vec::new, |flags| {
            vec![crate::event::Subscription::new(crate::privacy::USER_DELETED, flags)]
        })
    }

    fn tools(&self) -> Vec<crate::mcp::Tool> {
        self.catalog_state().map_or_else(Vec::new, module_catalog::tools)
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        self.database = Some(context.database().clone());
        let flags = Arc::new(UserFlags::new(context.database().clone(), self.max_flags));
        context.personal_data(flags.clone())?;
        self.flags.fill(flags);
        let key = SecretKey::new(context.database().clone(), self.secret.clone());
        context.provide::<SecretKey>(Arc::new(key))?;
        context.provide::<PublicUrl>(Arc::new(self.public_url.clone()))
    }
}

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
