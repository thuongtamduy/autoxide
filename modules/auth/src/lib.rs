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
#[cfg(any(test, feature = "bench-seed"))]
mod bench_seed;
mod cleanup;
mod client_config;
mod config;
#[cfg(test)]
mod credential_key_tests;
mod delegation;
#[cfg(test)]
mod delegation_tests;
mod doctor;
mod email_report;
mod events;
mod find;
mod guardrail;
#[cfg(test)]
mod guardrail_tests;
#[cfg(test)]
mod hardening_tests;
mod http;
mod invitation;
mod links;
mod login_link;
#[cfg(test)]
mod login_policy_tests;
mod mfa;
mod migration;
mod oidc;
#[cfg(test)]
mod org_tests;
mod passkey;
mod password;
#[cfg(test)]
mod password_change_tests;
mod permissions;
mod phone;
#[cfg(test)]
mod phone_tests;
mod policy;
mod privacy;
mod reauth;
mod recovery;
#[cfg(test)]
pub(crate) mod registration_tests;
mod relocation;
mod reset;
mod resource;
mod role;
mod sandbox;
mod scim;
mod session;
mod settings;
mod step_up;
#[cfg(test)]
mod step_up_tests;
mod support;
#[cfg(test)]
mod support_tests;
#[cfg(test)]
mod tenancy_tests;
mod token;
mod tools;
mod user;
mod verify;
#[cfg(test)]
mod web_tests;

use jiff::SignedDuration;
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
    login_links: Option<Arc<login_link::LoginLinks>>,
    invitations: Option<Arc<invitation::Invitations>>,
    verifications: Option<Arc<verify::EmailVerifications>>,
    administrator: Option<Arc<doctor::Administrator>>,
}

impl Module for AuthModule {
    fn navigation(&self) -> autoxide_kernel::module::navigation::Navigation {
        use autoxide_kernel::module::navigation::{NavGroup, NavItem, Navigation};
        Navigation::default()
            .group(NavGroup::new("organization", "Organization", "sitemap", 90))
            .group(NavGroup::new("customers", "Customers", "users", 60))
            .group(NavGroup::new("settings", "Settings", "gear", 110))
            .item(
                NavItem::new("organization.access", "organization", "People and roles", 20)
                    .permissions(&[crate::permissions::USERS_READ, crate::permissions::ROLES_READ]),
            )
            .item(
                NavItem::new("customers.accounts", "customers", "Buyer accounts", 10)
                    .permissions(&[crate::permissions::CUSTOMERS_READ]),
            )
            .item(
                NavItem::new("settings.api_keys", "settings", "API keys", 20)
                    .mobile(autoxide_kernel::module::navigation::MobilePlacement::Hidden, 20),
            )
    }

