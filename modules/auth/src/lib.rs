//! Module `auth`: tài khoản người dùng và mật khẩu.
//!
//! Đây là module của core, không phải module nghiệp vụ: mọi bản cài Autoxide đều
//! cần nó. Hiện có: người dùng với mật khẩu hash bằng Argon2id (D-044), đăng nhập
//! bằng access token và refresh token xoay vòng (D-045). Các mục sau của lộ trình
//! thêm role, permission và API key vào chính module này.
//!
//! Module khác KHÔNG đọc bảng `auth_…`; chúng lấy người dùng qua trait
//! [`api::Users`] mà module này công bố lúc khởi động.

pub mod api;
mod config;
mod http;
mod migration;
mod password;
mod session;
mod token;
mod user;

use std::sync::Arc;

use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::config::AuthConfig;
use crate::password::Passwords;
use crate::session::SessionService;
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
    sessions: Option<Arc<SessionService>>,
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
        let sessions =
            SessionService::new(database, users.clone(), access_tokens, config.refresh_token_ttl)?;
        self.sessions = Some(Arc::new(sessions));
        // `Arc<UserService>` tự đổi thành `Arc<dyn Users>`: module khác chỉ thấy trait.
        context.provide::<dyn api::Users>(users)
    }

    fn routes(&self) -> OpenApiRouter {
        match &self.sessions {
            Some(sessions) => http::routes(sessions.clone()),
            None => OpenApiRouter::new(),
        }
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

    /// Service phiên đăng nhập trên database của test, với khoá cố định, kèm
    /// service người dùng dùng chung database đó.
    pub async fn sessions(
        test: &TestDatabase,
    ) -> (std::sync::Arc<crate::session::SessionService>, std::sync::Arc<crate::user::UserService>)
    {
        let database = database(test).await;
        let users = std::sync::Arc::new(crate::user::UserService::new(
            database.clone(),
            crate::password::Passwords::new(),
        ));
        let tokens = crate::token::AccessTokens::new(
            b"test-key-test-key-test-key-test-key".to_vec(),
            std::time::Duration::from_secs(900),
        );
        let sessions = crate::session::SessionService::new(
            database,
            users.clone(),
            tokens,
            std::time::Duration::from_secs(30 * 24 * 60 * 60),
        )
        .unwrap();
        (std::sync::Arc::new(sessions), users)
    }
}
