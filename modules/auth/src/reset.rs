//! Đặt lại mật khẩu qua mail (D-063).
//!
//! 1. `request(email)`: có tài khoản đang hoạt động với email đó thì tạo token ngẫu
//!    nhiên (database giữ bản băm) và gửi mail. Không có thì không làm gì — nhưng bên gọi
//!    nhận cùng một kết quả, nên không dò được email nào đã đăng ký.
//! 2. `confirm(token, mật khẩu mới)`: token đúng, chưa dùng, chưa hết hạn thì đặt mật
//!    khẩu mới; mọi phiên đăng nhập và mọi token đặt lại khác của người đó hết hiệu lực.

use std::sync::Arc;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::i18n::{Language, Translations};
use autoxide_kernel::mail::{MailTemplate, Mailer};
use autoxide_kernel::tokens::OneTimeTokens;
use chrono::{DateTime, SubsecRound, Utc};
use uuid::Uuid;

use crate::events;
use crate::password;
use crate::user::UserService;

/// Token sống chừng này.
const TOKEN_MINUTES: i64 = 30;
/// Mỗi email nhận nhiều nhất một mail mỗi chừng này giây (chống dùng API để spam).
const COOLDOWN_SECONDS: i64 = 60;
/// Mục đích của token trong bảng `auth_tokens` (kernel, D-111); `subject` là ID người dùng.
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

pub(crate) struct PasswordResets {
    pub database: Database,
    pub users: Arc<UserService>,
    pub mailer: Arc<dyn Mailer>,
    pub translations: Translations,
    pub outbox: Outbox,
    /// Đặt lại mật khẩu là lúc chủ thật lấy lại tài khoản: passkey và API key gắn vào trước
    /// đó (có thể bởi kẻ gian) bị thu hồi (D-079).
    pub passkeys: Arc<crate::passkey::PasskeyService>,
    pub api_keys: Arc<crate::api_key::ApiKeyService>,
    /// Trang của giao diện, có `{token}`; `None`: mail chứa token.
    pub url: Option<String>,
}

