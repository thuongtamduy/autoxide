//! Xác nhận lại danh tính (bước 34 mục 9, D-308): người đã đăng nhập chứng minh lại mình là chủ
//! tài khoản bằng mật khẩu, mã TOTP, passkey hay link qua email, nhận một **token xác nhận lại**
//! ngắn hạn, dùng một lần. Lệnh nhạy cảm (tạo API key, đổi email, bật TOTP, thêm passkey, xoá tài
//! khoản) nhận token này thay cho `current_password` — tài khoản chỉ có passkey cũng làm được.
//!
//! Token lưu ở bảng `auth_tokens` (kernel, D-117) với mục đích `reauth`, chủ là ID người dùng;
//! dùng ở người khác hay sau khi hết hạn thì như token lạ.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::i18n::{Language, Translations};
use autoxide_kernel::mail::{MailTemplate, Mailer};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::tokens::OneTimeTokens;
use uuid::Uuid;

use crate::api_key::Reauth;
use crate::mfa::MfaService;
use crate::passkey::PasskeyService;
use crate::session::SessionService;

const PURPOSE: &str = "reauth";

fn tokens() -> OneTimeTokens {
    OneTimeTokens::new("auth")
}

const REAUTH_MAIL: MailTemplate = MailTemplate::new(
    "auth.mail.reauth",
    "Confirm it is you",
    "Use this confirmation token within {minutes} minutes to confirm a sensitive change to \
     your account (send it as `reauth_token`):\n\n{token}\n\nIf you did not ask for this, \
     change your password and sign out your other sessions.",
);

/// Cách chứng minh khi xin token xác nhận lại.
pub(crate) enum Proof {
    Password(String),
    /// Mã TOTP, hay passkey: như khi tạo API key.
    Factor(Reauth),
    /// Gửi token qua mail tới email của tài khoản.
    EmailLink,
    /// Mã SMS gửi tới số đăng nhập của tài khoản (`POST /phone/codes`), cho tài khoản chỉ có số.
    PhoneCode {
        challenge_id: Uuid,
        code: String,
    },
}

/// Token vừa phát; `token` là `None` khi gửi qua mail.
pub(crate) struct Issued {
    pub token: Option<String>,
    pub expires_at: Timestamp,
}

pub(crate) struct Reauthentication {
    pub sessions: Arc<SessionService>,
    pub mfa: Arc<MfaService>,
    pub passkeys: Arc<PasskeyService>,
    pub phone: Arc<crate::phone::PhoneSignIn>,
    pub mailer: Arc<dyn Mailer>,
    pub translations: Translations,
    /// Hạn của token (`AUTOXIDE_AUTH_REAUTH_MINUTES`).
    pub ttl: SignedDuration,
}

impl Reauthentication {
    /// Kiểm `proof` của người dùng `user_id`, rồi phát token xác nhận lại.
    pub(crate) async fn issue(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        proof: Proof,
        language: Language,
    ) -> Result<Issued, Error> {
        let mail_to = match proof {
            Proof::Password(password) => {
                if let Some(problem) = FieldError::blank("password", &password) {
                    return Err(Error::validation(vec![problem]));
                }
                self.sessions.check_password(tenant, user_id, &password).await?;
                None
            }
            Proof::Factor(Reauth::Totp(code)) => {
                self.mfa.reauthenticate(tenant, user_id, &code).await?;
                None
            }
            Proof::Factor(Reauth::Passkey { ceremony_id, credential }) => {
                self.passkeys.reauthenticate(tenant, user_id, ceremony_id, &credential).await?;
                None
            }
            // Token không đổi được lấy token mới (kéo dài hạn mãi).
            Proof::Factor(Reauth::Token(_)) => return Err(invalid()),
            Proof::EmailLink => {
                let user = self
                    .sessions
                    .users()
                    .get(tenant, user_id)
                    .await?
                    .ok_or_else(|| crate::user::user_not_found(user_id))?;
                // Tài khoản chỉ có số điện thoại (bước 47): không có địa chỉ để gửi.
                if user.email.is_empty() {
                    return Err(Error::validation(vec![FieldError::new(
                        "method",
                        "unavailable",
                        "this account has no email; use `phone_code`",
                    )]));
                }
                Some(user.email)
            }
            Proof::PhoneCode { challenge_id, code } => {
                self.phone.reauthenticate(tenant, user_id, challenge_id, &code).await?;
                None
            }
        };
        let transaction = tenant.database().begin_write().await?;
        let issued = tokens()
            .issue(&transaction, tenant, PURPOSE, &user_id.to_string(), None, self.ttl)
            .await?;
        transaction.commit().await?;
        let Some(email) = mail_to else {
            return Ok(Issued { token: Some(issued.token), expires_at: issued.expires_at });
        };
        let minutes = self.ttl.as_mins().to_string();
        let params = [("minutes", minutes.as_str()), ("token", issued.token.as_str())];
        let mail = REAUTH_MAIL.mail(&email, &self.translations, language, &params);
        self.mailer.send(tenant, mail).await?;
        Ok(Issued { token: None, expires_at: issued.expires_at })
    }
}