    fn name(&self) -> &'static str {
        "auth"
    }

    // Core không chạy được nếu thiếu module này: `AUTOXIDE_MODULES` không tắt được nó (bước 70 P1).
    fn essential(&self) -> bool {
        true
    }

    // Module buộc phải có (`context.require::<…>()?` trong `start`); bật module này thì chúng được
    // bật kèm (bước 70 P1). Thứ dùng "nếu có" (`.ok()`) không khai ở đây.
    fn depends_on(&self) -> Vec<&'static str> {
        vec!["mail"]
    }

    fn description(&self) -> &'static str {
        "User accounts, sign-in with passwords, passkeys and TOTP, access and refresh tokens, \
         API keys, roles and permissions."
    }

    // Module nền: không gói nào tắt được (bước 35, D-323).
    fn features(&self) -> Vec<autoxide_kernel::entitlement::Feature> {
        vec![api_key::API_KEYS_FEATURE, scim::SCIM_FEATURE]
    }

    fn limits(&self) -> Vec<autoxide_kernel::entitlement::Limit> {
        vec![crate::role::MEMBERS_LIMIT]
    }

    fn always_enabled(&self) -> bool {
        true
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn upgrades(&self) -> Vec<Box<dyn autoxide_kernel::migration::DataUpgrade>> {
        migration::upgrades()
    }

    /// Khách mới có role `admin` (mọi permission) như khách mặc định (bước 33).
    fn org_settings(&self) -> Vec<autoxide_kernel::tenancy::OrgSetting> {
        settings::all()
    }

    fn seed_tenant(&self) -> Option<Arc<dyn autoxide_kernel::tenancy::TenantSeed>> {
        Some(Arc::new(role::AdminRoleSeed))
    }

    fn client_config(&self) -> Result<Option<autoxide_kernel::module::ClientSection>, Error> {
        self.config.as_ref().map_or(Ok(None), client_config::section)
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(AuthConfig::read(reader)?);
        Ok(())
    }

    // Thời hạn lưu dữ liệu cá nhân (bước 37 C5, D-360), đọc từ cấu hình đang có.
    fn retention(&self) -> Vec<autoxide_kernel::privacy::RetentionPolicy> {
        use autoxide_kernel::privacy::{DataCategory, Keep, RetentionPolicy};
        let Some(config) = &self.config else {
            return Vec::new();
        };
        let days = u32::try_from(config.refresh_token_ttl.as_secs() / 86_400).unwrap_or(u32::MAX);
        vec![
            RetentionPolicy::new(
                "auth.accounts",
                "Accounts (email, name, password hash); kept until the account is deleted.",
                DataCategory::Basic,
                Keep::UntilDeleted,
            ),
            RetentionPolicy::new(
                "auth.sessions",
                "Sign-in sessions and refresh tokens (hashed).",
                DataCategory::Security,
                Keep::Days(days.max(1)),
            )
            .setting(config::REFRESH_TOKEN_DAYS.name),
        ]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.finder_source(Arc::new(find::Users))?;
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `auth` started before it was configured"))?;
        // Service không giữ database: mỗi hàm nhận khách của request (bước 33).
        let databases = context.tenancy();
        let outbox = context.outbox()?;
        self.cleanup = Some(Arc::new(cleanup::Cleanup));
        self.administrator = Some(Arc::new(doctor::Administrator { databases: databases.clone() }));
        // Chính sách đăng nhập theo khách (D-20261126-tenant-login-policy): đọc cài đặt ở nút gốc qua `organizations`.
        let policy = policy::TenantPolicy::new(
            context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
        );
        let users = Arc::new(UserService::new(
            Passwords::new().with_policy(policy.clone()),
            outbox.clone(),
        ));
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
        let access_tokens = AccessTokens::new(secret, config.access_token_ttl)
            .with_previous(config.token_secret_previous.clone());
        let throttle = LoginThrottle::new(config.login_max_failures, config.login_lock)?;
        let mailer = context.require::<dyn Mailer>()?;
        let secrets = context.require::<autoxide_kernel::secret::SecretKey>()?;

        let mfa = Arc::new(
            MfaService::new(
                secrets.as_ref().clone(),
                config.totp_issuer.clone(),
                LoginThrottle::new(config.login_max_failures, config.login_lock)?,
                outbox.clone(),
            )
            .with_policy(policy.clone()),
        );
        let sessions = Arc::new(
            SessionService::new(
                users.clone(),
                access_tokens,
                config.refresh_token_ttl,
                throttle,
                mfa.clone(),
                outbox.clone(),
                databases.clone(),
            )?
            .with_policy(policy),
        );
        let passkeys = Arc::new(passkey::PasskeyService::new(
            users.clone(),
            sessions.clone(),
            outbox.clone(),
            config.passkey.as_ref(),
        )?);
        // Link trong thư theo khách: `kernel.public_url` và trang riêng `auth.link.*` (kernel kiểm host).
        let links = context
            .require::<autoxide_kernel::tenancy::SiteLinks>()
            .map(|links| (*links).clone())
            .unwrap_or_default();
        let verifications = Arc::new(verify::EmailVerifications {
            users: users.clone(),
            sessions: sessions.clone(),
            mailer: mailer.clone(),
            translations: context.translations().clone(),
            outbox: outbox.clone(),
            jobs: context.jobs(),
            links: links.clone(),
            url: config.email_verification_url.clone(),
            report_url: config.email_change_report_url.clone(),
            databases: databases.clone(),
        });
        let roles = Arc::new(
            RoleService::new(context.permissions().clone(), outbox.clone())
                .with_translations(context.translations().clone())
                .with_entitlements(context.entitlements()),
        );
        context.usage_counter(&crate::role::MEMBERS_LIMIT, Arc::new(crate::role::MemberCounter))?;
        let mut api_keys = ApiKeyService::new(
            roles.clone(),
            sessions.clone(),
            outbox.clone(),
            mfa.clone(),
            passkeys.clone(),
        )
        .with_max_days(config.api_key_max_days);
        // Cấu hình theo khách do kernel công bố; thiếu (chương trình rút gọn) thì không duyệt key.
        if let Ok(settings) = context.require::<dyn autoxide_kernel::tenancy::TenantSettings>() {
            api_keys = api_keys.with_settings(settings);
        }
        let api_keys = Arc::new(api_keys);
        let resets = Arc::new(reset::PasswordResets {
            jobs: context.jobs(),
            users: users.clone(),
            mailer: mailer.clone(),
            translations: context.translations().clone(),
            outbox: outbox.clone(),
            links: links.clone(),
            url: config.password_reset_url.clone(),
            databases: databases.clone(),
        });
        let delegations = Arc::new(delegation::DelegationService {
            databases: databases.clone(),
            outbox: outbox.clone(),
            catalog: context.permissions().clone(),
        });
        let support = Arc::new(support::SupportService {
            databases: databases.clone(),
            outbox: outbox.clone(),
            orgs: context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            max_hours: config.support_max_hours,
            catalog: context.permissions().clone(),
        });
        let guardrails = Arc::new(guardrail::GuardrailService::new(
            databases.clone(),
            context.permissions().clone(),
            outbox.clone(),
        ));
        let authenticator = RequestAuthenticator {
            guardrails: guardrails.clone(),
            support: support.clone(),
            sessions: sessions.clone(),
            roles: roles.clone(),
            api_keys: api_keys.clone(),
            databases: databases.clone(),
            delegations: delegations.clone(),
            step_up: step_up::StepUpPolicy {
                window: SignedDuration::from_mins(config.step_up_minutes),
                settings: context.require::<dyn autoxide_kernel::tenancy::TenantSettings>().ok(),
            },
        };
        // `Arc<X>` tự đổi thành `Arc<dyn Trait>`: bên dùng chỉ thấy trait.
        context.provide::<dyn Authenticator>(Arc::new(authenticator))?;
        context.provide::<dyn api::Users>(users.clone())?;
        // Mail của sandbox chỉ tới nhân viên khách gốc vào được sandbox (D-232).
        context.provide::<dyn autoxide_kernel::tenancy::sandbox::SandboxRecipients>(Arc::new(
            sandbox::StaffRecipients { databases: databases.clone() },
        ))?;
        context.provide::<dyn api::TenantAdmins>(users.clone())?;
        // Tên hiển thị của người dùng cho module khác (vd tên người làm trong audit log).
        context.resource_type(resource::USER, Arc::new(resource::UserResolver))?;
        context.provide::<dyn api::Accounts>(users.clone())?;
        context.provide::<dyn api::AccountRecovery>(Arc::new(recovery::OperatorRecovery {
            users: users.clone(),
            mfa: mfa.clone(),
            passkeys: passkeys.clone(),
        }))?;
        context.personal_data(Arc::new(privacy::AuthPersonalData {
            users: users.clone(),
            roles: roles.clone(),
            sessions: sessions.clone(),
            api_keys: api_keys.clone(),
            passkeys: passkeys.clone(),
            mfa: mfa.clone(),
        }))?;
        // Đăng nhập bằng số: `compliance` (đăng ký sau) cắm bộ mã qua `AuthExtensions`.
        let phone = Arc::new(phone::PhoneSignIn::new(
            sessions.clone(),
            secrets.as_ref().clone(),
            outbox.clone(),
            context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            // Tạo tài khoản từ token xem đơn: `orders` đăng ký sau nên nối muộn.
            context.later::<dyn autoxide_kernel::sms::VerifiedPhoneTokens>(),
        ));
        context.provide::<dyn api::AuthExtensions>(phone.clone())?;
        let reauth = Arc::new(reauth::Reauthentication {
            sessions: sessions.clone(),
            mfa: mfa.clone(),
            passkeys: passkeys.clone(),
            phone: phone.clone(),
            mailer: mailer.clone(),
            translations: context.translations().clone(),
            ttl: SignedDuration::try_from(config.reauth_ttl)
                .unwrap_or_else(|_| SignedDuration::from_mins(10)),
        });
        let login_links = Arc::new(login_link::LoginLinks {
            enabled: config.login_link,
            jobs: context.jobs(),
            users: users.clone(),
            sessions: sessions.clone(),
            mailer: mailer.clone(),
            translations: context.translations().clone(),
            outbox: outbox.clone(),
            links: links.clone(),
            url: config.login_link_url.clone(),
            minutes: config.login_link_minutes,
            databases: databases.clone(),
        });
        self.login_links = Some(login_links.clone());
        let oidc = Arc::new(oidc::OidcService {
            enabled: config.oidc,
            providers: oidc::ProviderService {
                secrets: secrets.as_ref().clone(),
                outbox: outbox.clone(),
            },
            client: oidc::OidcClient::new(config.oidc_allow_private_networks)?,
            users: users.clone(),
            roles: roles.clone(),
            sessions: sessions.clone(),
            outbox: outbox.clone(),
            secrets: secrets.as_ref().clone(),
            databases: databases.clone(),
            orgs: context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            entitlements: context.entitlements(),
            self_registration: config.self_registration,
        });
        let invitations = Arc::new(invitation::Invitations {
            users: users.clone(),
            roles: roles.clone(),
            sessions: sessions.clone(),
            jobs: context.jobs(),
            mailer: mailer.clone(),
            translations: context.translations().clone(),
            outbox: outbox.clone(),
            databases: databases.clone(),
            links: links.clone(),
            url: config.invitation_url.clone(),
            days: config.invitation_days,
            entitlements: context.entitlements(),
        });
        context.provide::<dyn api::Invitations>(invitations.clone())?;
        self.invitations = Some(invitations.clone());
        // Đồng bộ nhân viên qua SCIM (bước 66): khách lấy từ token SCIM qua sổ database.
        let scim = Arc::new(scim::ScimService {
            databases: databases.clone(),
            outbox: outbox.clone(),
            entitlements: context.entitlements(),
            users: users.clone(),
            roles: roles.clone(),
        });
        // Nhóm SCIM chưa có luật ánh xạ vào hộp việc (bước 51 P2).
        context.work_source(Arc::new(scim::UnmappedGroups))?;
        self.state = Some(http::HttpState {
            scim,
            phone,
            oidc,
            support,
            invitations,
            login_links,
            delegations,
            guardrails,
            orgs: context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            reauth,
            personal_data: context.personal_data_export(),
            sessions,
            users,
            roles,
            api_keys,
            mfa,
            resets: resets.clone(),
            verifications: verifications.clone(),
            passkeys,
            self_registration: config.self_registration,
            country_header: config.country_header.clone(),
        });
        self.resets = Some(resets);
        self.verifications = Some(verifications);
        Ok(())
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn deprecations(&self) -> Vec<autoxide_kernel::http::Deprecation> {
        // `/register` báo 409 khi email đã có tài khoản; `/registrations` trả lời như nhau
        // (D-20261121-registration-uniform). Hành vi cũ giữ tới ngày sunset.
        vec![
            autoxide_kernel::http::Deprecation::new(
                axum::http::Method::POST,
                "/register",
                "2026-11-21",
            )
            .sunset("2027-05-21")
            .successor("/api/v1/auth/registrations"),
        ]
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn tenant_settings(&self) -> Vec<autoxide_kernel::tenancy::TenantSetting> {
        let mut settings = vec![api_key::REQUIRE_APPROVAL, step_up::STEP_UP];
        settings.extend(links::ALL);
        settings
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
        if let Some(invitations) = &self.invitations {
            let mailer = Arc::new(invitation::InvitationMailer(invitations.clone()));
            jobs.push(JobRegistration::new(invitation::SEND_INVITATION, mailer));
        }
        if let Some(links) = &self.login_links {
            let mailer = Arc::new(login_link::LoginLinkMailer(links.clone()));
            jobs.push(JobRegistration::new(login_link::SEND_LOGIN_LINK, mailer));
        }
        if let Some(verifications) = &self.verifications {
            let mailer = Arc::new(email_report::EmailChangedMailer(verifications.clone()));
            jobs.push(JobRegistration::new(email_report::SEND_EMAIL_CHANGED, mailer));
        }
        jobs
    }

    fn schedules(&self) -> Vec<Schedule> {
        vec![cleanup::schedule()]
    }

    fn checks(&self) -> Vec<autoxide_kernel::doctor::Check> {
        let Some(administrator) = self.administrator.clone() else {
            return Vec::new();
        };
        // `administrator` chỉ có sau `start`, mà `start` cần `config`: có cái này là có cái kia.
        let configured = self.config.as_ref().is_some_and(|config| config.token_secret.is_some());
        vec![
            autoxide_kernel::doctor::Check::new(
                "administrator",
                "An active account with the `admin` role exists, so someone can manage the server.",
                administrator,
            ),
            autoxide_kernel::doctor::Check::new(
                "token_secret",
                "AUTOXIDE_AUTH_TOKEN_SECRET is set, so access tokens survive a restart and work \
                 on every server.",
                Arc::new(doctor::TokenSecret { configured }),
            ),
        ]
    }

    // Lệnh sinh dữ liệu đo (bước 39 B1): chỉ có khi bật cargo feature `bench-seed`.
    #[cfg(feature = "bench-seed")]
    fn tasks(&self) -> Vec<autoxide_kernel::task::Task> {
        self.administrator
            .as_ref()
            .map(|administrator| vec![bench_seed::task(administrator.databases.clone())])
            .unwrap_or_default()
    }

    fn tools(&self) -> Vec<autoxide_kernel::mcp::Tool> {
        self.state.clone().map_or_else(Vec::new, |state| tools::all(&Arc::new(state)))
    }

    fn routes(&self) -> OpenApiRouter {
        // Clone trạng thái chỉ là clone vài `Arc`: tăng bộ đếm tham chiếu, không chép
        // service.
        self.state.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    // Tách / gộp khách (bước 38): chính sách ở `relocation.rs`.
    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        let relocation = relocation::relocation();
        // Rào quyền chép sang khách mới ngay trong bước mở nó (gọi sau `start`).
        match &self.state {
            Some(state) => {
                relocation.on_switch(Arc::new(guardrail::GuardrailSwitch(state.guardrails.clone())))
            }
            None => relocation,
        }
    }

    // Uỷ quyền vào sandbox (bước 38, D-232): nghe event sandbox của kernel.
    fn subscriptions(&self) -> Vec<autoxide_kernel::event::Subscription> {
        let mut subscriptions = sandbox::subscriptions();
        let guardrails = self.state.as_ref().map(|state| state.guardrails.clone());
        subscriptions.extend(guardrail::subscriptions(guardrails));
        subscriptions
    }

    // Sao role vào sandbox mới (bước 38, D-232).
    fn sandbox_copy(&self) -> Option<Arc<dyn autoxide_kernel::tenancy::sandbox::SandboxCopy>> {
        let roles = self.state.as_ref()?.roles.clone();
        Some(Arc::new(sandbox::RoleCopy { roles }))
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

    use autoxide_kernel::time::Timestamp;
    /// Bên gửi mail giả dùng chung (testkit).
    pub use autoxide_testkit::FakeMailer;
    use jiff::SignedDuration;

    use std::sync::Arc;

    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::database::Database;
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_kernel::tenancy::TenantContext;
    use autoxide_testkit::TestDatabase;

    /// Kết nối tới database trống của test và áp dụng mọi migration của module.
    pub async fn database(test: &TestDatabase) -> Database {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        let database = Database::connect(&config.database).await.unwrap();
        // Bảng của kernel (`kernel_secrets`: khoá gốc mã hoá secret TOTP).
        autoxide_kernel::module::Modules::new(Vec::new())
            .unwrap()
            .migrator()
            .unwrap()
            .up(&database)
            .await
            .unwrap();
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
        pub reauth: Arc<crate::reauth::Reauthentication>,
        pub phone: Arc<crate::phone::PhoneSignIn>,
        pub mails: Arc<FakeMailer>,
        pub database: Database,
        /// Khách mặc định trên database của test.
        pub tenant: TenantContext,
        pub personal_data: autoxide_kernel::privacy::PersonalDataExport,
    }

    pub async fn services(test: &TestDatabase) -> Services {
        let database = database(test).await;
        // Một khách (khách mặc định) trên database của test: token mang khách tra qua sổ này.
        let registry = autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone());
        let outbox = autoxide_kernel::event::Outbox::standalone("auth");
        let users = Arc::new(crate::user::UserService::new(
            crate::password::Passwords::new(),
            outbox.clone(),
        ));
        let tokens = crate::token::AccessTokens::new(
            TOKEN_SECRET.as_bytes().to_vec(),
            std::time::Duration::from_secs(900),
        );
        let mfa = Arc::new(crate::mfa::MfaService::new(
            autoxide_kernel::secret::SecretKey::new(database.clone(), None),
            "Autoxide".to_owned(),
            crate::session::LoginThrottle::new(5, std::time::Duration::from_secs(15 * 60)).unwrap(),
            outbox.clone(),
        ));
        let sessions = Arc::new(
            crate::session::SessionService::new(
                users.clone(),
                tokens,
                std::time::Duration::from_secs(30 * 24 * 60 * 60),
                crate::session::LoginThrottle::new(5, std::time::Duration::from_secs(15 * 60))
                    .unwrap(),
                mfa.clone(),
                outbox.clone(),
                registry.clone(),
            )
            .unwrap(),
        );
        let catalog = autoxide_kernel::access::PermissionCatalog::new(vec![
            ("auth", crate::permissions::all()),
            (
                "notes",
                vec![
                    autoxide_kernel::access::Permission::new("notes.write", "Write notes."),
                    // Permission theo nút (bước 34), cho test độ sâu của role.
                    autoxide_kernel::access::Permission::new("notes.read", "Read notes.").scoped(),
                ],
            ),
        ])
        .unwrap();
        let roles = Arc::new(crate::role::RoleService::new(catalog, outbox.clone()));
        let mails = Arc::new(FakeMailer::default());
        let translations = autoxide_kernel::i18n::Translations::new(vec![(
            "auth",
            vec![("vi", include_str!("../locales/vi.json"))],
        )])
        .unwrap();

        let verifications = Arc::new(crate::verify::EmailVerifications {
            users: users.clone(),
            sessions: sessions.clone(),
            mailer: mails.clone(),
            translations: translations.clone(),
            outbox: autoxide_kernel::event::Outbox::standalone("auth"),
            jobs: autoxide_kernel::job::JobQueue::new("auth"),
            links: Default::default(),
            url: None,
            report_url: None,
            databases: registry.clone(),
        });
        let passkey_config = crate::config::PasskeyConfig {
            rp_id: PASSKEY_RP_ID.to_owned(),
            origins: vec![PASSKEY_ORIGIN.to_owned()],
        };
        let passkeys = Arc::new(
            crate::passkey::PasskeyService::new(
                users.clone(),
                sessions.clone(),
                autoxide_kernel::event::Outbox::standalone("auth"),
                Some(&passkey_config),
            )
            .unwrap(),
        );
        let api_keys = Arc::new(crate::api_key::ApiKeyService::new(
            roles.clone(),
            sessions.clone(),
            outbox,
            mfa.clone(),
            passkeys.clone(),
        ));
        let resets = Arc::new(crate::reset::PasswordResets {
            jobs: autoxide_kernel::job::JobQueue::new("auth"),
            users: users.clone(),
            mailer: mails.clone(),
            translations: translations.clone(),
            outbox: autoxide_kernel::event::Outbox::standalone("auth"),
            links: Default::default(),
            url: None,
            databases: registry.clone(),
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
        let phone = Arc::new(crate::phone::PhoneSignIn::new(
            sessions.clone(),
            autoxide_kernel::secret::SecretKey::new(database.clone(), None),
            autoxide_kernel::event::Outbox::standalone("auth"),
            None,
            autoxide_kernel::module::Later::ready(None),
        ));
        let reauth = Arc::new(crate::reauth::Reauthentication {
            sessions: sessions.clone(),
            mfa: mfa.clone(),
            passkeys: passkeys.clone(),
            phone: phone.clone(),
            mailer: mails.clone(),
            translations: translations.clone(),
            ttl: SignedDuration::from_mins(10),
        });
        Services {
            reauth,
            phone,
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
            tenant: TenantContext::default_on(database.clone()),
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
        services
            .resets
            .request(&services.tenant, email, Default::default(), language)
            .await
            .unwrap();
        send_resets(services).await;
    }

    /// Payload của các job loại `kind` đang chờ, theo thứ tự xếp hàng; đánh dấu chúng là xong.
    pub async fn take_jobs(services: &Services, kind: &str) -> Vec<String> {
        use sea_orm::ConnectionTrait as _;
        use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Order, Query};
        let select = Query::select()
            .columns([Alias::new("id"), Alias::new("payload")])
            .from(Alias::new("auth_jobs"))
            .and_where(Expr::col(Alias::new("kind")).eq(kind))
            .and_where(Expr::col(Alias::new("completed_at")).is_null())
            .order_by(Alias::new("id"), Order::Asc)
            .to_owned();
        let mut payloads = Vec::new();
        for row in services.database.write().query_all(&select).await.unwrap() {
            let id: uuid::Uuid = row.try_get("", "id").unwrap();
            payloads.push(row.try_get("", "payload").unwrap());
            let done = Query::update()
                .table(Alias::new("auth_jobs"))
                .value(Alias::new("completed_at"), Timestamp::now())
                .and_where(Expr::col(Alias::new("id")).eq(id))
                .to_owned();
            services.database.write().execute(&done).await.unwrap();
        }
        payloads
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
            services.resets.send(&services.tenant, &job).await.unwrap();
            let done = Query::update()
                .table(Alias::new("auth_jobs"))
                .value(Alias::new("completed_at"), Timestamp::now())
                .and_where(Expr::col(Alias::new("id")).eq(id))
                .to_owned();
            services.database.write().execute(&done).await.unwrap();
        }
    }

    pub fn last_token(services: &Services) -> String {
        let sent = services.mails.sent();
        let text = &sent.last().unwrap().text;
        text.split_whitespace()
            .find(|word| {
                word.len() == 43
                    && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
            .unwrap()
            .to_owned()
    }

    /// Service phiên đăng nhập kèm service người dùng dùng chung database.
    pub async fn sessions(
        test: &TestDatabase,
    ) -> (Arc<crate::session::SessionService>, Arc<crate::user::UserService>, TenantContext) {
        let services = services(test).await;
        (services.sessions, services.users, services.tenant)
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
