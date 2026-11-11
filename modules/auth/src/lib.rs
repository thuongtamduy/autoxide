//! Module `auth`: tài khoản người dùng và mật khẩu.
//!
//! Đây là module của core, không phải module nghiệp vụ: mọi bản cài Autoxide đều
//! cần nó. Hiện có: người dùng với mật khẩu hash bằng Argon2id (D-044), đăng nhập
//! bằng access token và refresh token xoay vòng (D-045). Các mục sau của lộ trình
//! thêm role, permission và API key vào chính module này.
//!
//! Module khác KHÔNG đọc bảng `auth_…`; chúng lấy người dùng qua trait
//! [`api::Users`] mà module này công bố lúc khởi động.

mod access;
pub mod api;
mod api_key;
mod config;
mod http;
mod mfa;
mod migration;
mod password;
mod permissions;
mod role;
mod session;
mod token;
mod user;

use std::sync::Arc;

use autoxide_kernel::access::{Authenticator, Permission};
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::access::RequestAuthenticator;
use crate::api_key::ApiKeyService;
use crate::config::AuthConfig;
use crate::mfa::MfaService;
use crate::password::Passwords;
use crate::role::RoleService;
use crate::session::{LoginThrottle, SessionService};
use crate::token::AccessTokens;
use crate::user::UserService;

/// Module `auth`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    AuthModule::default()
}

#[derive(Default)]
struct AuthModule {
    /// Có sau `configure`.
    config: Option<AuthConfig>,
    /// Có sau `start`.
    state: Option<http::HttpState>,
}

impl Module for AuthModule {
    fn name(&self) -> &'static str {
        "auth"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(AuthConfig::read(reader)?);
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `auth` started before it was configured"))?;
        let database = context.database().clone();
        let users = Arc::new(UserService::new(database.clone(), Passwords::new()));
        let secret = match config.token_secret {
            Some(secret) => secret,
            None => {
                tracing::warn!(
                    "AUTOXIDE_AUTH_TOKEN_SECRET is not set: using a random key for this \
                     process. Access tokens stop working after a restart and cannot be shared \
                     between servers; set it for production."
                );
                random_secret()?
            }
        };
        let access_tokens = AccessTokens::new(secret, config.access_token_ttl);
        let throttle =
            LoginThrottle::new(database.clone(), config.login_max_failures, config.login_lock)?;
        let mfa = Arc::new(MfaService::new(database.clone(), config.totp_issuer.clone()));
        let sessions = Arc::new(SessionService::new(
            database.clone(),
            users.clone(),
            access_tokens,
            config.refresh_token_ttl,
            throttle,
            mfa.clone(),
        )?);
        let roles = Arc::new(RoleService::new(database.clone(), context.permissions().clone()));
        let api_keys = Arc::new(ApiKeyService::new(database, users.clone(), roles.clone()));
        let authenticator = RequestAuthenticator {
            sessions: sessions.clone(),
            roles: roles.clone(),
            api_keys: api_keys.clone(),
        };
        // `Arc<X>` tự đổi thành `Arc<dyn Trait>`: bên dùng chỉ thấy trait.
        context.provide::<dyn Authenticator>(Arc::new(authenticator))?;
        context.provide::<dyn api::Users>(users.clone())?;
        context.provide::<dyn api::Accounts>(users.clone())?;
        self.state = Some(http::HttpState {
            sessions,
            users,
            roles,
            api_keys,
            mfa,
            self_registration: config.self_registration,
        });
        Ok(())
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn routes(&self) -> OpenApiRouter {
        // Clone trạng thái chỉ là clone vài `Arc`: tăng bộ đếm tham chiếu, không chép
        // service.
        self.state.clone().map_or_else(OpenApiRouter::new, http::routes)
    }
}

/// Khoá ký token ngẫu nhiên 256 bit, khi người cài chưa đặt khoá.
fn random_secret() -> Result<Vec<u8>, Error> {
    let mut secret = vec![0u8; 32];
    getrandom::fill(&mut secret)
        .map_err(|error| Error::internal(format!("random number generator: {error}")))?;
    Ok(secret)
}

#[cfg(test)]
pub(crate) mod testing {
    //! Đồ dùng chung cho test trong crate này.

    use std::sync::Arc;

    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::database::Database;
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_testkit::TestDatabase;

    /// Kết nối tới database trống của test và áp dụng mọi migration của module.
    pub async fn database(test: &TestDatabase) -> Database {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        let database = Database::connect(&config.database).await.unwrap();
        Migrator::new(vec![ModuleMigrations::new("auth", super::migration::all())])
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        database
    }

    /// Mọi service của module trên database của test, với khoá ký cố định.
    pub struct Services {
        pub users: Arc<crate::user::UserService>,
        pub sessions: Arc<crate::session::SessionService>,
        pub roles: Arc<crate::role::RoleService>,
        pub api_keys: Arc<crate::api_key::ApiKeyService>,
        pub mfa: Arc<crate::mfa::MfaService>,
        pub authenticator: Arc<dyn autoxide_kernel::access::Authenticator>,
    }

    pub async fn services(test: &TestDatabase) -> Services {
        let database = database(test).await;
        let users = Arc::new(crate::user::UserService::new(
            database.clone(),
            crate::password::Passwords::new(),
        ));
        let tokens = crate::token::AccessTokens::new(
            b"test-key-test-key-test-key-test-key".to_vec(),
            std::time::Duration::from_secs(900),
        );
        let mfa = Arc::new(crate::mfa::MfaService::new(database.clone(), "Autoxide".to_owned()));
        let sessions = Arc::new(
            crate::session::SessionService::new(
                database.clone(),
                users.clone(),
                tokens,
                std::time::Duration::from_secs(30 * 24 * 60 * 60),
                crate::session::LoginThrottle::new(
                    database.clone(),
                    5,
                    std::time::Duration::from_secs(15 * 60),
                )
                .unwrap(),
                mfa.clone(),
            )
            .unwrap(),
        );
        let catalog = autoxide_kernel::access::PermissionCatalog::new(vec![
            ("auth", crate::permissions::all()),
            (
                "notes",
                vec![autoxide_kernel::access::Permission::new("notes.write", "Write notes.")],
            ),
        ])
        .unwrap();
        let roles = Arc::new(crate::role::RoleService::new(database.clone(), catalog));
        let api_keys =
            Arc::new(crate::api_key::ApiKeyService::new(database, users.clone(), roles.clone()));
        let authenticator = Arc::new(crate::access::RequestAuthenticator {
            sessions: sessions.clone(),
            roles: roles.clone(),
            api_keys: api_keys.clone(),
        });
        Services { users, sessions, roles, api_keys, mfa, authenticator }
    }

    /// Service phiên đăng nhập kèm service người dùng dùng chung database.
    pub async fn sessions(
        test: &TestDatabase,
    ) -> (Arc<crate::session::SessionService>, Arc<crate::user::UserService>) {
        let services = services(test).await;
        (services.sessions, services.users)
    }
}
