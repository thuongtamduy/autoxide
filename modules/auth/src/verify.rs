//! Xác minh email qua mail (D-065).
//!
//! - Tài khoản mới (tự đăng ký hay do quản trị viên tạo qua API) nhận một mail chứa
//!   token; người dùng đã đăng nhập xin gửi lại được.
//! - Đổi email: cần mật khẩu hiện tại; mail đi tới địa chỉ MỚI, và email chỉ đổi khi
//!   token trong mail đó được xác nhận — không ai đổi được email sang địa chỉ không phải
//!   của mình.
//! - `confirm(token)`: token đúng, chưa dùng, chưa hết hạn thì ghi nhận email đã xác
//!   minh (và thay email nếu là đổi email); mọi token xác minh khác của người đó hết
//!   hiệu lực. Email đổi thì địa chỉ cũ nhận mail báo kèm cách báo "không phải tôi"
//!   (`email_report.rs`).

use std::sync::Arc;

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::i18n::{Language, Translations};
use autoxide_kernel::job::JobQueue;
use autoxide_kernel::mail::{MailTemplate, Mailer};
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext, link_token};
use autoxide_kernel::tokens::{OneTimeTokens, TokenRejection};
use chrono::{DateTime, SubsecRound, Utc};
use serde_json::json;
use uuid::Uuid;

use crate::api::User;
use crate::email_report::EmailChangedJob;
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
    pub users: Arc<UserService>,
    pub sessions: Arc<SessionService>,
    pub mailer: Arc<dyn Mailer>,
    pub translations: Translations,
    pub outbox: Outbox,
    /// Hàng đợi job của `auth`: mail báo địa chỉ cũ khi đổi email (`email_report.rs`).
    pub jobs: JobQueue,
    /// Trang của giao diện, có `{token}`; `None`: mail chứa token.
    pub url: Option<String>,
    /// Trang báo "không phải tôi", có `{token}`; `None`: mail chứa token.
    pub report_url: Option<String>,
    /// Sổ database: tìm khách ghi trong token của link.
    pub databases: DatabaseRegistry,
}

impl EmailVerifications {
    /// Gửi mail xác minh cho tài khoản vừa tạo. Hỏng thì chỉ ghi log: tài khoản đã tạo
    /// xong, người dùng xin gửi lại được (`send_current`).
    pub(crate) async fn after_signup(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        user: &User,
        language: Language,
    ) {
        if let Err(error) = self.send(tenant, actor, user, &user.email, language).await {
            tracing::warn!(user_id = %user.id, error = %error, "cannot send the verification mail");
        }
    }

    /// Gửi lại mail xác minh email hiện tại của người dùng `user_id`.
    pub(crate) async fn send_current(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        language: Language,
    ) -> Result<(), Error> {
        self.send_current_by(tenant, Actor::user(user_id), user_id, language).await
    }

    /// Như [`send_current`](Self::send_current), do `actor` (vd người quản trị gửi lại thư cho
    /// một nhân viên) — bên gọi đã kiểm quyền.
    pub(crate) async fn send_current_by(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        user_id: Uuid,
        language: Language,
    ) -> Result<(), Error> {
        let user =
            self.users.get(tenant, user_id).await?.ok_or_else(|| user::user_not_found(user_id))?;
        if user.email_verified_at.is_some() {
            return Err(already_verified());
        }
        self.send(tenant, actor, &user, &user.email, language).await
    }

    /// Người dùng `user_id` xin đổi email sang `new_email`: kiểm mật khẩu hiện tại, rồi
    /// gửi mail xác minh tới địa chỉ mới. Email chỉ đổi khi địa chỉ mới được xác nhận.
    #[cfg(test)]
    pub(crate) async fn change_email(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        current_password: &str,
        new_email: &str,
        language: Language,
    ) -> Result<(), Error> {
        self.change_email_confirmed(tenant, user_id, current_password, None, new_email, language)
            .await
    }

