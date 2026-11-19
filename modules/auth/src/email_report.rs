//! Đổi email thì báo địa chỉ cũ (đặc tả 32 B1, D-250), để chủ tài khoản biết khi tài khoản bị
//! chiếm: kẻ biết mật khẩu đổi email sang địa chỉ của nó thì chủ thật mất cả đường đặt lại mật
//! khẩu.
//!
//! 1. Xác nhận địa chỉ mới (`verify.rs`, `confirm`) xếp job [`SEND_EMAIL_CHANGED`] trong CÙNG
//!    transaction: email không đổi (rollback) thì không có mail. Request không chờ mail.
//! 2. Job tạo token một lần (mục đích [`PURPOSE`], `subject` là ID người dùng, `data` là địa chỉ
//!    cũ, sống [`REPORT_DAYS`] ngày) và gửi tới **địa chỉ cũ**: tài khoản đã đổi sang địa chỉ
//!    mới (che bớt), lúc nào, và cách báo "không phải tôi".
//! 3. `report(token)` (`POST /api/v1/auth/email-change/report`, không cần đăng nhập), một
//!    transaction: dùng token, khoá dòng người dùng, trả email về địa chỉ cũ khi chưa ai lấy,
//!    khoá tài khoản (`UserService::lock_out_in`: bỏ mật khẩu, thu hồi phiên, API key, passkey,
//!    MFA, token đang chờ), ghi event `auth.email_change_reported`, rồi — khi email đã về địa
//!    chỉ cũ — xếp job gửi mail đặt lại mật khẩu tới đó (D-162). Địa chỉ cũ đã bị người khác lấy
//!    thì tài khoản vẫn bị khoá, nhưng không gửi mail đặt lại nào (mail đó sẽ tới tay người
//!    khác, hay tới kẻ chiếm): chủ tài khoản nhờ quản trị viên.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::mail::MailTemplate;
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, link_token};
use autoxide_kernel::tokens::{OneTimeTokens, TokenRejection};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::events;
use crate::reset::ResetJob;
use crate::user::entity as user_row;
use crate::verify::EmailVerifications;

/// Loại job gửi mail báo tới địa chỉ cũ.
pub(crate) const SEND_EMAIL_CHANGED: &str = "auth.send_email_changed";
/// Mục đích của token "không phải tôi" trong bảng `auth_tokens` (kernel, D-117).
pub(crate) const PURPOSE: &str = "email_change_report";
/// Token báo sống chừng này: chủ tài khoản có thể đọc mail muộn.
pub(crate) const REPORT_DAYS: i64 = 7;

const CHANGED_LINK: MailTemplate = MailTemplate::new(
    "auth.mail.email_changed_link",
    "The email address of your account changed",
    "The account that used {old_email} now signs in with {new_email} (changed at {at} UTC). This \
     mail goes to the old address so that you know.\n\nIf you made this change, there is \
     nothing to do.\n\nIf you did NOT, open this link within {days} days:\n\n{link}\n\nThe \
     account is then signed out everywhere, the address {old_email} is given back to it, and a \
     mail to choose a new password is sent here.",
);

const CHANGED_CODE: MailTemplate = MailTemplate::new(
    "auth.mail.email_changed_code",
    "The email address of your account changed",
    "The account that used {old_email} now signs in with {new_email} (changed at {at} UTC). This \
     mail goes to the old address so that you know.\n\nIf you made this change, there is \
     nothing to do.\n\nIf you did NOT, send this token within {days} days to `POST \
     /api/v1/auth/email-change/report`:\n\n{token}\n\nThe account is then signed out \
     everywhere, the address {old_email} is given back to it, and a mail to choose a new \
     password is sent here.",
);

fn tokens() -> OneTimeTokens {
    OneTimeTokens::new("auth")
}

/// Job [`SEND_EMAIL_CHANGED`]. Mang hai địa chỉ — job xong thì job queue xoá payload (D-079).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EmailChangedJob {
    pub user_id: Uuid,
    pub old_email: String,
    pub new_email: String,
    pub changed_at: DateTime<Utc>,
    /// Mã ngôn ngữ của mail (`en`, `vi`).
    pub language: String,
}

