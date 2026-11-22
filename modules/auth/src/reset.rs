//! Đặt lại mật khẩu qua mail (D-063).
//!
//! 1. `request(email)`: xếp một job [`SEND_RESET`] — cùng một việc cho mọi email, nên cả câu trả
//!    lời lẫn thời gian trả lời không cho dò được email nào đã đăng ký (D-162). Job: có tài
//!    khoản đang hoạt động với email đó thì tạo token ngẫu nhiên (database giữ bản băm) và gửi
//!    mail; không có thì không làm gì.
//! 2. `confirm(token, mật khẩu mới)`: token đúng, chưa dùng, chưa hết hạn thì đặt mật
//!    khẩu mới; mọi phiên đăng nhập và mọi token đặt lại khác của người đó hết hiệu lực.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::i18n::{Language, Translations};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use autoxide_kernel::mail::{MailTemplate, Mailer};
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext, link_token};
use autoxide_kernel::tokens::{OneTimeTokens, TokenRejection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events;
use crate::password;
use crate::user::UserService;

/// Token sống chừng này.
const TOKEN_MINUTES: i64 = 30;
/// Mỗi email nhận nhiều nhất một mail mỗi chừng này giây (chống dùng API để spam).
const COOLDOWN_SECONDS: i64 = 60;
/// Mục đích của token trong bảng `auth_tokens` (kernel, D-117); `subject` là ID người dùng.
const PURPOSE: &str = "password_reset";

fn tokens() -> OneTimeTokens {
    OneTimeTokens::new("auth")
}

const RESET_LINK: MailTemplate = MailTemplate::new(
    "auth.mail.password_reset_link",
    "Reset your password",
    "Someone asked to reset the password of the account {email}.\n\nIf it was you, open \
     this link within {minutes} minutes and choose a new password:\n\n{link}\n\nIf it was \
     not you, ignore this mail: your password stays the same.",
);

const RESET_CODE: MailTemplate = MailTemplate::new(
    "auth.mail.password_reset_code",
    "Reset your password",
    "Someone asked to reset the password of the account {email}.\n\nIf it was you, use this \
     reset token within {minutes} minutes to choose a new password (`POST \
     /api/v1/auth/password-reset/confirm`):\n\n{token}\n\nIf it was not you, ignore this mail: \
     your password stays the same.",
);

/// Loại job gửi mail đặt lại mật khẩu.
pub(crate) const SEND_RESET: &str = "auth.send_password_reset";

/// Job [`SEND_RESET`]. Mang email người hỏi — job xong thì job queue xoá payload (D-079).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ResetJob {
    pub email: String,
    /// Tài khoản nhân sự hay người mua có email này (D-309); job cũ không có: nhân sự.
    #[serde(default)]
    pub realm: autoxide_kernel::access::Realm,
    /// Mã ngôn ngữ của mail (`en`, `vi`).
    pub language: String,
}

impl Job for ResetJob {
    const KIND: &'static str = SEND_RESET;
}

/// Bên chạy job [`SEND_RESET`].
pub(crate) struct ResetMailer(pub Arc<PasswordResets>);

#[async_trait::async_trait]
impl JobHandler for ResetMailer {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("a password-reset mail job must belong to a tenant"))?;
        let job: ResetJob = serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.send(tenant, &job).await
    }
}

pub(crate) struct PasswordResets {
    pub jobs: JobQueue,
    pub users: Arc<UserService>,
    pub mailer: Arc<dyn Mailer>,
    pub translations: Translations,
    pub outbox: Outbox,
    /// Link theo tên miền của khách (`kernel.public_url`).
    pub links: autoxide_kernel::tenancy::SiteLinks,
    /// Trang của giao diện, có `{token}`; `None`: mail chứa token.
    pub url: Option<String>,
    /// Sổ database: tìm khách ghi trong token của link.
    pub databases: DatabaseRegistry,
}

impl PasswordResets {
    /// Xếp việc gửi mail đặt lại mật khẩu cho `email`. Như nhau dù email có tồn tại hay không
    /// — kể cả thời gian: việc tra tài khoản, tạo token, gửi mail chạy trong job (D-162).
    pub(crate) async fn request(
        &self,
        tenant: &TenantContext,
        email: &str,
        realm: autoxide_kernel::access::Realm,
        language: Language,
    ) -> Result<(), Error> {
        let job =
            ResetJob { email: email.trim().to_owned(), realm, language: language.tag().to_owned() };
        let write = tenant.database().write();
        self.jobs.enqueue(write, tenant, &job, EnqueueOptions::default()).await?;
        Ok(())
    }

