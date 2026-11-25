//! Service của phiên đăng nhập.
//!
//! Đăng nhập cấp hai token:
//!
//! - **access token** (JWT, sống ngắn, mặc định 15 phút): gửi kèm mọi request; máy
//!   chủ kiểm chữ ký, không hỏi database;
//! - **refresh token** (chuỗi ngẫu nhiên, sống dài, mặc định 30 ngày): chỉ dùng để
//!   đổi lấy cặp token mới. Database lưu bản băm của nó.
//!
//! **Xoay vòng (rotation):** mỗi refresh token chỉ đổi được MỘT lần; lần đổi đó cấp
//! refresh token mới cùng "họ" (family). **Phát hiện dùng lại (reuse detection):**
//! một refresh token đã đổi rồi mà bị gửi lại nghĩa là có hai bên giữ nó — người
//! dùng và kẻ đã đánh cắp. Không biết bên nào là thật, nên thu hồi cả họ: cả hai
//! đều phải đăng nhập lại.
//!
//! **Khách của token (bước 33):** access token mang claim `tid` (xem `token.rs`); refresh token
//! và `mfa_token` mang khách theo `autoxide_kernel::tenancy::link_token` (khách mặc định giữ dạng
//! cũ, khách khác `t<ID khách>.<phần thô>`) — đổi, đăng xuất, nhập mã MFA chạy được trên tên miền
//! chính không header: khách lấy từ chính token ([`token::resolve`]). Database chỉ giữ bản băm
//! của phần thô.

use autoxide_kernel::access::Realm;
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::rows;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
};
use uuid::Uuid;

use super::entity::{self, Column, Entity};
use crate::api::User;
use crate::token::{self, AccessTokens};
use crate::user::UserService;

use super::{LoginThrottle, challenge};
use crate::events;
use crate::mfa::MfaService;
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::tenancy::{DatabaseRegistry, ScopeExt, TenantContext, link_token};
use serde_json::json;

mod devices;
mod refresh;
use devices::*;

/// Mã lỗi khi sai email hoặc mật khẩu (tạo ở `user::service`).
const INVALID_CREDENTIALS: &str = "auth.invalid_credentials";

/// Số byte ngẫu nhiên của một refresh token: 256 bit.
const REFRESH_TOKEN_BYTES: usize = 32;

/// Cặp token trả cho client sau khi đăng nhập hoặc đổi token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tokens {
    pub access_token: String,
    /// Số giây access token còn sống.
    pub expires_in: u64,
    pub refresh_token: String,
    pub refresh_token_expires_at: Timestamp,
    /// Phiên mà cặp token này thuộc về (họ refresh token): client giữ lại để nhận ra
    /// phiên của chính mình trong danh sách phiên.
    pub session_id: Uuid,
}

/// Một phiên đăng nhập còn hiệu lực: một họ refresh token, nhìn từ phía người dùng.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Session {
    pub id: Uuid,
    pub user_agent: Option<String>,
    /// Cách mở phiên; `None` với phiên cũ (D-272).
    pub sign_in_method: Option<String>,
    /// Quốc gia gần đúng, khi biết.
    pub country: Option<String>,
    pub started_at: Timestamp,
    /// Lần cấp token gần nhất (đăng nhập hoặc refresh).
    pub last_active_at: Timestamp,
    pub expires_at: Timestamp,
}

impl From<entity::Model> for Session {
    fn from(row: entity::Model) -> Self {
        Session {
            id: row.family_id,
            user_agent: row.user_agent,
            sign_in_method: row.sign_in_method,
            country: row.country,
            started_at: row.session_started_at.unwrap_or(row.created_at),
            last_active_at: row.created_at,
            expires_at: row.expires_at,
        }
    }
}

/// Độ dài tối đa của `User-Agent` được lưu (khớp cột của m0015).
const USER_AGENT_MAX_CHARS: usize = 255;

pub(crate) struct SessionService {
    users: Arc<UserService>,
    access_tokens: AccessTokens,
    refresh_token_ttl: SignedDuration,
    throttle: LoginThrottle,
    mfa: Arc<MfaService>,
    outbox: Outbox,
    /// Sổ database: tìm khách ghi trong refresh token / `mfa_token`.
    databases: DatabaseRegistry,
}

