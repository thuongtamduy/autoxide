//! Đặt lại mật khẩu qua mail (D-063).
//!
//! 1. `request(email)`: có tài khoản đang hoạt động với email đó thì tạo token ngẫu
//!    nhiên (database giữ bản băm) và gửi mail. Không có thì không làm gì — nhưng bên gọi
//!    nhận cùng một kết quả, nên không dò được email nào đã đăng ký.
//! 2. `confirm(token, mật khẩu mới)`: token đúng, chưa dùng, chưa hết hạn thì đặt mật
//!    khẩu mới; mọi phiên đăng nhập và mọi token đặt lại khác của người đó hết hiệu lực.

use std::sync::Arc;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::i18n::{Language, Translations};
use autoxide_kernel::mail::{MailTemplate, Mailer};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use uuid::Uuid;

use self::entity::{Column, Entity};
use crate::events;
use crate::password;
use crate::token;
use crate::user::UserService;

/// Token sống chừng này.
const TOKEN_MINUTES: i64 = 30;
/// Mỗi email nhận nhiều nhất một mail mỗi chừng này giây (chống dùng API để spam).
const COOLDOWN_SECONDS: i64 = 60;

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

/// Entity của bảng `auth_password_reset` (SeaORM).
mod entity {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_password_reset")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub user_id: Uuid,
        #[sea_orm(unique)]
        pub token_hash: String,
        pub expires_at: DateTimeUtc,
        pub used_at: Option<DateTimeUtc>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

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
        let recent = Entity::find()
            .filter(Column::UserId.eq(user.id))
            .filter(Column::CreatedAt.gt(now - chrono::Duration::seconds(COOLDOWN_SECONDS)))
            .order_by_desc(Column::Id)
            .one(self.database.write())
            .await?;
        if recent.is_some() {
            return Ok(());
        }

        let token = token::random_token(32)?;
        let transaction = self.database.begin_write().await?;
        entity::ActiveModel {
            id: Set(Uuid::now_v7()),
            user_id: Set(user.id),
            token_hash: Set(token::sha256(&token)),
            expires_at: Set(now + chrono::Duration::minutes(TOKEN_MINUTES)),
            used_at: Set(None),
            created_at: Set(now),
        }
        .insert(&transaction)
        .await?;
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
        if let Some(problem) = password::check_rules("password", &new_password) {
            return Err(Error::validation(vec![problem]));
        }
        let now = now();
        let row = Entity::find()
            .filter(Column::TokenHash.eq(token::sha256(token)))
            .one(self.database.write())
            .await?
            .filter(|row| row.used_at.is_none() && row.expires_at > now)
            .ok_or_else(invalid_token)?;
        // Có điều kiện: hai request cùng dùng một token thì chỉ một bên được.
        let claimed = Entity::update_many()
            .col_expr(Column::UsedAt, Expr::value(Some(now)))
            .filter(Column::Id.eq(row.id))
            .filter(Column::UsedAt.is_null())
            .exec(self.database.write())
            .await?;
        if claimed.rows_affected == 0 {
            return Err(invalid_token());
        }
        let actor = Actor::user(row.user_id);
        self.users.set_password(actor, row.user_id, new_password).await?;
        self.passkeys.remove_all(actor, row.user_id).await?;
        self.api_keys.revoke_all(actor, row.user_id).await?;
        // Các token đặt lại khác của người này (mail cũ hơn) cũng hết dùng được.
        Entity::update_many()
            .col_expr(Column::UsedAt, Expr::value(Some(now)))
            .filter(Column::UserId.eq(row.user_id))
            .filter(Column::UsedAt.is_null())
            .exec(self.database.write())
            .await?;
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
        Entity::update_many()
            .col_expr(Column::CreatedAt, Expr::value(now() - chrono::Duration::minutes(minutes)))
            .col_expr(
                Column::ExpiresAt,
                Expr::value(now() + chrono::Duration::minutes(30 - minutes)),
            )
            .exec(services.database.write())
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
            services.users.update(Actor::system(), ada.id, disable).await.unwrap();
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
            let session = services.sessions.login("ada@example.com", OLD).await.unwrap().tokens();
            services.resets.request("ada@example.com", Language::En).await.unwrap();
            let token = last_token(&services);

            // Mật khẩu sai luật: báo lỗi, token vẫn còn dùng được.
            let short = services.resets.confirm(&token, "short".to_owned()).await.unwrap_err();
            assert_eq!(short.code(), "validation_failed");
            services.resets.confirm(&token, NEW.to_owned()).await.unwrap();

            let again = services.resets.confirm(&token, NEW.to_owned()).await.unwrap_err();
            assert_eq!(again.kind(), ErrorKind::BadRequest);
            assert_eq!(again.code(), "auth.invalid_reset_token");
            assert!(services.sessions.login("ada@example.com", OLD).await.is_err());
            let login = services.sessions.login("ada@example.com", NEW).await.unwrap();
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