    /// Job: gửi mail đặt lại mật khẩu nếu email là của một tài khoản đang hoạt động.
    pub(crate) async fn send(&self, tenant: &TenantContext, job: &ResetJob) -> Result<(), Error> {
        let language = if job.language == Language::Vi.tag() { Language::Vi } else { Language::En };
        let Some(user) = self.users.find_by_email_in(tenant, job.realm, &job.email).await? else {
            return Ok(());
        };
        if !user.is_active() {
            return Ok(());
        }
        let language = self.users.mail_language(tenant, user.id, language).await?;
        let now = now();
        let subject = user.id.to_string();
        let since = now - SignedDuration::from_secs(COOLDOWN_SECONDS);
        if !tokens()
            .recent(tenant.database().write(), tenant, PURPOSE, &subject, since)
            .await?
            .is_empty()
        {
            return Ok(());
        }

        let transaction = tenant.database().begin_write().await?;
        let ttl = SignedDuration::from_mins(TOKEN_MINUTES);
        let raw = tokens().issue(&transaction, tenant, PURPOSE, &subject, None, ttl).await?.token;
        // Token trong mail mang khách: link mở trên tên miền chính vẫn tìm được khách.
        let token = link_token(tenant, &raw);
        let event = events::PasswordResetRequested::default();
        let subject = events::user_subject(user.id);
        self.outbox.record(&transaction, tenant, Actor::system(), subject, &event).await?;
        transaction.commit().await?;

        let minutes = TOKEN_MINUTES.to_string();
        let mail = match &self.url {
            Some(url) => {
                let link = self.links.rewrite(tenant, url).await?.replace("{token}", &token);
                let params =
                    [("email", user.email.as_str()), ("minutes", &minutes), ("link", &link)];
                RESET_LINK.mail(&user.email, &self.translations, language, &params)
            }
            None => {
                let params =
                    [("email", user.email.as_str()), ("minutes", &minutes), ("token", &token)];
                RESET_CODE.mail(&user.email, &self.translations, language, &params)
            }
        };
        self.mailer.send(tenant, mail).await
    }

    /// Đặt mật khẩu mới bằng `token` lấy từ mail. Khách lấy từ token (`request`: khách của
    /// request, nếu xác định được — xem `token::resolve`).
    pub(crate) async fn confirm(
        &self,
        request: Option<&TenantContext>,
        token: &str,
        new_password: String,
    ) -> Result<(), Error> {
        // Kiểm luật mật khẩu TRƯỚC khi dùng token: mật khẩu sai thì token còn dùng lại được.
        let problems: Vec<FieldError> = FieldError::blank("token", token)
            .into_iter()
            .chain(password::check_rules("password", &new_password))
            .collect();
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let Some((tenant, token)) = crate::token::resolve(&self.databases, request, token).await?
        else {
            return Err(invalid_token().with_reason(TokenRejection::Unknown.as_str()));
        };
        let tenant = &tenant;
        let consumed = tokens().consume(tenant.database().write(), tenant, PURPOSE, token).await?;
        let Some(user_id) = consumed.and_then(|consumed| consumed.subject.parse::<Uuid>().ok())
        else {
            let read = tenant.database().read();
            let why = tokens().rejection(read, tenant, PURPOSE, token).await?;
            return Err(invalid_token().with_reason(why.as_str()));
        };
        let actor = Actor::user(user_id);
        // Một transaction: mật khẩu mới, thu hồi phiên, passkey, API key, token đặt lại và
        // đổi email khác (D-156).
        self.users.recover_password(tenant, actor, user_id, new_password).await
    }
}

fn invalid_token() -> Error {
    Error::bad_request(
        "auth.invalid_reset_token",
        "This password-reset link is unknown, expired or already used. Ask for a new one.",
    )
}