impl Job for EmailChangedJob {
    const KIND: &'static str = SEND_EMAIL_CHANGED;
}

/// Bên chạy job [`SEND_EMAIL_CHANGED`].
pub(crate) struct EmailChangedMailer(pub Arc<EmailVerifications>);

#[async_trait::async_trait]
impl JobHandler for EmailChangedMailer {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("an email-changed mail job must belong to a tenant"))?;
        let job: EmailChangedJob =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.send_email_changed(tenant, &job).await
    }
}

impl EmailVerifications {
    /// Xếp mail báo tới địa chỉ cũ, trong `transaction` của việc đổi email.
    pub(crate) async fn enqueue_email_changed(
        &self,
        tenant: &TenantContext,
        transaction: &sea_orm::DatabaseTransaction,
        job: &EmailChangedJob,
    ) -> Result<(), Error> {
        self.jobs.enqueue(transaction, tenant, job, EnqueueOptions::default()).await?;
        Ok(())
    }

    /// Job: tạo token báo và gửi mail tới địa chỉ cũ.
    pub(crate) async fn send_email_changed(
        &self,
        tenant: &TenantContext,
        job: &EmailChangedJob,
    ) -> Result<(), Error> {
        let language = if job.language == Language::Vi.tag() { Language::Vi } else { Language::En };
        let language = self.users.mail_language(tenant, job.user_id, language).await?;
        let ttl = chrono::Duration::days(REPORT_DAYS);
        let subject = job.user_id.to_string();
        let transaction = tenant.database().begin_write().await?;
        let raw = tokens()
            .issue(&transaction, tenant, PURPOSE, &subject, Some(&job.old_email), ttl)
            .await?
            .token;
        transaction.commit().await?;
        // Token trong mail mang khách: link mở trên tên miền chính vẫn tìm được khách.
        let token = link_token(tenant, &raw);
        let new_email = mask(&job.new_email);
        let at = job.changed_at.format("%Y-%m-%d %H:%M").to_string();
        let days = REPORT_DAYS.to_string();
        let mail = match &self.report_url {
            Some(url) => {
                let link = self.links.rewrite(tenant, url).await?.replace("{token}", &token);
                let params = [
                    ("old_email", job.old_email.as_str()),
                    ("new_email", &new_email),
                    ("at", &at),
                    ("days", &days),
                    ("link", &link),
                ];
                CHANGED_LINK.mail(&job.old_email, &self.translations, language, &params)
            }
            None => {
                let params = [
                    ("old_email", job.old_email.as_str()),
                    ("new_email", &new_email),
                    ("at", &at),
                    ("days", &days),
                    ("token", &token),
                ];
                CHANGED_CODE.mail(&job.old_email, &self.translations, language, &params)
            }
        };
        self.mailer.send(tenant, mail).await
    }

