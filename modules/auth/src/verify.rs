//! Xác minh email qua mail (D-065).
//!
//! - Tài khoản mới (tự đăng ký hay do quản trị viên tạo qua API) nhận một mail chứa
//!   token; người dùng đã đăng nhập xin gửi lại được.
//! - Đổi email: cần mật khẩu hiện tại; mail đi tới địa chỉ MỚI, và email chỉ đổi khi
//!   token trong mail đó được xác nhận — không ai đổi được email sang địa chỉ không phải
//!   của mình.
//! - `confirm(token)`: token đúng, chưa dùng, chưa hết hạn thì ghi nhận email đã xác
//!   minh (và thay email nếu là đổi email); mọi token xác minh khác của người đó hết
//!   hiệu lực.

use std::sync::Arc;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::i18n::{Language, Translations};
use autoxide_kernel::mail::{MailTemplate, Mailer};
use autoxide_kernel::tokens::OneTimeTokens;
use chrono::{DateTime, SubsecRound, Utc};
use serde_json::json;
use uuid::Uuid;

use crate::api::User;
use crate::events;
use crate::session::SessionService;
use crate::user::{self, UserService};

/// Token sống chừng này: mail xác minh hay bị đọc muộn hơn mail đặt lại mật khẩu.
const TOKEN_HOURS: i64 = 24;
/// Mỗi địa chỉ nhận nhiều nhất một mail mỗi chừng này giây (chống dùng API để spam).
const COOLDOWN_SECONDS: i64 = 60;
/// Mục đích của token trong bảng `auth_tokens` (kernel, D-117): `subject` là ID người dùng,
/// `data` là địa chỉ email đang xác minh.
const PURPOSE: &str = "email_verification";

fn tokens() -> OneTimeTokens {
    OneTimeTokens::new("auth")
}

const VERIFY_LINK: MailTemplate = MailTemplate::new(
    "auth.mail.verify_email_link",
    "Confirm your email address",
    "Confirm that {email} is your email address by opening this link within {hours} \
     hours:\n\n{link}\n\nIf you did not ask for this, ignore this mail.",
);

const VERIFY_CODE: MailTemplate = MailTemplate::new(
    "auth.mail.verify_email_code",
    "Confirm your email address",
    "Confirm that {email} is your email address with this verification token within {hours} \
     hours (`POST /api/v1/auth/email-verification/confirm`):\n\n{token}\n\nIf you did not ask \
     for this, ignore this mail.",
);

pub(crate) struct EmailVerifications {
    pub database: Database,
    pub users: Arc<UserService>,
    pub sessions: Arc<SessionService>,
    pub mailer: Arc<dyn Mailer>,
    pub translations: Translations,
    pub outbox: Outbox,
    /// Trang của giao diện, có `{token}`; `None`: mail chứa token.
    pub url: Option<String>,
}

impl EmailVerifications {
    /// Gửi mail xác minh cho tài khoản vừa tạo. Hỏng thì chỉ ghi log: tài khoản đã tạo
    /// xong, người dùng xin gửi lại được (`send_current`).
    pub(crate) async fn after_signup(&self, actor: Actor, user: &User, language: Language) {
        if let Err(error) = self.send(actor, user, &user.email, language).await {
            tracing::warn!(user_id = %user.id, error = %error, "cannot send the verification mail");
        }
    }

    /// Gửi lại mail xác minh email hiện tại của người dùng `user_id`.
    pub(crate) async fn send_current(
        &self,
        user_id: Uuid,
        language: Language,
    ) -> Result<(), Error> {
        let user = self.users.get(user_id).await?.ok_or_else(|| user::user_not_found(user_id))?;
        if user.email_verified_at.is_some() {
            return Err(already_verified());
        }
        self.send(Actor::user(user_id), &user, &user.email, language).await
    }

    /// Người dùng `user_id` xin đổi email sang `new_email`: kiểm mật khẩu hiện tại, rồi
    /// gửi mail xác minh tới địa chỉ mới. Email chỉ đổi khi địa chỉ mới được xác nhận.
    pub(crate) async fn change_email(
        &self,
        user_id: Uuid,
        current_password: &str,
        new_email: &str,
        language: Language,
    ) -> Result<(), Error> {
        let email = user::normalize_email(new_email);
        let problems: Vec<FieldError> = user::check_email_field("new_email", &email)
            .into_iter()
            .chain(FieldError::blank("current_password", current_password))
            .collect();
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let (user, _) = self.sessions.check_password(user_id, current_password).await?;
        if email == user.email && user.email_verified_at.is_some() {
            return Err(already_verified());
        }
        if email != user.email && self.users.find_by_email(&email).await?.is_some() {
            return Err(user::email_taken(&email));
        }
        self.send(Actor::user(user_id), &user, &email, language).await
    }