/// Kết quả của bước nhập mật khẩu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LoginOutcome {
    /// Đăng nhập xong.
    Tokens(Tokens),
    /// Cần mã MFA: gửi `mfa_token` cùng mã tới `complete_mfa` trong `expires_in` giây.
    MfaRequired { mfa_token: String, expires_in: u64 },
}

#[cfg(test)]
impl LoginOutcome {
    /// Cặp token, cho test của tài khoản chưa bật MFA.
    pub(crate) fn tokens(self) -> Tokens {
        match self {
            LoginOutcome::Tokens(tokens) => tokens,
            LoginOutcome::MfaRequired { .. } => panic!("expected tokens, got an MFA challenge"),
        }
    }
}

impl SessionService {
    pub(crate) fn new(
        users: Arc<UserService>,
        access_tokens: AccessTokens,
        refresh_token_ttl: std::time::Duration,
        throttle: LoginThrottle,
        mfa: Arc<MfaService>,
        outbox: Outbox,
        databases: DatabaseRegistry,
    ) -> Result<Self, Error> {
        let refresh_token_ttl = SignedDuration::try_from(refresh_token_ttl)
            .map_err(|error| Error::internal(format!("refresh token lifetime: {error}")))?;
        Ok(SessionService {
            users,
            access_tokens,
            refresh_token_ttl,
            throttle,
            mfa,
            outbox,
            databases,
        })
    }

    /// Đăng nhập bằng email và mật khẩu. Tài khoản chưa bật MFA: mở một họ refresh
    /// token mới. Đã bật: mở một lượt chờ mã ([`LoginOutcome::MfaRequired`]); token
    /// chỉ cấp ở [`complete_mfa`](Self::complete_mfa).
    ///
    /// Email đang bị khoá vì sai quá nhiều lần thì trả 429 ngay, KHÔNG kiểm mật khẩu
    /// — kẻ đoán không biết được lần đoán nào đúng.
    /// `user_agent`: header `User-Agent` của client, để người dùng nhận ra phiên này.
    #[cfg(test)]
    pub(crate) async fn login(
        &self,
        tenant: &TenantContext,
        email: &str,
        password: &str,
        user_agent: Option<&str>,
    ) -> Result<LoginOutcome, Error> {
        self.login_in(tenant, Realm::Staff, email, password, None, user_agent).await
    }

    /// Như [`login`](Self::login), cho tài khoản `realm` (D-309). Bộ đếm lần sai của người mua
    /// tách khỏi của nhân sự cùng email.
    /// `ip`: địa chỉ client — lần sai đếm theo cặp email + địa chỉ (D-20261121-login-throttle-pair).
    pub(crate) async fn login_in(
        &self,
        tenant: &TenantContext,
        realm: Realm,
        email: &str,
        password: &str,
        ip: Option<std::net::IpAddr>,
        user_agent: Option<&str>,
    ) -> Result<LoginOutcome, Error> {
        let throttled = throttle_key(realm, email);
        let email_for_check = email;
        let email = throttled.as_str();
        // Lượt thử được đếm TRƯỚC khi kiểm mật khẩu; đúng thì xoá bộ đếm bên dưới.
        self.throttle.reserve_login(tenant, email, ip).await?;
        let (user, hash) =
            self.users.verify_credentials_in(tenant, realm, email_for_check, password).await?;
        if self.mfa.status(tenant, user.id).await?.totp_enabled {
            // Mật khẩu đúng: trả lại lượt vừa xin, nhưng chưa xoá các lần sai trước — đúng
            // mật khẩu mới là nửa đường.
            self.throttle.refund_login(tenant, email, ip).await?;
            let write = tenant.database().write();
            let raw = challenge::open(write, tenant, user.id, now()).await?;
            return Ok(LoginOutcome::MfaRequired {
                mfa_token: link_token(tenant, &raw),
                expires_in: challenge::CHALLENGE_TTL_SECONDS.unsigned_abs(),
            });
        }
        if crate::role::passkey_required(tenant, user.id).await? {
            // Mật khẩu đúng nhưng chưa đủ: như MFA, trả lại lượt vừa xin.
            self.throttle.refund_login(tenant, email, ip).await?;
            return Err(crate::role::passkey_required_error());
        }
        self.throttle.clear_login(tenant, email, ip).await?;
        let proof = Proof::Password(hash);
        let tokens = self.start_session(tenant, user.id, "password", user_agent, proof).await?;
        Ok(LoginOutcome::Tokens(tokens))
    }