    /// Chủ tài khoản báo "không phải tôi" bằng `token` trong mail gửi tới địa chỉ cũ.
    /// Khách lấy từ token (`request`: khách của request, nếu xác định được — xem
    /// `token::resolve`).
    pub(crate) async fn report(
        &self,
        request: Option<&TenantContext>,
        token: &str,
        language: Language,
    ) -> Result<(), Error> {
        let Some((tenant, token)) = crate::token::resolve(&self.databases, request, token).await?
        else {
            return Err(invalid_report_token().with_reason(TokenRejection::Unknown.as_str()));
        };
        let tenant = &tenant;
        let transaction = tenant.database().begin_write().await?;
        let consumed = tokens().consume(&transaction, tenant, PURPOSE, token).await?;
        let found = consumed
            .and_then(|consumed| Some((consumed.subject.parse::<Uuid>().ok()?, consumed.data?)));
        let Some((user_id, old_email)) = found else {
            let why = tokens().rejection(&transaction, tenant, PURPOSE, token).await?;
            transaction.rollback().await?;
            return Err(invalid_report_token().with_reason(why.as_str()));
        };
        // Khoá dòng người dùng: hai lần báo cùng lúc (hay báo trong lúc đổi email) xét lần lượt.
        let locked = user_row::Entity::find_by_id(user_id).scoped(tenant);
        let Some(row) = rows::lock_one(&transaction, locked).await? else {
            // Tài khoản đã bị xoá: cùng lỗi như token sai, không cho dò gì.
            transaction.rollback().await?;
            return Err(invalid_report_token());
        };
        let current = row.email.clone();
        // Địa chỉ cũ còn trống thì trả lại (index unique: kiểm trước, vì lỗi ràng buộc làm hỏng
        // cả transaction trên Postgres).
        let taken = user_row::Entity::find()
            .filter(user_row::Column::Email.eq(old_email.as_str()))
            .filter(user_row::Column::Realm.eq(row.realm.as_str()))
            .filter(user_row::Column::Id.ne(user_id))
            .scoped(tenant)
            .one(&transaction)
            .await?
            .is_some();
        let restored = !taken;
        if restored && current != old_email {
            self.users
                .apply_verified_email(tenant, &transaction, user_id, &old_email, now())
                .await?;
        }
        let actor = Actor::user(user_id);
        self.users.lock_out_in(tenant, &transaction, actor, user_id).await?;
        // Mọi token báo khác của người này cũng hết dùng được.
        tokens().revoke(&transaction, tenant, PURPOSE, &user_id.to_string()).await?;
        let after = if restored { old_email.clone() } else { current.clone() };
        let event = events::EmailChangeReported {
            before: Some(json!({"email": current})),
            after: Some(json!({"email": after, "restored": restored})),
        };
        self.outbox
            .record(&transaction, tenant, actor, events::user_subject(user_id), &event)
            .await?;
        if restored {
            let job = ResetJob {
                email: old_email,
                realm: crate::user::realm_of(&row.realm),
                language: language.tag().to_owned(),
            };
            self.jobs.enqueue(&transaction, tenant, &job, EnqueueOptions::default()).await?;
        }
        transaction.commit().await?;
        tracing::warn!(user_id = %user_id, restored, "an email change was reported as not made by the owner");
        Ok(())
    }
}

/// Địa chỉ che bớt cho mail: `n***@example.org` — đủ để chủ nhận ra, không lộ trọn địa chỉ của
/// kẻ chiếm cho người đọc trộm hộp thư cũ.
pub(crate) fn mask(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let first: String = local.chars().take(1).collect();
            format!("{first}***@{domain}")
        }
        None => "***".to_owned(),
    }
}

