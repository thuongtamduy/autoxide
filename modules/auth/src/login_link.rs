//! Đăng nhập bằng link qua email (bước 34 mục 11, D-311): không cần mật khẩu.
//!
//! - `request`: chỉ sinh `device_secret` ngẫu nhiên (khi xin gắn thiết bị) và xếp job — KHÔNG tra
//!   tài khoản trong request: trả lời như nhau, cùng thời gian, dù email có hay không (D-154, D-162).
//! - Job `auth.send_login_link`: tài khoản đang hoạt động thì phát token dùng một lần
//!   (`AUTOXIDE_AUTH_LOGIN_LINK_MINUTES`), lưu bản băm của `device_secret` trong `data`, gửi mail.
//! - `confirm`: dùng token (một lần); có gắn thiết bị thì `device_secret` phải khớp; tài khoản bật
//!   MFA thì vẫn hỏi mã như `/login`.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::access::Realm;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::i18n::{Language, Translations};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use autoxide_kernel::mail::{MailTemplate, Mailer};
use autoxide_kernel::secret::{random_token, sha256};
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext, link_token};
use autoxide_kernel::tokens::{OneTimeTokens, TokenRejection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events;
use crate::session::{LoginOutcome, SessionService};
use crate::user::UserService;

const PURPOSE: &str = "login_link";
/// Mỗi tài khoản nhận nhiều nhất một link mỗi chừng này giây (chống dùng API để spam).
const COOLDOWN_SECONDS: i64 = 60;

fn tokens() -> OneTimeTokens {
    OneTimeTokens::new("auth")
}

const LINK_MAIL: MailTemplate = MailTemplate::new(
    "auth.mail.login_link",
    "Your sign-in link",
    "Open this link within {minutes} minutes to sign in as {email}:\n\n{link}\n\nIf you did not \
     ask for it, ignore this mail; nobody can sign in without it.",
);

const CODE_MAIL: MailTemplate = MailTemplate::new(
    "auth.mail.login_code",
    "Your sign-in token",
    "Use this token within {minutes} minutes to sign in as {email} \
     (`POST /api/v1/auth/login/link/confirm`):\n\n{token}\n\nIf you did not ask for it, ignore \
     this mail; nobody can sign in without it.",
);

/// Loại job gửi link đăng nhập.
pub(crate) const SEND_LOGIN_LINK: &str = "auth.send_login_link";

/// Job [`SEND_LOGIN_LINK`]. Mang email người hỏi — job xong thì job queue xoá payload (D-079).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LoginLinkJob {
    pub email: String,
    #[serde(default)]
    pub realm: Realm,
    pub language: String,
    /// Bản băm của `device_secret` khi người xin muốn gắn link vào thiết bị đang dùng.
    pub device: Option<String>,
}

impl Job for LoginLinkJob {
    const KIND: &'static str = SEND_LOGIN_LINK;
}

/// Bên chạy job [`SEND_LOGIN_LINK`].
pub(crate) struct LoginLinkMailer(pub Arc<LoginLinks>);

#[async_trait::async_trait]
impl JobHandler for LoginLinkMailer {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("a login-link mail job must belong to a tenant"))?;
        let job: LoginLinkJob =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.send(tenant, &job).await
    }
}

pub(crate) struct LoginLinks {
    /// `AUTOXIDE_AUTH_LOGIN_LINK`: tắt thì mọi endpoint trả `auth.login_link_disabled`.
    pub enabled: bool,
    pub jobs: JobQueue,
    pub users: Arc<UserService>,
    pub sessions: Arc<SessionService>,
    pub mailer: Arc<dyn Mailer>,
    pub translations: Translations,
    pub outbox: Outbox,
    /// Link theo tên miền của khách (`kernel.public_url`).
    pub links: autoxide_kernel::tenancy::SiteLinks,
    /// Trang của giao diện, có `{token}`; `None`: mail chứa token.
    pub url: Option<String>,
    pub minutes: i64,
    pub databases: DatabaseRegistry,
}