    /// Xác nhận email bằng `token` lấy từ mail. Trả tài khoản sau khi xác minh.
    pub(crate) async fn confirm(&self, token: &str) -> Result<User, Error> {
        let now = now();
        let transaction = self.database.begin_write().await?;
        let consumed = tokens().consume(&transaction, PURPOSE, token).await?;
        let found = consumed
            .and_then(|consumed| Some((consumed.subject.parse::<Uuid>().ok()?, consumed.data?)));
        let Some((user_id, email)) = found else {
            transaction.rollback().await?;
            return Err(invalid_token());
        };
        let old = self.users.apply_verified_email(&transaction, user_id, &email, now).await?;
        // Token khác của người này (mail cũ hơn, hay cho địa chỉ khác) hết dùng được.
        // Mọi token xác minh khác của người này hết hiệu lực.
        tokens().revoke(&transaction, PURPOSE, &user_id.to_string()).await?;
        // Email đổi: mail đặt lại mật khẩu đã gửi tới địa chỉ cũ hết dùng được (D-156).
        if old != email {
            tokens().revoke(&transaction, "password_reset", &user_id.to_string()).await?;
        }
        let before = (old != email).then(|| json!({"email": old}));
        let event = events::EmailVerified { before, after: Some(json!({"email": email})) };
        let subject = events::user_subject(user_id);
        self.outbox.record(&transaction, Actor::user(user_id), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, changed = old != email, "email verified");
        self.users.get(user_id).await?.ok_or_else(|| user::user_not_found(user_id))
    }

    /// Tạo token cho (`user`, `email`) và gửi mail tới `email`. Vừa gửi cho địa chỉ đó
    /// chưa quá [`COOLDOWN_SECONDS`] thì thôi.
    async fn send(
        &self,
        actor: Actor,
        user: &User,
        email: &str,
        language: Language,
    ) -> Result<(), Error> {
        let now = now();
        let subject = user.id.to_string();
        let since = now - chrono::Duration::seconds(COOLDOWN_SECONDS);
        let recent = tokens().recent(self.database.write(), PURPOSE, &subject, since).await?;
        if recent.iter().any(|token| token.data.as_deref() == Some(email)) {
            return Ok(());
        }

        let transaction = self.database.begin_write().await?;
        let ttl = chrono::Duration::hours(TOKEN_HOURS);
        let token = tokens().issue(&transaction, PURPOSE, &subject, Some(email), ttl).await?.token;
        let event = events::EmailVerificationRequested {
            before: None,
            after: Some(json!({"email": email})),
        };
        self.outbox.record(&transaction, actor, events::user_subject(user.id), &event).await?;
        transaction.commit().await?;

        let hours = TOKEN_HOURS.to_string();
        let mail = match &self.url {
            Some(url) => {
                let link = url.replace("{token}", &token);
                let params = [("email", email), ("hours", &hours), ("link", &link)];
                VERIFY_LINK.mail(email, &self.translations, language, &params)
            }
            None => {
                let params = [("email", email), ("hours", &hours), ("token", &token)];
                VERIFY_CODE.mail(email, &self.translations, language, &params)
            }
        };
        self.mailer.send(mail).await
    }
}

fn invalid_token() -> Error {
    Error::bad_request(
        "auth.invalid_verification_token",
        "The email-verification token is unknown, expired or already used. Ask for a new \
         verification mail (`POST /api/v1/auth/me/email/verification`).",
    )
}