pub(crate) fn invalid_report_token() -> Error {
    Error::bad_request(
        "auth.invalid_email_change_report_token",
        "This link is unknown, expired or already used.",
    )
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::pagination::{Order, PageRequest};
    use autoxide_testkit::TestDatabase;

    use crate::passkey::testing::SoftAuthenticator;
    use crate::testing::{Services, take_jobs};
    use crate::user::NewUser;

    const PASSWORD: &str = "correct horse battery staple";

    /// Token (43 ký tự base64url) trong mail cuối cùng gửi tới `to`.
    fn token_to(services: &Services, to: &str) -> String {
        let sent = services.mails.sent.lock().unwrap();
        let mail = sent.iter().rev().find(|mail| mail.to == to).unwrap();
        mail.text
            .split_whitespace()
            .find(|word| {
                word.len() == 43
                    && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
            .unwrap()
            .to_owned()
    }

    /// Ada đổi email sang `new_email` và xác nhận; chạy job báo. Trả token báo trong mail tới
    /// địa chỉ cũ.
    async fn change(services: &Services, ada: Uuid, new_email: &str) -> String {
        let tenant = services.tenant.clone();
        let verifications = &services.verifications;
        verifications.change_email(&tenant, ada, PASSWORD, new_email, Language::En).await.unwrap();
        let confirm = token_to(services, new_email);
        verifications.confirm(Some(&tenant), &confirm, Language::Vi).await.unwrap();
        let jobs = take_jobs(services, SEND_EMAIL_CHANGED).await;
        assert_eq!(jobs.len(), 1, "one notice per confirmed change");
        let job: EmailChangedJob = serde_json::from_str(&jobs[0]).unwrap();
        verifications.send_email_changed(&tenant, &job).await.unwrap();
        token_to(services, "ada@example.com")
    }

    async fn setup(test: &TestDatabase) -> (Services, Uuid) {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let ada = NewUser {
            email: "ada@example.com".to_owned(),
            name: "Ada".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let ada = services.users.create(&tenant, Actor::system(), ada).await.unwrap();
        (services, ada.id)
    }

    /// Đổi email thì địa chỉ cũ nhận mail (che địa chỉ mới, bằng ngôn ngữ của người xác nhận),
    /// job xếp cùng transaction: xác nhận thất bại (địa chỉ bị lấy mất) thì không có job.
    #[tokio::test]
    async fn doi_email_xep_mail_bao_dia_chi_cu() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let tenant = services.tenant.clone();
            change(&services, ada, "ada@new.example").await;
            let mail = services.mails.sent.lock().unwrap().last().unwrap().clone();
            assert_eq!(mail.to, "ada@example.com");
            assert_eq!(mail.subject, "Địa chỉ email của tài khoản đã đổi");
            assert!(mail.text.contains("a***@new.example"), "{}", mail.text);
            assert!(!mail.text.contains("ada@new.example"), "the new address is masked");

            // Xác nhận hỏng (người khác lấy địa chỉ trong lúc chờ): rollback, không có mail báo.
            let verifications = &services.verifications;
            verifications
                .change_email(&tenant, ada, PASSWORD, "taken@example.com", Language::En)
                .await
                .unwrap();
            let pending = token_to(&services, "taken@example.com");
            let bob = NewUser {
                email: "taken@example.com".to_owned(),
                name: "Bob".to_owned(),
                password: None,
            };
            services.users.create(&tenant, Actor::system(), bob).await.unwrap();
            let error =
                verifications.confirm(Some(&tenant), &pending, Language::En).await.unwrap_err();
            assert_eq!(error.code(), "auth.email_taken");
            assert!(take_jobs(&services, SEND_EMAIL_CHANGED).await.is_empty());
        })
        .await
        .unwrap();
    }

    /// "Không phải tôi": email về địa chỉ cũ, phiên / refresh token / API key / passkey / TOTP
    /// / mật khẩu đều hết, mail đặt lại mật khẩu xếp tới địa chỉ cũ, có event; token dùng một
    /// lần.
    #[tokio::test]
    async fn bao_khong_phai_toi_thu_hoi_moi_thu_va_tra_email_cu() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let tenant = services.tenant.clone();
            // Kẻ chiếm (biết mật khẩu) đăng nhập, gắn passkey, API key, TOTP, rồi đổi email.
            let crate::session::LoginOutcome::Tokens(tokens) =
                services.sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap()
            else {
                panic!("no second factor yet");
            };
            let mut device = SoftAuthenticator::new(7);
            let ceremony =
                services.passkeys.start_registration(&tenant, ada, PASSWORD).await.unwrap();
            let credential = device.register(&ceremony.options);
            services
                .passkeys
                .finish_registration(&tenant, ada, ceremony.id, "Phone", &credential)
                .await
                .unwrap();
            let key = crate::api_key::NewApiKey {
                allowed_ips: Vec::new(),
                kind: autoxide_kernel::access::ActorKind::Human,
                org_id: None,
                name: "agent".to_owned(),
                permissions: vec!["auth.users.read".to_owned()],
                expires_at: None,
                current_password: PASSWORD.to_owned(),
                second_factor: None,
            };
            let admin = autoxide_kernel::access::Principal::with_all_permissions(ada);
            services.api_keys.create(&tenant, &admin, key).await.unwrap();
            let setup = services.mfa.start_totp(&tenant, ada, "ada@example.com").await.unwrap();
            let unix = u64::try_from(Utc::now().timestamp()).unwrap();
            let code = crate::mfa::service::tests::code_at(&setup.secret, unix);
            services.mfa.confirm_totp(&tenant, ada, &code).await.unwrap();
            let report = change(&services, ada, "thief@evil.example").await;

            services.verifications.report(Some(&tenant), &report, Language::En).await.unwrap();
            let user = services.users.get(&tenant, ada).await.unwrap().unwrap();
            assert_eq!(user.email, "ada@example.com");
            assert!(services.sessions.refresh(Some(&tenant), &tokens.refresh_token).await.is_err());
            assert!(
                services.sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.is_err()
            );
            assert!(
                services
                    .sessions
                    .login(&tenant, "thief@evil.example", PASSWORD, None)
                    .await
                    .is_err()
            );
            let page = PageRequest::first(10, Order::Asc);
            assert!(services.api_keys.list(&tenant, ada, &page).await.unwrap().data.is_empty());
            assert!(services.passkeys.list(&tenant, ada, &page).await.unwrap().data.is_empty());
            assert!(!services.mfa.status(&tenant, ada).await.unwrap().totp_enabled);
            let resets = take_jobs(&services, crate::reset::SEND_RESET).await;
            assert_eq!(resets.len(), 1);
            assert!(resets[0].contains("ada@example.com"), "{}", resets[0]);

            // Token dùng một lần; token lạ: cùng lỗi.
            for token in [report.as_str(), "nope"] {
                let error = services
                    .verifications
                    .report(Some(&tenant), token, Language::En)
                    .await
                    .unwrap_err();
                assert_eq!(error.code(), "auth.invalid_email_change_report_token");
            }
        })
        .await
        .unwrap();
    }

    /// Địa chỉ cũ đã có người khác lấy: email giữ nguyên nhưng tài khoản vẫn bị khoá; không gửi
    /// mail đặt lại tới ai.
    #[tokio::test]
    async fn dia_chi_cu_da_bi_nguoi_khac_lay() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let tenant = services.tenant.clone();
            let report = change(&services, ada, "ada@new.example").await;
            let bob = NewUser {
                email: "ada@example.com".to_owned(),
                name: "Bob".to_owned(),
                password: Some(PASSWORD.to_owned()),
            };
            services.users.create(&tenant, Actor::system(), bob).await.unwrap();
            services.verifications.report(Some(&tenant), &report, Language::En).await.unwrap();
            let user = services.users.get(&tenant, ada).await.unwrap().unwrap();
            assert_eq!(user.email, "ada@new.example");
            assert!(
                services.sessions.login(&tenant, "ada@new.example", PASSWORD, None).await.is_err()
            );
            assert!(take_jobs(&services, crate::reset::SEND_RESET).await.is_empty());
            // Bob không bị gì.
            services.sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Hai lần báo cùng lúc với một token: một bên được.
    #[tokio::test]
    async fn bao_dong_thoi_chi_mot_ben_duoc() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let tenant = services.tenant.clone();
            let report = change(&services, ada, "ada@new.example").await;
            let (first, second) = tokio::join!(
                services.verifications.report(Some(&tenant), &report, Language::En),
                services.verifications.report(Some(&tenant), &report, Language::En),
            );
            assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn token_bao_het_han_thi_khong_dung_duoc() {
        TestDatabase::run(async |test| {
            let (services, ada) = setup(test).await;
            let tenant = services.tenant.clone();
            let report = change(&services, ada, "ada@new.example").await;
            use sea_orm::ConnectionTrait as _;
            use sea_orm::sea_query::{Alias, Query};
            let expire = Query::update()
                .table(Alias::new("auth_tokens"))
                .value(Alias::new("expires_at"), Utc::now() - chrono::Duration::minutes(1))
                .to_owned();
            services.database.write().execute(&expire).await.unwrap();
            let error = services
                .verifications
                .report(Some(&tenant), &report, Language::En)
                .await
                .unwrap_err();
            assert_eq!(error.code(), "auth.invalid_email_change_report_token");
        })
        .await
        .unwrap();
    }

    #[test]
    fn che_dia_chi() {
        assert_eq!(mask("nam@example.org"), "n***@example.org");
        assert_eq!(mask("đức@ví-dụ.vn"), "đ***@ví-dụ.vn");
        assert_eq!(mask("broken"), "***");
    }
}