    /// Như [`change_email`](Self::change_email), xác nhận bằng mật khẩu hay token xác nhận lại
    /// (`reauth_token`, D-308) — đúng một trong hai.
    pub(crate) async fn change_email_confirmed(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        current_password: &str,
        reauth_token: Option<&str>,
        new_email: &str,
        language: Language,
    ) -> Result<(), Error> {
        let email = autoxide_kernel::email::normalize(new_email);
        let confirm = crate::reauth::Confirm::from_request(current_password, reauth_token);
        let problems: Vec<FieldError> = user::check_email_field("new_email", &email)
            .into_iter()
            .chain(confirm.as_ref().err().cloned())
            .collect();
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let confirm = confirm.map_err(|problem| Error::validation(vec![problem]))?;
        let (user, _) = confirm.check(&self.sessions, tenant, user_id).await?;
        if email == user.email && user.email_verified_at.is_some() {
            return Err(already_verified());
        }
        if email != user.email
            && self.users.find_by_email_in(tenant, user.realm, &email).await?.is_some()
        {
            return Err(user::email_taken(&email));
        }
        self.send(tenant, Actor::user(user_id), &user, &email, language).await
    }

    /// Email đang chờ xác minh sau khi `user` xin đổi (địa chỉ mới, hạn của link); `None` khi
    /// không có (token xác minh còn dùng được mà địa chỉ khác email hiện tại).
    pub(crate) async fn pending_change(
        &self,
        tenant: &TenantContext,
        user: &User,
    ) -> Result<Option<(String, DateTime<Utc>)>, Error> {
        let subject = user.id.to_string();
        let live = tokens().live(tenant.database().read(), tenant, PURPOSE, &subject).await?;
        Ok(live.into_iter().find_map(|token| {
            let email = token.data.filter(|email| *email != user.email)?;
            Some((email, token.expires_at))
        }))
    }