fn already_verified() -> Error {
    Error::conflict("auth.email_already_verified", "This email address is already verified.")
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_testkit::TestDatabase;

    use crate::testing::Services;
    use crate::user::NewUser;

    const PASSWORD: &str = "correct horse battery staple";

    async fn setup(test: &TestDatabase) -> (Services, User) {
        let services = crate::testing::services(test).await;
        let ada = NewUser {
            email: "ada@example.com".to_owned(),
            name: "Ada".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let ada = services.users.create(Actor::system(), ada).await.unwrap();
        (services, ada)
    }

    /// Người nhận và token của mail cuối cùng.
    fn last_mail(services: &Services) -> (String, String) {
        let sent = services.mails.sent.lock().unwrap();
        let mail = sent.last().unwrap();
        let token = mail
            .text
            .split_whitespace()
            .find(|word| {
                word.len() == 43
                    && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
            .unwrap()
            .to_owned();
        (mail.to.clone(), token)
    }

    fn sent(services: &Services) -> usize {
        services.mails.sent.lock().unwrap().len()
    }

    /// Cho thời gian trôi qua: lượt chặn gửi dồn hết, token gần hết hạn hơn.
    async fn time_passes(services: &Services, hours: i64) {
        use sea_orm::ConnectionTrait as _;
        use sea_orm::sea_query::{Alias, Query};
        let update = Query::update()
            .table(Alias::new("auth_tokens"))
            .value(Alias::new("created_at"), now() - chrono::Duration::minutes(2))
            .value(Alias::new("expires_at"), now() + chrono::Duration::hours(24 - hours))
            .to_owned();
        services.database.write().execute(&update).await.unwrap();
    }

    #[tokio::test]
    async fn tai_khoan_moi_xac_minh_bang_token_trong_mail() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let verifications = &services.verifications;
            assert!(ada.email_verified_at.is_none());

            verifications.after_signup(Actor::system(), &ada, Language::Vi).await;
            let (to, token) = last_mail(&services);
            assert_eq!(to, "ada@example.com");
            assert_eq!(services.mails.sent.lock().unwrap()[0].subject, "Xác nhận địa chỉ email");
            // Xin gửi lại ngay: không thêm mail.
            verifications.send_current(ada.id, Language::En).await.unwrap();
            assert_eq!(sent(&services), 1);

            let verified = verifications.confirm(&token).await.unwrap();
            assert_eq!(verified.email, "ada@example.com");
            assert!(verified.email_verified_at.is_some());
            let again = verifications.confirm(&token).await.unwrap_err();
            assert_eq!(again.code(), "auth.invalid_verification_token");
            let done = verifications.send_current(ada.id, Language::En).await.unwrap_err();
            assert_eq!(done.code(), "auth.email_already_verified");
            let unknown = verifications.confirm("nope").await.unwrap_err();
            assert_eq!(unknown.code(), "auth.invalid_verification_token");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn token_het_han_khong_dung_duoc() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            services.verifications.send_current(ada.id, Language::En).await.unwrap();
            let (_, token) = last_mail(&services);

            time_passes(&services, 25).await;

            let expired = services.verifications.confirm(&token).await.unwrap_err();
            assert_eq!(expired.code(), "auth.invalid_verification_token");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn doi_email_chi_xay_ra_khi_dia_chi_moi_duoc_xac_nhan() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let bob = NewUser {
                email: "bob@example.com".to_owned(),
                name: "Bob".to_owned(),
                password: None,
            };
            services.users.create(Actor::system(), bob).await.unwrap();
            let verifications = &services.verifications;
            verifications.send_current(ada.id, Language::En).await.unwrap();
            let (_, old_token) = last_mail(&services);

            let change = |password: &'static str, email: &'static str| {
                verifications.change_email(ada.id, password, email, Language::En)
            };
            assert_eq!(
                change("wrong", "ada@new.example").await.unwrap_err().code(),
                "auth.wrong_password"
            );
            assert_eq!(
                change(PASSWORD, "not an email").await.unwrap_err().code(),
                "validation_failed"
            );
            assert_eq!(
                change(PASSWORD, "BOB@example.com").await.unwrap_err().code(),
                "auth.email_taken"
            );
            change(PASSWORD, " Ada@New.example ").await.unwrap();
            let (to, token) = last_mail(&services);
            assert_eq!(to, "ada@new.example");
            // Chưa xác nhận: email cũ vẫn dùng.
            let before = services.users.get(ada.id).await.unwrap().unwrap();
            assert_eq!(before.email, "ada@example.com");

            let changed = verifications.confirm(&token).await.unwrap();
            assert_eq!(changed.email, "ada@new.example");
            assert!(changed.email_verified_at.is_some());
            services.sessions.login("ada@new.example", PASSWORD, None).await.unwrap();
            assert!(services.sessions.login("ada@example.com", PASSWORD, None).await.is_err());
            // Token cũ (cho địa chỉ cũ) hết dùng được: không quay email về được.
            let stale = verifications.confirm(&old_token).await.unwrap_err();
            assert_eq!(stale.code(), "auth.invalid_verification_token");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn email_bi_nguoi_khac_lay_trong_luc_cho_thi_khong_doi() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let verifications = &services.verifications;
            verifications
                .change_email(ada.id, PASSWORD, "shared@example.com", Language::En)
                .await
                .unwrap();
            let (_, token) = last_mail(&services);
            let carol = NewUser {
                email: "shared@example.com".to_owned(),
                name: "Carol".to_owned(),
                password: None,
            };
            services.users.create(Actor::system(), carol).await.unwrap();

            let taken = verifications.confirm(&token).await.unwrap_err();

            assert_eq!(taken.code(), "auth.email_taken");
            let ada = services.users.get(ada.id).await.unwrap().unwrap();
            assert_eq!((ada.email.as_str(), ada.email_verified_at), ("ada@example.com", None));
        })
        .await
        .unwrap();
    }

    /// Nhiều request cùng xác nhận một token: đúng một bên được — trên cả hai database.
    #[tokio::test]
    async fn xac_nhan_dong_thoi_chi_mot_ben_duoc() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            services.verifications.send_current(ada.id, Language::En).await.unwrap();
            let (_, token) = last_mail(&services);

            let attempts: Vec<_> = (0..5)
                .map(|_| {
                    let verifications = services.verifications.clone();
                    let token = token.clone();
                    tokio::spawn(async move { verifications.confirm(&token).await })
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
}
