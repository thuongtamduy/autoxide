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
mod cleanup;
mod config;
mod doctor;
mod events;
mod http;
mod mfa;
mod migration;
mod passkey;
mod password;
mod permissions;
mod privacy;
mod recovery;
mod reset;
mod role;
mod session;
mod token;
mod tools;
mod user;
mod verify;

use std::sync::Arc;

use autoxide_kernel::access::{Authenticator, Permission};
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mail::Mailer;
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
    cleanup: Option<Arc<cleanup::Cleanup>>,
    resets: Option<Arc<reset::PasswordResets>>,
    administrator: Option<Arc<doctor::Administrator>>,
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
        let outbox = context.outbox()?;
        self.cleanup = Some(Arc::new(cleanup::Cleanup::new(database.clone())));
        self.administrator = Some(Arc::new(doctor::Administrator { database: database.clone() }));
        let users = Arc::new(UserService::new(database.clone(), Passwords::new(), outbox.clone()));
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
        let mailer = context.require::<dyn Mailer>()?;

        let mfa = Arc::new(MfaService::new(
            database.clone(),
            config.totp_issuer.clone(),
            LoginThrottle::new(database.clone(), config.login_max_failures, config.login_lock)?,
            outbox.clone(),
        ));
        let sessions = Arc::new(SessionService::new(
            database.clone(),
            users.clone(),
            access_tokens,
            config.refresh_token_ttl,
            throttle,
            mfa.clone(),
            outbox.clone(),
        )?);
        let passkeys = Arc::new(passkey::PasskeyService::new(
            database.clone(),
            users.clone(),
            sessions.clone(),
            outbox.clone(),
            config.passkey.as_ref(),
        )?);
        let verifications = Arc::new(verify::EmailVerifications {
            database: database.clone(),
            users: users.clone(),
            sessions: sessions.clone(),
            mailer: mailer.clone(),
            translations: context.translations().clone(),
            outbox: outbox.clone(),
            url: config.email_verification_url.clone(),
        });
        let roles = Arc::new(RoleService::new(
            database.clone(),
            context.permissions().clone(),
            outbox.clone(),
        ));
        let api_keys = Arc::new(ApiKeyService::new(
            database.clone(),
            users.clone(),
            roles.clone(),
            sessions.clone(),
            outbox.clone(),
        ));
        let resets = Arc::new(reset::PasswordResets {
            database: database.clone(),
            jobs: context.jobs(),
            users: users.clone(),
            mailer: mailer.clone(),
            translations: context.translations().clone(),
            outbox: outbox.clone(),
            url: config.password_reset_url.clone(),
        });
        let authenticator = RequestAuthenticator {
            sessions: sessions.clone(),
            roles: roles.clone(),
            api_keys: api_keys.clone(),
        };
        // `Arc<X>` tự đổi thành `Arc<dyn Trait>`: bên dùng chỉ thấy trait.
        context.provide::<dyn Authenticator>(Arc::new(authenticator))?;
        context.provide::<dyn api::Users>(users.clone())?;
        context.provide::<dyn api::Accounts>(users.clone())?;
        context.provide::<dyn api::AccountRecovery>(Arc::new(recovery::OperatorRecovery {
            users: users.clone(),
            mfa: mfa.clone(),
        }))?;
        context.personal_data(Arc::new(privacy::AuthPersonalData {
            users: users.clone(),
            roles: roles.clone(),
            sessions: sessions.clone(),
            api_keys: api_keys.clone(),
            passkeys: passkeys.clone(),
            mfa: mfa.clone(),
        }))?;
        self.state = Some(http::HttpState {
            personal_data: context.personal_data_export(),
            sessions,
            users,
            roles,
            api_keys,
            mfa,
            resets: resets.clone(),
            verifications,
            passkeys,
            self_registration: config.self_registration,
        });
        self.resets = Some(resets);
        Ok(())
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn events(&self) -> Vec<EventInfo> {
        events::all()
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        let mut jobs = Vec::new();
        if let Some(cleanup) = &self.cleanup {
            jobs.push(JobRegistration::new(cleanup::KIND, cleanup.clone()));
        }
        if let Some(resets) = &self.resets {
            let mailer = Arc::new(reset::ResetMailer(resets.clone()));
            jobs.push(JobRegistration::new(reset::SEND_RESET, mailer));
        }
        jobs
    }

    fn schedules(&self) -> Vec<Schedule> {
        vec![cleanup::schedule()]
    }

    fn checks(&self) -> Vec<autoxide_kernel::doctor::Check> {
        self.administrator.clone().map_or_else(Vec::new, |probe| {
            vec![autoxide_kernel::doctor::Check::new(
                "administrator",
                "An active account with the `admin` role exists, so someone can manage the server.",
                probe,
            )]
        })
    }

    fn tools(&self) -> Vec<autoxide_kernel::mcp::Tool> {
        self.state.clone().map_or_else(Vec::new, |state| tools::all(&Arc::new(state)))
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

    /// Khoá ký access token cố định của test (`AUTOXIDE_AUTH_TOKEN_SECRET`).
    pub const TOKEN_SECRET: &str = "test-key-test-key-test-key-test-key";
    /// Passkey của test: tên miền và origin (`AUTOXIDE_AUTH_PASSKEY_RP_ID`, `…_ORIGINS`).
    pub const PASSKEY_RP_ID: &str = "example.com";
    pub const PASSKEY_ORIGIN: &str = "https://example.com";

    /// Mọi service của module trên database của test, với khoá ký cố định.
    pub struct Services {
        pub users: Arc<crate::user::UserService>,
        pub sessions: Arc<crate::session::SessionService>,
        pub roles: Arc<crate::role::RoleService>,
        pub api_keys: Arc<crate::api_key::ApiKeyService>,
        pub mfa: Arc<crate::mfa::MfaService>,
        pub resets: Arc<crate::reset::PasswordResets>,
        pub verifications: Arc<crate::verify::EmailVerifications>,
        pub passkeys: Arc<crate::passkey::PasskeyService>,
        pub mails: Arc<FakeMailer>,
        pub database: Database,
        pub personal_data: autoxide_kernel::privacy::PersonalDataExport,
    }

    pub async fn services(test: &TestDatabase) -> Services {
        let database = database(test).await;
        let outbox = autoxide_kernel::event::Outbox::standalone("auth");
        let users = Arc::new(crate::user::UserService::new(
            database.clone(),
            crate::password::Passwords::new(),
            outbox.clone(),
        ));
        let tokens = crate::token::AccessTokens::new(
            TOKEN_SECRET.as_bytes().to_vec(),
            std::time::Duration::from_secs(900),
        );
        let mfa = Arc::new(crate::mfa::MfaService::new(
            database.clone(),
            "Autoxide".to_owned(),
            crate::session::LoginThrottle::new(
                database.clone(),
                5,
                std::time::Duration::from_secs(15 * 60),
            )
            .unwrap(),
            outbox.clone(),
        ));
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
                outbox.clone(),
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
        let roles =
            Arc::new(crate::role::RoleService::new(database.clone(), catalog, outbox.clone()));
        let api_keys = Arc::new(crate::api_key::ApiKeyService::new(
            database.clone(),
            users.clone(),
            roles.clone(),
            sessions.clone(),
            outbox,
        ));
        let mails = Arc::new(FakeMailer::default());
        let translations = autoxide_kernel::i18n::Translations::new(vec![(
            "auth",
            vec![("vi", include_str!("../locales/vi.json"))],
        )])
        .unwrap();

        let verifications = Arc::new(crate::verify::EmailVerifications {
            database: database.clone(),
            users: users.clone(),
            sessions: sessions.clone(),
            mailer: mails.clone(),
            translations: translations.clone(),
            outbox: autoxide_kernel::event::Outbox::standalone("auth"),
            url: None,
        });
        let passkey_config = crate::config::PasskeyConfig {
            rp_id: PASSKEY_RP_ID.to_owned(),
            origins: vec![PASSKEY_ORIGIN.to_owned()],
        };
        let passkeys = Arc::new(
            crate::passkey::PasskeyService::new(
                database.clone(),
                users.clone(),
                sessions.clone(),
                autoxide_kernel::event::Outbox::standalone("auth"),
                Some(&passkey_config),
            )
            .unwrap(),
        );
        let resets = Arc::new(crate::reset::PasswordResets {
            database: database.clone(),
            jobs: autoxide_kernel::job::JobQueue::new("auth"),
            users: users.clone(),
            mailer: mails.clone(),
            translations: translations.clone(),
            outbox: autoxide_kernel::event::Outbox::standalone("auth"),
            url: None,
        });
        let personal_data = autoxide_kernel::privacy::PersonalDataExport::default();
        personal_data
            .register(
                "auth",
                Arc::new(crate::privacy::AuthPersonalData {
                    users: users.clone(),
                    roles: roles.clone(),
                    sessions: sessions.clone(),
                    api_keys: api_keys.clone(),
                    passkeys: passkeys.clone(),
                    mfa: mfa.clone(),
                }),
            )
            .unwrap();
        Services {
            personal_data,
            passkeys,
            users,
            sessions,
            roles,
            api_keys,
            mfa,
            resets,
            verifications,
            mails,
            database,
        }
    }

    /// Token (chuỗi base64url 43 ký tự) trong mail cuối cùng đã gửi.
    /// Xin đặt lại mật khẩu cho `email` rồi chạy job gửi mail ngay, như worker (D-162).
    pub async fn request_reset(
        services: &Services,
        email: &str,
        language: autoxide_kernel::i18n::Language,
    ) {
        services.resets.request(email, language).await.unwrap();
        send_resets(services).await;
    }

    /// Chạy mọi job gửi mail đặt lại mật khẩu đang chờ, theo thứ tự xếp hàng.
    pub async fn send_resets(services: &Services) {
        use sea_orm::ConnectionTrait as _;
        use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Order, Query};
        let select = Query::select()
            .columns([Alias::new("id"), Alias::new("payload")])
            .from(Alias::new("auth_jobs"))
            .and_where(Expr::col(Alias::new("kind")).eq(crate::reset::SEND_RESET))
            .and_where(Expr::col(Alias::new("completed_at")).is_null())
            .order_by(Alias::new("id"), Order::Asc)
            .to_owned();
        for row in services.database.write().query_all(&select).await.unwrap() {
            let id: uuid::Uuid = row.try_get("", "id").unwrap();
            let payload: String = row.try_get("", "payload").unwrap();
            let job: crate::reset::ResetJob = serde_json::from_str(&payload).unwrap();
            services.resets.send(&job).await.unwrap();
            let done = Query::update()
                .table(Alias::new("auth_jobs"))
                .value(Alias::new("completed_at"), chrono::Utc::now())
                .and_where(Expr::col(Alias::new("id")).eq(id))
                .to_owned();
            services.database.write().execute(&done).await.unwrap();
        }
    }

    pub fn last_token(services: &Services) -> String {
        let sent = services.mails.sent.lock().unwrap();
        let text = &sent.last().unwrap().text;
        text.split_whitespace()
            .find(|word| {
                word.len() == 43
                    && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
            .unwrap()
            .to_owned()
    }

    /// `Mailer` giả: giữ lại mọi mail để test đọc.
    #[derive(Default)]
    pub struct FakeMailer {
        pub sent: std::sync::Mutex<Vec<autoxide_kernel::mail::Mail>>,
    }

    #[async_trait::async_trait]
    impl autoxide_kernel::mail::Mailer for FakeMailer {
        async fn send(
            &self,
            mail: autoxide_kernel::mail::Mail,
        ) -> Result<(), autoxide_kernel::error::Error> {
            self.sent.lock().unwrap().push(mail);
            Ok(())
        }
    }

    /// Service phiên đăng nhập kèm service người dùng dùng chung database.
    pub async fn sessions(
        test: &TestDatabase,
    ) -> (Arc<crate::session::SessionService>, Arc<crate::user::UserService>) {
        let services = services(test).await;
        (services.sessions, services.users)
    }

    /// Các event đã ghi vào `auth_events`, theo thứ tự: (tên, người dùng gây ra,
    /// subject, payload).
    pub async fn recorded_events(
        database: &Database,
    ) -> Vec<(String, Option<uuid::Uuid>, Option<String>, String)> {
        use sea_orm::ConnectionTrait;
        let backend = database.write().get_database_backend();
        let sql = "SELECT name, actor_user_id, subject, payload FROM auth_events ORDER BY id";
        let rows = database
            .write()
            .query_all_raw(sea_orm::Statement::from_string(backend, sql))
            .await
            .unwrap();
        rows.iter()
            .map(|row| {
                (
                    row.try_get("", "name").unwrap(),
                    row.try_get("", "actor_user_id").unwrap(),
                    row.try_get("", "subject").unwrap(),
                    row.try_get("", "payload").unwrap(),
                )
            })
            .collect()
    }
}