fn now() -> Timestamp {
    Timestamp::now()
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::error::ErrorKind;
    use autoxide_testkit::TestDatabase;

    use crate::session::LoginOutcome;
    use crate::testing::Services;
    use crate::user::{NewUser, UserChanges};

    const OLD: &str = "correct horse battery staple";
    const NEW: &str = "a brand new passphrase";

    async fn setup(test: &TestDatabase) -> Services {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let ada = NewUser {
            email: "ada@example.com".to_owned(),
            name: "Ada".to_owned(),
            password: Some(OLD.to_owned()),
        };
        services.users.create(&tenant, Actor::system(), ada).await.unwrap();
        services
    }

    /// Token trong mail cuối cùng: chuỗi base64url 43 ký tự.
    fn last_token(services: &Services) -> String {
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

    fn sent(services: &Services) -> usize {
        services.mails.sent().len()
    }

    /// Cho thời gian trôi qua: token và lượt chặn gửi dồn coi như đã cũ.
    async fn time_passes(services: &Services, minutes: i64) {
        use sea_orm::ConnectionTrait as _;
        use sea_orm::sea_query::{Alias, Query};
        let update = Query::update()
            .table(Alias::new("auth_tokens"))
            .value(Alias::new("created_at"), now() - SignedDuration::from_mins(minutes))
            .value(Alias::new("expires_at"), now() + SignedDuration::from_mins(30 - minutes))
            .to_owned();
        services.database.write().execute(&update).await.unwrap();
    }

    /// Xoá tài khoản thì token đặt lại của người đó cũng bị xoá (bảng `auth_tokens` không có
    /// foreign key tới người dùng).
    #[tokio::test]
    async fn xoa_tai_khoan_xoa_luon_token() {
        TestDatabase::run(async |test| {
            let services = setup(test).await;
            let tenant = services.tenant.clone();
            crate::testing::request_reset(&services, "ada@example.com", Language::En).await;
            let token = last_token(&services);
            let ada =
                services.users.find_by_email(&tenant, "ada@example.com").await.unwrap().unwrap();
            services.users.delete(&tenant, Actor::system(), ada.id).await.unwrap();
            let error =
                services.resets.confirm(Some(&tenant), &token, NEW.to_owned()).await.unwrap_err();
            assert_eq!(error.code(), "auth.invalid_reset_token");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn chi_email_co_that_nhan_mail_va_theo_ngon_ngu() {
        TestDatabase::run(async |test| {
            let services = setup(test).await;
            let tenant = services.tenant.clone();

            crate::testing::request_reset(&services, "ghost@example.com", Language::En).await;
            assert_eq!(sent(&services), 0);
            crate::testing::request_reset(&services, " ADA@example.com", Language::Vi).await;
            assert_eq!(sent(&services), 1);
            {
                let mails = services.mails.sent();
                assert_eq!(mails[0].to, "ada@example.com");
                assert_eq!(mails[0].subject, "Đặt lại mật khẩu");
                assert!(mails[0].text.contains("trong 30 phút"), "{}", mails[0].text);
            }
            // Gửi dồn trong 60 giây: không thêm mail.
            crate::testing::request_reset(&services, "ada@example.com", Language::En).await;
            assert_eq!(sent(&services), 1);
            time_passes(&services, 2).await;
            crate::testing::request_reset(&services, "ada@example.com", Language::En).await;
            assert_eq!(sent(&services), 2);
            assert_eq!(services.mails.sent()[1].subject, "Reset your password");

            // Tài khoản bị khoá: không gửi.
            let ada =
                services.users.find_by_email(&tenant, "ada@example.com").await.unwrap().unwrap();
            let disable = UserChanges { name: None, disabled: Some(true), ..Default::default() };
            services.users.update(&tenant, Actor::system(), ada.id, disable, None).await.unwrap();
            time_passes(&services, 2).await;
            crate::testing::request_reset(&services, "ada@example.com", Language::En).await;
            assert_eq!(sent(&services), 2);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn dat_mat_khau_moi_bang_token_mot_lan() {
        TestDatabase::run(async |test| {
            let services = setup(test).await;
            let tenant = services.tenant.clone();
            let session = services
                .sessions
                .login(&tenant, "ada@example.com", OLD, None)
                .await
                .unwrap()
                .tokens();
            crate::testing::request_reset(&services, "ada@example.com", Language::En).await;
            let token = last_token(&services);

            // Mật khẩu sai luật: báo lỗi, token vẫn còn dùng được.
            let short = services
                .resets
                .confirm(Some(&tenant), &token, "short".to_owned())
                .await
                .unwrap_err();
            assert_eq!(short.code(), "validation_failed");
            services.resets.confirm(Some(&tenant), &token, NEW.to_owned()).await.unwrap();

            let again =
                services.resets.confirm(Some(&tenant), &token, NEW.to_owned()).await.unwrap_err();
            assert_eq!(again.kind(), ErrorKind::BadRequest);
            assert_eq!(again.code(), "auth.invalid_reset_token");
            assert_eq!(again.reason(), Some("used"));
            assert!(services.sessions.login(&tenant, "ada@example.com", OLD, None).await.is_err());
            let login =
                services.sessions.login(&tenant, "ada@example.com", NEW, None).await.unwrap();
            assert!(matches!(login, LoginOutcome::Tokens(_)));
            // Phiên cũ đã bị huỷ.
            assert!(
                services.sessions.refresh(Some(&tenant), &session.refresh_token).await.is_err()
            );
            let unknown =
                services.resets.confirm(Some(&tenant), "nope", NEW.to_owned()).await.unwrap_err();
            assert_eq!(unknown.code(), "auth.invalid_reset_token");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn token_het_han_va_token_cu_khong_dung_duoc() {
        TestDatabase::run(async |test| {
            let services = setup(test).await;
            let tenant = services.tenant.clone();
            crate::testing::request_reset(&services, "ada@example.com", Language::En).await;
            let older = last_token(&services);
            time_passes(&services, 2).await;
            crate::testing::request_reset(&services, "ada@example.com", Language::En).await;
            let newer = last_token(&services);

            services.resets.confirm(Some(&tenant), &newer, NEW.to_owned()).await.unwrap();
            // Đặt lại xong thì token cũ hơn cũng hết dùng được.
            let stale =
                services.resets.confirm(Some(&tenant), &older, NEW.to_owned()).await.unwrap_err();
            assert_eq!(stale.code(), "auth.invalid_reset_token");

            time_passes(&services, 2).await;
            crate::testing::request_reset(&services, "ada@example.com", Language::En).await;
            let expiring = last_token(&services);
            time_passes(&services, 31).await;
            let expired = services
                .resets
                .confirm(Some(&tenant), &expiring, NEW.to_owned())
                .await
                .unwrap_err();
            assert_eq!(expired.code(), "auth.invalid_reset_token");
            assert_eq!(expired.reason(), Some("expired"));
        })
        .await
        .unwrap();
    }

    /// Nhiều request dùng CÙNG một token cùng lúc: đúng một bên đặt được mật khẩu —
    /// trên cả hai database.
    #[tokio::test]
    async fn dung_cung_token_dong_thoi_chi_mot_ben_duoc() {
        TestDatabase::run(async |test| {
            let services = setup(test).await;
            let tenant = services.tenant.clone();
            crate::testing::request_reset(&services, "ada@example.com", Language::En).await;
            let token = last_token(&services);

            let attempts: Vec<_> = (0..5)
                .map(|index| {
                    let resets = services.resets.clone();
                    let token = token.clone();
                    let tenant = tenant.clone();
                    tokio::spawn(async move {
                        resets.confirm(Some(&tenant), &token, format!("{NEW} number {index}")).await
                    })
                })
                .collect();
            let mut succeeded = 0;
            for attempt in attempts {
                if attempt.await.unwrap().is_ok() {
                    succeeded += 1;
                }
            }

            assert_eq!(succeeded, 1);
        })
        .await
        .unwrap();
    }

    /// Đề bài giao diện: khách đã đặt `kernel.public_url` thì link trong thư mở trên tên miền của
    /// khách, giữ đường dẫn của địa chỉ máy chủ; khách khác vẫn dùng địa chỉ máy chủ.
    #[tokio::test]
    async fn link_trong_thu_theo_ten_mien_cua_khach() {
        TestDatabase::run(async |test| {
            let services = setup(test).await;
            let tenant = services.tenant.clone();
            let settings = std::sync::Arc::new(autoxide_testkit::FakeTenantSettings::default());
            let resets = PasswordResets {
                jobs: services.resets.jobs.clone(),
                users: services.resets.users.clone(),
                mailer: services.resets.mailer.clone(),
                translations: services.resets.translations.clone(),
                outbox: services.resets.outbox.clone(),
                links: autoxide_kernel::tenancy::SiteLinks::new(settings.clone()),
                url: Some("https://www.example.com/reset?token={token}".to_owned()),
                databases: services.resets.databases.clone(),
            };
            let job = |email: &str| ResetJob {
                email: email.to_owned(),
                realm: autoxide_kernel::access::Realm::Staff,
                language: "en".to_owned(),
            };
            resets.send(&tenant, &job("ada@example.com")).await.unwrap();
            let text = services.mails.sent().last().unwrap().text.clone();
            assert!(text.contains("https://www.example.com/reset?token="), "{text}");

            settings.set(tenant.id(), "kernel.public_url", "https://shop.acme.vn");
            time_passes(&services, 2).await;
            resets.send(&tenant, &job("ada@example.com")).await.unwrap();
            let text = services.mails.sent().last().unwrap().text.clone();
            assert!(text.contains("https://shop.acme.vn/reset?token="), "{text}");
        })
        .await
        .unwrap();
    }
}