    /// Người dùng `user_id` (đang đăng nhập) tự đổi mật khẩu, kèm mật khẩu hiện tại.
    ///
    /// Mỗi lần nhập sai mật khẩu hiện tại được tính vào login throttling của email như
    /// một lần đăng nhập sai: ai cầm được access token cũng không đoán mãi được mật
    /// khẩu. Đổi xong thì mọi phiên cũ kết thúc; trả cặp token của một phiên mới để
    /// client đang dùng không phải đăng nhập lại. `revoke_other`: gỡ luôn API key, passkey, liên kết
    /// đăng nhập ngoài như khi khôi phục mật khẩu (L-7 của rà soát bảo mật 2026-11).
    pub(crate) async fn change_password(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        current: &str,
        new: String,
        revoke_other: bool,
        user_agent: Option<&str>,
    ) -> Result<Tokens, Error> {
        // Mọi lỗi của đầu vào báo một lần.
        let problems: Vec<FieldError> = FieldError::blank("current_password", current)
            .into_iter()
            .chain(crate::password::check_rules("new_password", &new))
            .collect();
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        self.check_password(tenant, user_id, current).await?;
        self.users
            .write_password(
                tenant,
                Actor::user(user_id),
                user_id,
                "new_password",
                new,
                if revoke_other {
                    crate::user::PasswordWrite::ChangeRevokingAll
                } else {
                    crate::user::PasswordWrite::Change
                },
            )
            .await?;
        self.start_session(tenant, user_id, "password_change", user_agent, Proof::Active).await
    }

    /// Mở phiên cho người dùng `user_id` vừa chứng minh qua kênh khác (vd chấp nhận lời mời bằng
    /// link trong mail tới chính email đó, D-312). Tài khoản bị khoá thì từ chối.
    pub(crate) async fn open_session(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        method: &str,
        user_agent: Option<&str>,
    ) -> Result<Tokens, Error> {
        self.start_session(tenant, user_id, method, user_agent, Proof::Active).await
    }

    /// Người dùng `user_id` vừa chứng minh bằng link qua email (D-311): bật MFA thì vẫn cần mã
    /// (như `/login`), không thì mở phiên. Tài khoản bị khoá thì từ chối.
    pub(crate) async fn login_with_link(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        user_agent: Option<&str>,
    ) -> Result<LoginOutcome, Error> {
        self.login_verified(tenant, user_id, "email_link", user_agent).await
    }

    /// Người dùng `user_id` vừa chứng minh qua kênh `method` (`email_link`, `oidc`): bật MFA thì
    /// vẫn cần mã như `/login`, không thì mở phiên. Tài khoản bị khoá thì từ chối.
    pub(crate) async fn login_verified(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        method: &str,
        user_agent: Option<&str>,
    ) -> Result<LoginOutcome, Error> {
        let user = self.users.get(tenant, user_id).await?.ok_or_else(token::invalid_token)?;
        if !user.is_active() {
            return Err(crate::user::user_disabled());
        }
        if self.mfa.status(tenant, user.id).await?.totp_enabled {
            let write = tenant.database().write();
            let raw = challenge::open(write, tenant, user.id, now()).await?;
            return Ok(LoginOutcome::MfaRequired {
                mfa_token: link_token(tenant, &raw),
                expires_in: challenge::CHALLENGE_TTL_SECONDS.unsigned_abs(),
            });
        }
        if crate::role::passkey_required(tenant, user.id).await? {
            return Err(crate::role::passkey_required_error());
        }
        let tokens = self.start_session(tenant, user.id, method, user_agent, Proof::Active).await?;
        Ok(LoginOutcome::Tokens(tokens))
    }

    /// Mở phiên cho người dùng `user_id` vừa chứng minh bằng passkey (đã kiểm ở
    /// `PasskeyService`). Tài khoản bị khoá thì từ chối.
    pub(crate) async fn login_with_passkey(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        passkey_id: Uuid,
        user_agent: Option<&str>,
    ) -> Result<Tokens, Error> {
        let user = self
            .users
            .get(tenant, user_id)
            .await?
            .ok_or_else(|| crate::user::user_not_found(user_id))?;
        if !user.is_active() {
            return Err(crate::user::user_disabled());
        }
        let proof = Proof::Passkey(passkey_id);
        self.start_session(tenant, user_id, "passkey", user_agent, proof).await
    }