/// Dùng token xác nhận lại `token` của người dùng `user_id` (một lần). Token lạ, đã dùng, hết hạn
/// hay của người khác: `auth.reauth_token_invalid`.
pub(crate) async fn consume(
    tenant: &TenantContext,
    user_id: Uuid,
    token: &str,
) -> Result<(), Error> {
    let transaction = tenant.database().begin_write().await?;
    let consumed = tokens().consume(&transaction, tenant, PURPOSE, token.trim()).await?;
    if consumed.is_none_or(|consumed| consumed.subject != user_id.to_string()) {
        transaction.rollback().await?;
        return Err(invalid());
    }
    transaction.commit().await?;
    Ok(())
}

/// Cách chủ tài khoản xác nhận một lệnh nhạy cảm: mật khẩu hiện tại hay token xác nhận lại.
/// `&str` đổi thẳng thành `Password` (chỗ gọi cũ, test giữ nguyên).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Confirm<'a> {
    Password(&'a str),
    Token(&'a str),
}

impl<'a> From<&'a str> for Confirm<'a> {
    fn from(password: &'a str) -> Self {
        Confirm::Password(password)
    }
}

impl<'a> Confirm<'a> {
    /// Từ thân request: đúng MỘT trong `current_password` / `reauth_token`; thiếu hay thừa thì
    /// lỗi của ô `current_password` (bên gọi gom cùng lỗi khác).
    pub(crate) fn from_request(
        current_password: &'a str,
        reauth_token: Option<&'a str>,
    ) -> Result<Self, FieldError> {
        match reauth_token {
            None => match FieldError::blank("current_password", current_password) {
                Some(problem) => Err(problem),
                None => Ok(Confirm::Password(current_password)),
            },
            Some(token) if current_password.trim().is_empty() => Ok(Confirm::Token(token)),
            Some(_) => Err(FieldError::new(
                "current_password",
                "conflict",
                "give either the current password or `reauth_token`, not both",
            )),
        }
    }

    /// Kiểm chủ tài khoản `user_id`. Trả tài khoản và hash mật khẩu vừa kiểm (cho `lock_grant`;
    /// `None` khi dùng token).
    pub(crate) async fn check(
        self,
        sessions: &SessionService,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<(crate::api::User, Option<String>), Error> {
        match self {
            Confirm::Password(password) => {
                let (user, hash) = sessions.check_password(tenant, user_id, password).await?;
                Ok((user, Some(hash)))
            }
            Confirm::Token(token) => {
                consume(tenant, user_id, token).await?;
                let user = sessions
                    .users()
                    .get(tenant, user_id)
                    .await?
                    .ok_or_else(|| crate::user::user_not_found(user_id))?;
                Ok((user, None))
            }
        }
    }
}

fn invalid() -> Error {
    Error::forbidden(
        "auth.reauth_token_invalid",
        "The confirmation token is unknown, expired, already used or not yours. Get a new one \
         with `POST /api/v1/auth/me/reauth`.",
    )
}