impl LoginLinks {
    fn require_enabled(&self) -> Result<(), Error> {
        if self.enabled {
            Ok(())
        } else {
            Err(Error::not_found(
                "auth.login_link_disabled",
                "Signing in with a link is not turned on on this server (AUTOXIDE_AUTH_LOGIN_LINK).",
            ))
        }
    }

    /// Xếp việc gửi link cho `email`. Trả `device_secret` khi `bind_device` (sinh mới mỗi lần, cả
    /// khi email không có tài khoản). Như nhau dù email có hay không.
    pub(crate) async fn request(
        &self,
        tenant: &TenantContext,
        email: &str,
        realm: Realm,
        bind_device: bool,
        language: Language,
    ) -> Result<Option<String>, Error> {
        self.require_enabled()?;
        if let Some(problem) = FieldError::blank("email", email) {
            return Err(Error::validation(vec![problem]));
        }
        let secret = if bind_device { Some(random_token(32)?) } else { None };
        let job = LoginLinkJob {
            email: email.trim().to_owned(),
            realm,
            language: language.tag().to_owned(),
            device: secret.as_deref().map(sha256),
        };
        let write = tenant.database().write();
        self.jobs.enqueue(write, tenant, &job, EnqueueOptions::default()).await?;
        Ok(secret)
    }

    /// Job: gửi link nếu email là của một tài khoản đang hoạt động.
    pub(crate) async fn send(
        &self,
        tenant: &TenantContext,
        job: &LoginLinkJob,
    ) -> Result<(), Error> {
        let fallback = if job.language == Language::Vi.tag() { Language::Vi } else { Language::En };
        let Some(user) = self.users.find_by_email_in(tenant, job.realm, &job.email).await? else {
            return Ok(());
        };
        if !user.is_active() {
            return Ok(());
        }
        let language = self.users.mail_language(tenant, user.id, fallback).await?;
        let subject = user.id.to_string();
        let since = now() - SignedDuration::from_secs(COOLDOWN_SECONDS);
        let recent =
            tokens().recent(tenant.database().write(), tenant, PURPOSE, &subject, since).await?;
        if !recent.is_empty() {
            return Ok(());
        }
        let ttl = SignedDuration::from_mins(self.minutes);
        let transaction = tenant.database().begin_write().await?;
        let raw = tokens()
            .issue(&transaction, tenant, PURPOSE, &subject, job.device.as_deref(), ttl)
            .await?
            .token;
        transaction.commit().await?;
        // Token trong mail mang khách: link mở trên tên miền chính vẫn tìm được khách.
        let token = link_token(tenant, &raw);
        let minutes = self.minutes.to_string();
        let page = self.links.link(tenant, &crate::links::LOGIN_LINK, self.url.as_deref()).await?;
        let mail = match &page {
            Some(url) => {
                let link = url.replace("{token}", &token);
                let params =
                    [("email", user.email.as_str()), ("minutes", &minutes), ("link", &link)];
                LINK_MAIL.mail(&user.email, &self.translations, language, &params)
            }
            None => {
                let params =
                    [("email", user.email.as_str()), ("minutes", &minutes), ("token", &token)];
                CODE_MAIL.mail(&user.email, &self.translations, language, &params)
            }
        };
        self.mailer.send(tenant, mail).await
    }