    /// Kiểm mật khẩu hiện tại của người dùng `user_id` trước một việc nhạy cảm (đổi mật
    /// khẩu, đổi email). Sai thì `auth.wrong_password` và tính vào login throttling.
    ///
    /// Trả kèm hash mật khẩu vừa kiểm: bên cấp quyền mới (API key) đưa nó vào
    /// [`lock_grant`] trong transaction ghi của mình.
    /// Dịch vụ tài khoản phiên dùng chung.
    pub(crate) fn users(&self) -> &Arc<UserService> {
        &self.users
    }

    pub(crate) async fn check_password(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        current: &str,
    ) -> Result<(User, String), Error> {
        let user = self
            .users
            .get(tenant, user_id)
            .await?
            .ok_or_else(|| crate::user::user_not_found(user_id))?;
        // Đếm theo tài khoản (không theo email: người mua chỉ có số điện thoại có email rỗng), và
        // kiểm mật khẩu trong đúng loại tài khoản của người dùng.
        let key = account_key(user.id);
        self.throttle.reserve(tenant, &key).await?;
        match self.users.verify_credentials_in(tenant, user.realm, &user.email, current).await {
            Ok(verified) => {
                self.throttle.clear(tenant, &key).await?;
                Ok(verified)
            }
            Err(error) if error.code() == INVALID_CREDENTIALS => Err(Error::bad_request(
                "auth.wrong_password",
                "The current password is incorrect. If you forgot it, you can reset it by email.",
            )),
            Err(error) => Err(error),
        }
    }