impl PasswordResets {
    /// Gửi mail đặt lại mật khẩu nếu `email` là của một tài khoản đang hoạt động. Kết
    /// quả như nhau dù email có tồn tại hay không.
    pub(crate) async fn request(&self, email: &str, language: Language) -> Result<(), Error> {
        let Some(user) = self.users.find_by_email(email).await? else {
            return Ok(());
        };
        if !user.is_active() {
            return Ok(());
        }
        let now = now();
        let subject = user.id.to_string();
        let since = now - chrono::Duration::seconds(COOLDOWN_SECONDS);
        if !tokens().recent(self.database.write(), PURPOSE, &subject, since).await?.is_empty() {
            return Ok(());
        }

        let transaction = self.database.begin_write().await?;
        let ttl = chrono::Duration::minutes(TOKEN_MINUTES);
        let token = tokens().issue(&transaction, PURPOSE, &subject, None, ttl).await?.token;
        let event = events::PasswordResetRequested::default();
        let subject = events::user_subject(user.id);
        self.outbox.record(&transaction, Actor::system(), subject, &event).await?;
        transaction.commit().await?;

        let minutes = TOKEN_MINUTES.to_string();
        let mail = match &self.url {
            Some(url) => {
                let link = url.replace("{token}", &token);
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
        self.mailer.send(mail).await
    }

    /// Đặt mật khẩu mới bằng `token` lấy từ mail.
    pub(crate) async fn confirm(&self, token: &str, new_password: String) -> Result<(), Error> {
        // Kiểm luật mật khẩu TRƯỚC khi dùng token: mật khẩu sai thì token còn dùng lại được.
        let problems: Vec<FieldError> = FieldError::blank("token", token)
            .into_iter()
            .chain(password::check_rules("password", &new_password))
            .collect();
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let consumed = tokens().consume(self.database.write(), PURPOSE, token).await?;
        let user_id = consumed
            .and_then(|consumed| consumed.subject.parse::<Uuid>().ok())
            .ok_or_else(invalid_token)?;
        let actor = Actor::user(user_id);
        self.users.set_password(actor, user_id, new_password).await?;
        self.passkeys.remove_all(actor, user_id).await?;
        self.api_keys.revoke_all(actor, user_id).await?;
        // Mọi token đặt lại khác của người này hết hiệu lực.
        tokens().revoke(self.database.write(), PURPOSE, &user_id.to_string()).await?;
        Ok(())
    }
}

fn invalid_token() -> Error {
    Error::bad_request(
        "auth.invalid_reset_token",
        "The password-reset token is unknown, expired or already used. Ask for a new reset mail \
         (`POST /api/v1/auth/password-reset`).",
    )
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
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
        let ada = NewUser {
            email: "ada@example.com".to_owned(),
            name: "Ada".to_owned(),
            password: Some(OLD.to_owned()),
        };
        services.users.create(Actor::system(), ada).await.unwrap();
        services
    }

    /// Token trong mail cuối cùng: chuỗi base64url 43 ký tự.
    fn last_token(services: &Services) -> String {
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

    fn sent(services: &Services) -> usize {
        services.mails.sent.lock().unwrap().len()
    }

    /// Cho thời gian trôi qua: token và lượt chặn gửi dồn coi như đã cũ.
    async fn time_passes(services: &Services, minutes: i64) {
        use sea_orm::ConnectionTrait as _;
        use sea_orm::sea_query::{Alias, Query};
        let update = Query::update()
            .table(Alias::new("auth_tokens"))
            .value(Alias::new("created_at"), now() - chrono::Duration::minutes(minutes))
            .value(Alias::new("expires_at"), now() + chrono::Duration::minutes(30 - minutes))
            .to_owned();
        services.database.write().execute(&update).await.unwrap();
    }

    /// Xoá tài khoản thì token đặt lại của người đó cũng bị xoá (bảng `auth_tokens` không có
    /// foreign key tới người dùng).
    #[tokio::test]
    async fn xoa_tai_khoan_xoa_luon_token() {
        TestDatabase::run(async |test| {
            let services = setup(test).await;
            services.resets.request("ada@example.com", Language::En).await.unwrap();
            let token = last_token(&services);
            let ada = services.users.find_by_email("ada@example.com").await.unwrap().unwrap();
            services.users.delete(Actor::system(), ada.id).await.unwrap();
            let error = services.resets.confirm(&token, NEW.to_owned()).await.unwrap_err();
            assert_eq!(error.code(), "auth.invalid_reset_token");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn chi_email_co_that_nhan_mail_va_theo_ngon_ngu() {
        TestDatabase::run(async |test| {
            let services = setup(test).await;

            services.resets.request("ghost@example.com", Language::En).await.unwrap();
            assert_eq!(sent(&services), 0);
            services.resets.request(" ADA@example.com", Language::Vi).await.unwrap();
            assert_eq!(sent(&services), 1);
            {
                let mails = services.mails.sent.lock().unwrap();
                assert_eq!(mails[0].to, "ada@example.com");
                assert_eq!(mails[0].subject, "Đặt lại mật khẩu");
                assert!(mails[0].text.contains("trong 30 phút"), "{}", mails[0].text);
            }
            // Gửi dồn trong 60 giây: không thêm mail.
            services.resets.request("ada@example.com", Language::En).await.unwrap();
            assert_eq!(sent(&services), 1);
            time_passes(&services, 2).await;
            services.resets.request("ada@example.com", Language::En).await.unwrap();
            assert_eq!(sent(&services), 2);
            assert_eq!(services.mails.sent.lock().unwrap()[1].subject, "Reset your password");

            // Tài khoản bị khoá: không gửi.
            let ada = services.users.find_by_email("ada@example.com").await.unwrap().unwrap();
            let disable = UserChanges { name: None, disabled: Some(true) };
            services.users.update(Actor::system(), ada.id, disable, None).await.unwrap();
            time_passes(&services, 2).await;
            services.resets.request("ada@example.com", Language::En).await.unwrap();
            assert_eq!(sent(&services), 2);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn dat_mat_khau_moi_bang_token_mot_lan() {
        TestDatabase::run(async |test| {
            let services = setup(test).await;
            let session =
                services.sessions.login("ada@example.com", OLD, None).await.unwrap().tokens();
            services.resets.request("ada@example.com", Language::En).await.unwrap();
            let token = last_token(&services);

            // Mật khẩu sai luật: báo lỗi, token vẫn còn dùng được.
            let short = services.resets.confirm(&token, "short".to_owned()).await.unwrap_err();
            assert_eq!(short.code(), "validation_failed");
            services.resets.confirm(&token, NEW.to_owned()).await.unwrap();

            let again = services.resets.confirm(&token, NEW.to_owned()).await.unwrap_err();
            assert_eq!(again.kind(), ErrorKind::BadRequest);
            assert_eq!(again.code(), "auth.invalid_reset_token");
            assert!(services.sessions.login("ada@example.com", OLD, None).await.is_err());
            let login = services.sessions.login("ada@example.com", NEW, None).await.unwrap();
            assert!(matches!(login, LoginOutcome::Tokens(_)));
            // Phiên cũ đã bị huỷ.
            assert!(services.sessions.refresh(&session.refresh_token).await.is_err());
            let unknown = services.resets.confirm("nope", NEW.to_owned()).await.unwrap_err();
            assert_eq!(unknown.code(), "auth.invalid_reset_token");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn token_het_han_va_token_cu_khong_dung_duoc() {
        TestDatabase::run(async |test| {
            let services = setup(test).await;
            services.resets.request("ada@example.com", Language::En).await.unwrap();
            let older = last_token(&services);
            time_passes(&services, 2).await;
            services.resets.request("ada@example.com", Language::En).await.unwrap();
            let newer = last_token(&services);

            services.resets.confirm(&newer, NEW.to_owned()).await.unwrap();
            // Đặt lại xong thì token cũ hơn cũng hết dùng được.
            let stale = services.resets.confirm(&older, NEW.to_owned()).await.unwrap_err();
            assert_eq!(stale.code(), "auth.invalid_reset_token");

            time_passes(&services, 2).await;
            services.resets.request("ada@example.com", Language::En).await.unwrap();
            let expiring = last_token(&services);
            time_passes(&services, 31).await;
            let expired = services.resets.confirm(&expiring, NEW.to_owned()).await.unwrap_err();
            assert_eq!(expired.code(), "auth.invalid_reset_token");
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
            services.resets.request("ada@example.com", Language::En).await.unwrap();
            let token = last_token(&services);

            let attempts: Vec<_> = (0..5)
                .map(|index| {
                    let resets = services.resets.clone();
                    let token = token.clone();
                    tokio::spawn(async move {
                        resets.confirm(&token, format!("{NEW} number {index}")).await
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
}