    /// Đăng nhập bằng `token` lấy từ mail. Khách lấy từ token (`request`: khách của request nếu
    /// biết). Trả khách và kết quả như `/login` (token, hay cần mã MFA).
    pub(crate) async fn confirm(
        &self,
        request: Option<&TenantContext>,
        token: &str,
        device_secret: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<(TenantContext, LoginOutcome), Error> {
        self.require_enabled()?;
        if let Some(problem) = FieldError::blank("token", token) {
            return Err(Error::validation(vec![problem]));
        }
        let Some((tenant, raw)) = crate::token::resolve(&self.databases, request, token).await?
        else {
            return Err(invalid().with_reason(TokenRejection::Unknown.as_str()));
        };
        let consumed = tokens().consume(tenant.database().write(), &tenant, PURPOSE, raw).await?;
        let Some(consumed) = consumed else {
            let why = tokens().rejection(tenant.database().read(), &tenant, PURPOSE, raw).await?;
            return Err(invalid().with_reason(why.as_str()));
        };
        // Link gắn thiết bị: chỉ thiết bị giữ `device_secret` dùng được (link bị chặn giữa đường
        // vô dụng). Token đã bị dùng: thử sai không được thử lại.
        if let Some(expected) = consumed.data.as_deref()
            && device_secret.map(sha256).as_deref() != Some(expected)
        {
            return Err(invalid().with_reason("device"));
        }
        let user_id: Uuid = consumed.subject.parse().map_err(|_| invalid())?;
        let transaction = tenant.database().begin_write().await?;
        self.claim_unverified(&transaction, &tenant, user_id).await?;
        let event = events::LoginLinkUsed::default();
        self.outbox
            .record(
                &transaction,
                &tenant,
                Actor::user(user_id),
                events::user_subject(user_id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        let outcome = self.sessions.login_with_link(&tenant, user_id, user_agent).await?;
        Ok((tenant, outcome))
    }
}

impl LoginLinks {
    /// Tài khoản chưa ai xác minh email mà link gửi tới email đó vừa được mở: người mở mới là chủ
    /// email. Đánh dấu email đã xác minh và gỡ mọi cửa đặt trước khi email được xác minh — mật khẩu,
    /// phiên, passkey, API key, TOTP, liên kết ngoài — vì có thể do người khác đăng ký trước bằng
    /// email này đặt (D-20261121-login-link-claims). Tài khoản đã xác minh: không đổi gì.
    async fn claim_unverified(
        &self,
        transaction: &sea_orm::DatabaseTransaction,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<(), Error> {
        use crate::user::entity as user;
        use autoxide_kernel::tenancy::ScopeExt as _;
        use sea_orm::sea_query::Expr;
        use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};
        let Some(account) = autoxide_kernel::rows::lock_one(
            transaction,
            user::Entity::find_by_id(user_id).scoped(tenant),
        )
        .await?
        else {
            return Ok(());
        };
        if account.email_verified_at.is_some() || account.email.is_empty() {
            return Ok(());
        }
        let now = now();
        user::Entity::update_many()
            .col_expr(user::Column::PasswordHash, Expr::value(None::<String>))
            .col_expr(user::Column::EmailVerifiedAt, Expr::value(Some(now)))
            .col_expr(user::Column::UpdatedAt, Expr::value(now))
            .filter(user::Column::Id.eq(user_id))
            .scoped(tenant)
            .exec(transaction)
            .await?;
        let actor = Actor::user(user_id);
        SessionService::revoke_user(transaction, tenant, user_id).await?;
        crate::passkey::remove_all_in(transaction, tenant, &self.outbox, actor, user_id, false)
            .await?;
        crate::api_key::revoke_all_in(transaction, tenant, &self.outbox, actor, user_id).await?;
        crate::mfa::service::remove_in(transaction, tenant, &self.outbox, actor, user_id, false)
            .await?;
        crate::oidc::identity::delete_for_user(transaction, tenant, user_id).await?;
        crate::user::revoke_durable_grants_in(transaction, tenant, &self.outbox, actor, user_id)
            .await?;
        tracing::info!(%user_id, "unverified account claimed by its email owner");
        Ok(())
    }
}

fn now() -> Timestamp {
    Timestamp::now()
}

fn invalid() -> Error {
    Error::unauthenticated(
        "auth.login_link_invalid",
        "This sign-in link is wrong, already used or expired; ask for a new one.",
    )
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::time::Timestamp;
    use std::sync::Arc;

    use autoxide_kernel::access::Realm;
    use autoxide_kernel::event::Actor;
    use autoxide_kernel::i18n::Language;
    use autoxide_testkit::TestDatabase;

    use super::{LoginLinkJob, LoginLinks, SEND_LOGIN_LINK};
    use crate::http::test_support::PASSWORD;
    use crate::session::LoginOutcome;
    use crate::testing::{Services, last_token, services, take_jobs};
    use crate::user::NewUser;

    fn links(services: &Services, enabled: bool) -> LoginLinks {
        LoginLinks {
            enabled,
            jobs: autoxide_kernel::job::JobQueue::new("auth"),
            users: services.users.clone(),
            sessions: services.sessions.clone(),
            mailer: services.mails.clone(),
            translations: autoxide_kernel::i18n::Translations::default(),
            outbox: autoxide_kernel::event::Outbox::standalone("auth"),
            links: Default::default(),
            url: None,
            minutes: 15,
            databases: autoxide_kernel::tenancy::DatabaseRegistry::single(
                services.database.clone(),
            ),
        }
    }

    async fn ada(services: &Services) -> uuid::Uuid {
        let input = NewUser {
            email: "ada@example.com".to_owned(),
            name: "Ada".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        services.users.create(&services.tenant, Actor::system(), input).await.unwrap().id
    }

    /// Chạy các job gửi link đang chờ như worker.
    async fn send(services: &Services, links: &LoginLinks) {
        for payload in take_jobs(services, SEND_LOGIN_LINK).await {
            let job: LoginLinkJob = serde_json::from_str(&payload).unwrap();
            links.send(&services.tenant, &job).await.unwrap();
        }
    }

    fn sent(services: &Services) -> usize {
        services.mails.sent().len()
    }

    #[tokio::test]
    async fn link_dang_nhap_tra_loi_nhu_nhau_du_email_khong_co() {
        TestDatabase::run(async |test| {
            let services = services(test).await;
            ada(&services).await;
            let off = links(&services, false);
            let refused = off
                .request(&services.tenant, "ada@example.com", Realm::Staff, false, Language::En)
                .await;
            assert_eq!(refused.unwrap_err().code(), "auth.login_link_disabled");

            let links = links(&services, true);
            let tenant = &services.tenant;
            for email in ["ada@example.com", "nobody@example.com"] {
                let plain =
                    links.request(tenant, email, Realm::Staff, false, Language::En).await.unwrap();
                assert_eq!(plain, None);
                let bound =
                    links.request(tenant, email, Realm::Staff, true, Language::En).await.unwrap();
                assert_eq!(bound.map(|secret| secret.len()), Some(43), "{email}");
            }
            // Mỗi request một job, như nhau; mail chỉ tới tài khoản có thật (một lần, giãn cách).
            assert_eq!(take_jobs(&services, SEND_LOGIN_LINK).await.len(), 4);
            links
                .request(tenant, "ada@example.com", Realm::Staff, false, Language::En)
                .await
                .unwrap();
            links
                .request(tenant, "nobody@example.com", Realm::Staff, false, Language::En)
                .await
                .unwrap();
            send(&services, &links).await;
            assert_eq!(sent(&services), 1);
            let (_, outcome) =
                links.confirm(Some(tenant), &last_token(&services), None, None).await.unwrap();
            assert!(matches!(outcome, LoginOutcome::Tokens(_)));
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn link_dang_nhap_van_hoi_mfa() {
        TestDatabase::run(async |test| {
            let services = services(test).await;
            let user = ada(&services).await;
            // Email đã xác minh: link không gỡ TOTP (chỉ gỡ cửa của tài khoản chưa xác minh).
            let transaction = services.database.begin_write().await.unwrap();
            let (tenant, email) = (&services.tenant, "ada@example.com");
            let now = Timestamp::now();
            services
                .users
                .apply_verified_email(tenant, &transaction, user, email, now)
                .await
                .unwrap();
            transaction.commit().await.unwrap();
            let setup =
                services.mfa.start_totp(&services.tenant, user, "ada@example.com").await.unwrap();
            let now = u64::try_from(Timestamp::now().as_unix_seconds()).unwrap();
            let code = crate::mfa::service::tests::code_at(&setup.secret, now);
            services.mfa.confirm_totp(&services.tenant, user, &code).await.unwrap();
            let links = links(&services, true);
            links
                .request(&services.tenant, "ada@example.com", Realm::Staff, false, Language::En)
                .await
                .unwrap();
            send(&services, &links).await;
            let (_, outcome) = links
                .confirm(Some(&services.tenant), &last_token(&services), None, None)
                .await
                .unwrap();
            assert!(matches!(outcome, LoginOutcome::MfaRequired { .. }), "link không bỏ qua MFA");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn link_gan_thiet_bi_sai_secret_bi_tu_choi() {
        TestDatabase::run(async |test| {
            let services = services(test).await;
            ada(&services).await;
            let links = links(&services, true);
            let tenant = &services.tenant;
            let secret = links
                .request(tenant, "ada@example.com", Realm::Staff, true, Language::En)
                .await
                .unwrap()
                .unwrap();
            send(&services, &links).await;
            let token = last_token(&services);
            let wrong = links.confirm(Some(tenant), &token, Some("not the secret"), None).await;
            assert_eq!(wrong.unwrap_err().code(), "auth.login_link_invalid");
            // Token đã bị dùng: secret đúng cũng không cứu được (không đoán thử mãi).
            let late = links.confirm(Some(tenant), &token, Some(&secret), None).await;
            assert_eq!(late.unwrap_err().code(), "auth.login_link_invalid");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn link_dang_nhap_dung_mot_lan_dong_thoi() {
        TestDatabase::run(async |test| {
            let services = services(test).await;
            ada(&services).await;
            let links = Arc::new(links(&services, true));
            links
                .request(&services.tenant, "ada@example.com", Realm::Staff, false, Language::En)
                .await
                .unwrap();
            send(&services, &links).await;
            let token = last_token(&services);
            let mut tasks = Vec::new();
            for _ in 0..8 {
                let (links, tenant, token) =
                    (links.clone(), services.tenant.clone(), token.clone());
                tasks.push(tokio::spawn(async move {
                    links.confirm(Some(&tenant), &token, None, None).await.is_ok()
                }));
            }
            let mut won = 0;
            for task in tasks {
                won += usize::from(task.await.unwrap());
            }
            assert_eq!(won, 1);
        })
        .await
        .unwrap();
    }

    /// Tài khoản chưa xác minh email (có thể do người khác đăng ký trước): mở link gửi tới email đó
    /// thì email thành đã xác minh và mật khẩu, phiên đặt trước đó bị gỡ.
    #[tokio::test]
    async fn link_tren_tai_khoan_chua_xac_minh_go_mat_khau_cu() {
        TestDatabase::run(async |test| {
            let services = services(test).await;
            let tenant = &services.tenant;
            let id = ada(&services).await;
            let early = services.sessions.login(tenant, "ada@example.com", PASSWORD, None).await;
            let LoginOutcome::Tokens(early) = early.unwrap() else { panic!("không MFA") };
            let links = links(&services, true);
            links
                .request(tenant, "ada@example.com", Realm::Staff, false, Language::En)
                .await
                .unwrap();
            send(&services, &links).await;
            let token = last_token(&services);
            links.confirm(Some(tenant), &token, None, None).await.unwrap();
            let user = services.users.get(tenant, id).await.unwrap().unwrap();
            assert!(user.email_verified_at.is_some(), "email đã xác minh");
            let again = services.sessions.login(tenant, "ada@example.com", PASSWORD, None).await;
            assert!(again.is_err(), "mật khẩu đặt trước khi xác minh bị gỡ");
            let refreshed = services.sessions.refresh(Some(tenant), &early.refresh_token).await;
            assert!(refreshed.is_err(), "phiên cũ bị thu hồi");
        })
        .await
        .unwrap();
    }
}