    /// Bắt đầu bật TOTP cho người dùng `user_id`, kèm mật khẩu hiện tại (sai thì tính vào login
    /// throttling như [`check_password`](Self::check_password)): access token bị lộ không đủ để
    /// gắn ứng dụng authenticator của kẻ gian vào tài khoản rồi khoá chủ thật ở ngoài (D-156).
    pub(crate) async fn start_totp<'a>(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        confirm: impl Into<crate::reauth::Confirm<'a>>,
    ) -> Result<crate::mfa::TotpSetup, Error> {
        let (user, _) = confirm.into().check(self, tenant, user_id).await?;
        self.mfa.start_totp(tenant, user.id, &user.email).await
    }

    /// Người dùng `user_id` tự xoá tài khoản của mình, kèm mật khẩu hiện tại (sai thì
    /// tính vào login throttling như [`check_password`](Self::check_password)).
    pub(crate) async fn delete_own_account<'a>(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        confirm: impl Into<crate::reauth::Confirm<'a>>,
    ) -> Result<(), Error> {
        confirm.into().check(self, tenant, user_id).await?;
        self.users.delete(tenant, Actor::user(user_id), user_id).await
    }

    /// Bước hai của đăng nhập: `code` là mã TOTP hoặc một recovery code. Mỗi lần sai
    /// được tính vào login throttling của email, như một lần sai mật khẩu. Khách lấy từ
    /// `mfa_token` (`request`: khách của request, nếu xác định được — xem [`token::resolve`]);
    /// trả kèm khách đó.
    pub(crate) async fn complete_mfa(
        &self,
        request: Option<&TenantContext>,
        mfa_token: &str,
        code: &str,
        user_agent: Option<&str>,
    ) -> Result<(TenantContext, Tokens), Error> {
        let Some((tenant, raw)) = token::resolve(&self.databases, request, mfa_token).await? else {
            return Err(challenge::invalid_mfa_token());
        };
        let tokens = self.complete_mfa_in(&tenant, raw, code, user_agent).await?;
        Ok((tenant, tokens))
    }

    async fn complete_mfa_in(
        &self,
        tenant: &TenantContext,
        mfa_token: &str,
        code: &str,
        user_agent: Option<&str>,
    ) -> Result<Tokens, Error> {
        let write = tenant.database().write();
        let (challenge_id, user_id) =
            challenge::claim_attempt(write, tenant, mfa_token, now()).await?;
        let user =
            self.users.get(tenant, user_id).await?.ok_or_else(challenge::invalid_mfa_token)?;
        let key = account_key(user.id);
        self.throttle.reserve(tenant, &key).await?;
        if !self.mfa.verify_second_factor(tenant, user.id, code).await? {
            return Err(Error::unauthenticated(
                "auth.invalid_mfa_code",
                "The code is wrong, expired or already used. Enter the current code from your \
                 authenticator app, or an unused recovery code.",
            ));
        }

        if !user.is_active() {
            return Err(crate::user::user_disabled());
        }
        self.throttle.clear(tenant, &key).await?;
        let proof = Proof::Challenge(challenge_id);
        self.start_session(tenant, user.id, "password_and_mfa", user_agent, proof).await
    }

    /// Mở một họ refresh token mới và cấp cặp token đầu tiên.
    /// `method`: cách đã xác thực (`password`, `password_and_mfa`), ghi vào audit.
    ///
    /// `proof`: điều đã chứng minh người dùng, kiểm lại TRONG transaction cấp phiên (D-156).
    /// Đổi / đặt lại mật khẩu ghi dòng người dùng, xoá lượt chờ MFA và passkey trong
    /// transaction của nó, nên một trong hai bên chờ bên kia: mật khẩu đổi giữa lúc kiểm và
    /// lúc cấp thì không cấp; cấp trước thì phiên bị thu hồi cùng các phiên khác.
    async fn start_session(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        method: &str,
        user_agent: Option<&str>,
        proof: Proof,
    ) -> Result<Tokens, Error> {
        let now = now();
        let expires_at = now + self.refresh_token_ttl;
        let user_agent = user_agent
            .map(str::trim)
            .filter(|agent| !agent.is_empty())
            .map(|agent| agent.chars().take(USER_AGENT_MAX_CHARS).collect());
        let family_id = Uuid::now_v7();
        let session = SessionDetails {
            started_at: now,
            authenticated_at: now,
            user_agent,
            sign_in_method: Some(method.to_owned()),
            country: None,
        };
        let (refresh_token, row) =
            new_refresh_token(tenant, user_id, family_id, now, expires_at, session)?;
        let transaction = tenant.database().begin_write().await?;
        let still_valid = match &proof {
            Proof::Password(hash) => lock_grant(&transaction, tenant, user_id, Some(hash)).await?,
            Proof::Active => lock_grant(&transaction, tenant, user_id, None).await?,
            Proof::Challenge(id) => {
                challenge::consume(&transaction, tenant, *id).await?
                    && lock_grant(&transaction, tenant, user_id, None).await?
            }
            Proof::Passkey(id) => {
                crate::passkey::still_registered_in(&transaction, tenant, *id, user_id).await?
                    && lock_grant(&transaction, tenant, user_id, None).await?
            }
        };
        if !still_valid {
            transaction.rollback().await?;
            return Err(match proof {
                Proof::Challenge(_) => challenge::invalid_mfa_token(),
                Proof::Password(_) => crate::user::invalid_credentials(),
                Proof::Active | Proof::Passkey(_) => crate::user::user_disabled(),
            });
        }
        row.insert(&transaction).await?;
        // Lúc mở phiên gần nhất, cho danh sách người dùng (đề bài giao diện B1).
        crate::user::entity::Entity::update_many()
            .col_expr(crate::user::entity::Column::LastSignInAt, Expr::value(now))
            .filter(crate::user::entity::Column::Id.eq(user_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let event = events::SignedIn { before: None, after: Some(json!({"method": method})) };
        let subject = events::user_subject(user_id);
        self.outbox.record(&transaction, tenant, Actor::user(user_id), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, "signed in");
        let session = Stamp { id: family_id, authenticated_at: now };
        self.tokens(tenant, user_id, session, &refresh_token, expires_at, now)
    }

    /// Kiểm access token `token` (không có chữ `Bearer`): khách ghi trong token, người dùng, phiên,
    /// lần xác thực gần nhất. Chữ ký đã kiểm bằng khoá của đúng khách đó; chưa hỏi database nào.
    pub(crate) fn verify_access(&self, token: &str) -> Result<token::Verified, Error> {
        self.access_tokens.verify(token, Timestamp::now())
    }

    /// Người dùng `user_id` của một access token đã kiểm, tra ở khách `tenant` của token: không
    /// còn thì token sai; bị khoá thì `auth.user_disabled`. Cho test: request đi qua
    /// `RequestAuthenticator`, đọc người dùng cùng quyền trong một câu (D-319).
    #[cfg(test)]
    pub(crate) async fn token_user(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<User, Error> {
        let user = self.users.get(tenant, user_id).await?.ok_or_else(token::invalid_token)?;
        if !user.is_active() {
            return Err(crate::user::user_disabled());
        }
        Ok(user)
    }

    /// Người dùng của access token trong header `Authorization` (nguyên giá trị), ở khách
    /// `tenant` đã biết: token của khách khác là token sai. Cho test; request đi qua
    /// `RequestAuthenticator` (khách lấy từ token).
    #[cfg(test)]
    pub(crate) async fn authenticate(
        &self,
        tenant: &TenantContext,
        authorization: Option<&str>,
    ) -> Result<User, Error> {
        let verified = self.verify_access(bearer_token(authorization)?)?;
        if verified.tenant != tenant.id() {
            return Err(token::invalid_token());
        }
        self.token_user(tenant, verified.user).await
    }

    /// Cặp token của khách `tenant`: access token mang `tid`, refresh token (phần thô
    /// `refresh_token`) mang khách như token trong link.
    fn tokens(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        session: Stamp,
        refresh_token: &str,
        refresh_token_expires_at: Timestamp,
        now: Timestamp,
    ) -> Result<Tokens, Error> {
        let Stamp { id: session_id, authenticated_at } = session;
        let access_token =
            self.access_tokens.issue(tenant.id(), user_id, now, session_id, authenticated_at)?;
        Ok(Tokens {
            access_token,
            expires_in: self.access_tokens.ttl().as_secs(),
            refresh_token: link_token(tenant, refresh_token),
            refresh_token_expires_at,
            session_id,
        })
    }
}

/// Khoá dòng người dùng `user_id` tới hết transaction (`rows::lock_one`, D-181): refresh, thu
/// hồi phiên, khoá tài khoản hay đổi mật khẩu của cùng người dùng nối đuôi nhau thay vì chen nhau
/// (D-079) — Postgres bằng `FOR UPDATE`, SQLite vốn chỉ có một bên ghi. `true` nếu có dòng
/// (và, với `only_active`, tài khoản chưa bị khoá).
async fn lock_user_row<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    user_id: Uuid,
    only_active: bool,
) -> Result<bool, Error> {
    use crate::user::entity::{Column as User, Entity as Users};
    let mut select = Users::find_by_id(user_id).scoped(tenant);
    if only_active {
        select = select.filter(User::DisabledAt.is_null());
    }
    Ok(rows::lock_one(connection, select).await?.is_some())
}

/// Điều chứng minh người dùng khi mở phiên (xem `SessionService::start_session`).
enum Proof {
    /// Mật khẩu vừa kiểm khớp hash này.
    Password(String),
    /// Mật khẩu đúng và mã MFA đúng: lượt chờ `id` (dùng hết ngay trong transaction).
    Challenge(Uuid),
    /// Passkey `id` vừa xác minh.
    Passkey(Uuid),
    /// Người dùng vừa chứng minh xong ở bước trước trong cùng request (vd vừa đổi mật khẩu).
    Active,
}

/// Khoá dòng người dùng `user_id` trong `connection` trước khi cấp quyền mới (phiên, API key):
/// `true` nếu tài khoản còn hoạt động và — khi có `password_hash` — mật khẩu vẫn là mật khẩu
/// vừa kiểm. Khoá dòng có điều kiện (`rows::lock_one`): chạy như nhau trên SQLite và Postgres,
/// và chờ transaction đang đổi mật khẩu / khoá tài khoản xong rồi mới xét (D-156, D-181).
pub(crate) async fn lock_grant<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    user_id: Uuid,
    password_hash: Option<&str>,
) -> Result<bool, Error> {
    use crate::user::entity::{Column as User, Entity as Users};
    let mut select = Users::find_by_id(user_id).filter(User::DisabledAt.is_null()).scoped(tenant);
    if let Some(hash) = password_hash {
        select = select.filter(User::PasswordHash.eq(hash));
    }
    Ok(rows::lock_one(connection, select).await?.is_some())
}