    /// Huỷ yêu cầu đổi email của người dùng `user_id`: link đã gửi tới địa chỉ mới hết dùng được
    /// (cùng mọi link xác minh khác của tài khoản — xin lại được). Không có yêu cầu nào thì
    /// `auth.no_pending_email_change`.
    pub(crate) async fn cancel_change(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<(), Error> {
        let user =
            self.users.get(tenant, user_id).await?.ok_or_else(|| user::user_not_found(user_id))?;
        let Some((email, _)) = self.pending_change(tenant, &user).await? else {
            return Err(no_pending_change());
        };
        let transaction = tenant.database().begin_write().await?;
        tokens().revoke(&transaction, tenant, PURPOSE, &user_id.to_string()).await?;
        let event =
            events::EmailChangeCancelled { before: Some(json!({"email": email})), after: None };
        let actor = Actor::user(user_id);
        self.outbox
            .record(&transaction, tenant, actor, events::user_subject(user_id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, "pending email change cancelled");
        Ok(())
    }

    /// Xác nhận email bằng `token` lấy từ mail. Trả khách của token và tài khoản sau khi xác
    /// minh. Email đổi thì xếp mail báo tới địa chỉ cũ (bằng `language`) trong cùng transaction.
    /// Khách lấy từ token (`request`: khách của request, nếu xác định được — xem
    /// `token::resolve`).
    pub(crate) async fn confirm(
        &self,
        request: Option<&TenantContext>,
        token: &str,
        language: Language,
    ) -> Result<(TenantContext, User), Error> {
        let Some((tenant, raw)) = crate::token::resolve(&self.databases, request, token).await?
        else {
            return Err(invalid_token().with_reason(TokenRejection::Unknown.as_str()));
        };
        let user = self.confirm_in(&tenant, raw, language).await?;
        Ok((tenant, user))
    }

    async fn confirm_in(
        &self,
        tenant: &TenantContext,
        token: &str,
        language: Language,
    ) -> Result<User, Error> {
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        let consumed = tokens().consume(&transaction, tenant, PURPOSE, token).await?;
        let found = consumed
            .and_then(|consumed| Some((consumed.subject.parse::<Uuid>().ok()?, consumed.data?)));
        let Some((user_id, email)) = found else {
            let why = tokens().rejection(&transaction, tenant, PURPOSE, token).await?;
            transaction.rollback().await?;
            return Err(invalid_token().with_reason(why.as_str()));
        };
        let old =
            self.users.apply_verified_email(tenant, &transaction, user_id, &email, now).await?;
        // Token khác của người này (mail cũ hơn, hay cho địa chỉ khác) hết dùng được.
        // Mọi token xác minh khác của người này hết hiệu lực.
        tokens().revoke(&transaction, tenant, PURPOSE, &user_id.to_string()).await?;
        // Email đổi: mail đặt lại mật khẩu đã gửi tới địa chỉ cũ hết dùng được (D-156).
        if old != email {
            tokens().revoke(&transaction, tenant, "password_reset", &user_id.to_string()).await?;
            let job = EmailChangedJob {
                user_id,
                old_email: old.clone(),
                new_email: email.clone(),
                changed_at: now,
                language: language.tag().to_owned(),
            };
            self.enqueue_email_changed(tenant, &transaction, &job).await?;
        }
        let before = (old != email).then(|| json!({"email": old}));
        let event = events::EmailVerified { before, after: Some(json!({"email": email})) };
        let subject = events::user_subject(user_id);
        self.outbox.record(&transaction, tenant, Actor::user(user_id), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, changed = old != email, "email verified");
        self.users.get(tenant, user_id).await?.ok_or_else(|| user::user_not_found(user_id))
    }

    /// Tạo token cho (`user`, `email`) và gửi mail tới `email`. Vừa gửi cho địa chỉ đó
    /// chưa quá [`COOLDOWN_SECONDS`] thì thôi.
    async fn send(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        user: &User,
        email: &str,
        language: Language,
    ) -> Result<(), Error> {
        let now = now();
        let subject = user.id.to_string();
        let since = now - chrono::Duration::seconds(COOLDOWN_SECONDS);
        let recent =
            tokens().recent(tenant.database().write(), tenant, PURPOSE, &subject, since).await?;
        if recent.iter().any(|token| token.data.as_deref() == Some(email)) {
            return Ok(());
        }

        let transaction = tenant.database().begin_write().await?;
        let ttl = chrono::Duration::hours(TOKEN_HOURS);
        let raw =
            tokens().issue(&transaction, tenant, PURPOSE, &subject, Some(email), ttl).await?.token;
        // Token trong mail mang khách: link mở trên tên miền chính vẫn tìm được khách.
        let token = link_token(tenant, &raw);
        let event = events::EmailVerificationRequested {
            before: None,
            after: Some(json!({"email": email})),
        };
        self.outbox
            .record(&transaction, tenant, actor, events::user_subject(user.id), &event)
            .await?;
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
        self.mailer.send(tenant, mail).await
    }
}

fn invalid_token() -> Error {
    Error::bad_request(
        "auth.invalid_verification_token",
        "This verification link is unknown, expired or already used. Ask for a new verification mail.",
    )
}

fn no_pending_change() -> Error {
    Error::not_found(
        "auth.no_pending_email_change",
        "There is no email change waiting to be confirmed.",
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
        let tenant = services.tenant.clone();
        let ada = NewUser {
            email: "ada@example.com".to_owned(),
            name: "Ada".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let ada = services.users.create(&tenant, Actor::system(), ada).await.unwrap();
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
            let tenant = services.tenant.clone();
            let verifications = &services.verifications;
            assert!(ada.email_verified_at.is_none());

            verifications.after_signup(&tenant, Actor::system(), &ada, Language::Vi).await;
            let (to, token) = last_mail(&services);
            assert_eq!(to, "ada@example.com");
            assert_eq!(services.mails.sent.lock().unwrap()[0].subject, "Xác nhận địa chỉ email");
            // Xin gửi lại ngay: không thêm mail.
            verifications.send_current(&tenant, ada.id, Language::En).await.unwrap();
            assert_eq!(sent(&services), 1);

            let (_, verified) =
                verifications.confirm(Some(&tenant), &token, Language::En).await.unwrap();
            assert_eq!(verified.email, "ada@example.com");
            assert!(verified.email_verified_at.is_some());
            let again =
                verifications.confirm(Some(&tenant), &token, Language::En).await.unwrap_err();
            assert_eq!(again.code(), "auth.invalid_verification_token");
            // Giao diện nói được "link đã dùng rồi" thay vì "link sai".
            assert_eq!(again.reason(), Some("used"));
            let done = verifications.send_current(&tenant, ada.id, Language::En).await.unwrap_err();
            assert_eq!(done.code(), "auth.email_already_verified");
            let unknown =
                verifications.confirm(Some(&tenant), "nope", Language::En).await.unwrap_err();
            assert_eq!(unknown.code(), "auth.invalid_verification_token");
            assert_eq!(unknown.reason(), Some("unknown"));
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn token_het_han_khong_dung_duoc() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let tenant = services.tenant.clone();
            services.verifications.send_current(&tenant, ada.id, Language::En).await.unwrap();
            let (_, token) = last_mail(&services);

            time_passes(&services, 25).await;

            let expired = services
                .verifications
                .confirm(Some(&tenant), &token, Language::En)
                .await
                .unwrap_err();
            assert_eq!(expired.code(), "auth.invalid_verification_token");
            assert_eq!(expired.reason(), Some("expired"));
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn doi_email_chi_xay_ra_khi_dia_chi_moi_duoc_xac_nhan() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let tenant = services.tenant.clone();
            let bob = NewUser {
                email: "bob@example.com".to_owned(),
                name: "Bob".to_owned(),
                password: None,
            };
            services.users.create(&tenant, Actor::system(), bob).await.unwrap();
            let verifications = &services.verifications;
            verifications.send_current(&tenant, ada.id, Language::En).await.unwrap();
            let (_, old_token) = last_mail(&services);

            let change = |password: &'static str, email: &'static str| {
                verifications.change_email(&tenant, ada.id, password, email, Language::En)
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
            let before = services.users.get(&tenant, ada.id).await.unwrap().unwrap();
            assert_eq!(before.email, "ada@example.com");

            let (_, changed) =
                verifications.confirm(Some(&tenant), &token, Language::En).await.unwrap();
            assert_eq!(changed.email, "ada@new.example");
            assert!(changed.email_verified_at.is_some());
            services.sessions.login(&tenant, "ada@new.example", PASSWORD, None).await.unwrap();
            assert!(
                services.sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.is_err()
            );
            // Token cũ (cho địa chỉ cũ) hết dùng được: không quay email về được.
            let stale =
                verifications.confirm(Some(&tenant), &old_token, Language::En).await.unwrap_err();
            assert_eq!(stale.code(), "auth.invalid_verification_token");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn email_bi_nguoi_khac_lay_trong_luc_cho_thi_khong_doi() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let tenant = services.tenant.clone();
            let verifications = &services.verifications;
            verifications
                .change_email(&tenant, ada.id, PASSWORD, "shared@example.com", Language::En)
                .await
                .unwrap();
            let (_, token) = last_mail(&services);
            let carol = NewUser {
                email: "shared@example.com".to_owned(),
                name: "Carol".to_owned(),
                password: None,
            };
            services.users.create(&tenant, Actor::system(), carol).await.unwrap();

            let taken =
                verifications.confirm(Some(&tenant), &token, Language::En).await.unwrap_err();

            assert_eq!(taken.code(), "auth.email_taken");
            let ada = services.users.get(&tenant, ada.id).await.unwrap().unwrap();
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
            let tenant = services.tenant.clone();
            services.verifications.send_current(&tenant, ada.id, Language::En).await.unwrap();
            let (_, token) = last_mail(&services);

            let attempts: Vec<_> = (0..5)
                .map(|_| {
                    let verifications = services.verifications.clone();
                    let token = token.clone();
                    let tenant = tenant.clone();
                    tokio::spawn(async move {
                        verifications.confirm(Some(&tenant), &token, Language::En).await
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