/// Khoá dòng người dùng `user_id` (xem [`lock_user_row`]).
async fn lock_user<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<(), Error> {
    lock_user_row(connection, tenant, user_id, false).await.map(|_| ())
}

/// Khoá dòng người dùng `user_id` nếu tài khoản còn hoạt động; `false` nếu đã bị khoá
/// (hoặc không còn).
async fn lock_active_user<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<bool, Error> {
    lock_user_row(connection, tenant, user_id, true).await
}

/// Refresh token còn dùng được của người dùng `user_id`: mỗi phiên đúng một dòng.
fn live_tokens(tenant: &TenantContext, user_id: Uuid, now: Timestamp) -> sea_orm::Select<Entity> {
    Entity::find()
        .scoped(tenant)
        .filter(Column::UserId.eq(user_id))
        .filter(Column::UsedAt.is_null())
        .filter(Column::RevokedAt.is_null())
        .filter(Column::ExpiresAt.gt(now))
}

/// Thông tin của một phiên, chép sang mỗi refresh token của nó.
/// Phiên (họ refresh token) và lần xác thực gần nhất của nó, cho access token sắp cấp.
#[derive(Clone, Copy)]
struct Stamp {
    id: Uuid,
    authenticated_at: Timestamp,
}

struct SessionDetails {
    started_at: Timestamp,
    /// Lần xác thực gần nhất (đăng nhập, xác nhận lại).
    authenticated_at: Timestamp,
    user_agent: Option<String>,
    sign_in_method: Option<String>,
    country: Option<String>,
}

/// Sinh một refresh token mới và dòng database tương ứng (chưa ghi).
fn new_refresh_token(
    tenant: &TenantContext,
    user_id: Uuid,
    family_id: Uuid,
    now: Timestamp,
    expires_at: Timestamp,
    session: SessionDetails,
) -> Result<(String, entity::ActiveModel), Error> {
    let token = token::random_token(REFRESH_TOKEN_BYTES)?;
    let row = entity::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        family_id: Set(family_id),
        user_id: Set(user_id),
        token_hash: Set(token::hash_refresh_token(&token)),
        expires_at: Set(expires_at),
        used_at: Set(None),
        revoked_at: Set(None),
        created_at: Set(now),
        user_agent: Set(session.user_agent),
        session_started_at: Set(Some(session.started_at)),
        authenticated_at: Set(Some(session.authenticated_at)),
        sign_in_method: Set(session.sign_in_method),
        country: Set(session.country),
    };
    Ok((token, row))
}

/// Lấy token từ giá trị header `Authorization: Bearer <token>` (RFC 6750). Chữ
/// `Bearer` không phân biệt hoa thường.
pub(crate) fn bearer_token(authorization: Option<&str>) -> Result<&str, Error> {
    let missing = || Error::unauthenticated("auth.missing_token", "You need to sign in.");
    let value = authorization.ok_or_else(missing)?.trim();
    let (scheme, token) = value.split_once(' ').ok_or_else(missing)?;
    let token = token.trim();
    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() {
        return Err(missing());
    }
    Ok(token)
}

/// Bây giờ, cắt tới micro giây để hai database lưu cùng một giá trị.
fn now() -> Timestamp {
    Timestamp::now()
}

/// Khoá của login throttling: email của nhân sự giữ nguyên (bộ đếm cũ còn hiệu lực), của người
/// mua có tiền tố — sai mật khẩu ở cửa hàng không khoá đăng nhập của nhân sự cùng email.
/// Khoá đếm lần sai của một tài khoản đã biết (mật khẩu hiện tại, mã MFA): theo ID, không theo
/// email — không ghi email thô làm khoá, và người mua chỉ có số điện thoại (email rỗng) không dùng
/// chung một bộ đếm.
fn account_key(user_id: Uuid) -> String {
    format!("account:{user_id}")
}

fn throttle_key(realm: Realm, email: &str) -> String {
    match realm {
        Realm::Staff => email.to_owned(),
        Realm::Customer => format!("customer:{email}"),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod mfa_login_tests;

#[cfg(test)]
mod change_password_tests;

/// Thử phá header `Authorization` (D-20261123-property-fuzzing).
#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn header_bat_ky_khong_panic(raw in proptest::option::of("\\PC{0,120}")) {
            if let Ok(token) = bearer_token(raw.as_deref()) {
                prop_assert!(!token.is_empty() && token == token.trim());
            }
        }

        #[test]
        fn bearer_khong_phan_biet_hoa_thuong(
            scheme in "[bB][eE][aA][rR][eE][rR]",
            token in "[A-Za-z0-9._~+/=-]{1,80}",
        ) {
            let header = format!("{scheme} {token}");
            prop_assert_eq!(bearer_token(Some(&header)).unwrap(), token.as_str());
        }
    }
}
