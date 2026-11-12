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

use std::sync::Arc;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use chrono::{DateTime, SubsecRound, Utc};
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
use serde_json::json;

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
    pub refresh_token_expires_at: DateTime<Utc>,
    /// Phiên mà cặp token này thuộc về (họ refresh token): client giữ lại để nhận ra
    /// phiên của chính mình trong danh sách phiên.
    pub session_id: Uuid,
}

/// Một phiên đăng nhập còn hiệu lực: một họ refresh token, nhìn từ phía người dùng.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Session {
    pub id: Uuid,
    pub user_agent: Option<String>,
    pub started_at: DateTime<Utc>,
    /// Lần cấp token gần nhất (đăng nhập hoặc refresh).
    pub last_active_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl From<entity::Model> for Session {
    fn from(row: entity::Model) -> Self {
        Session {
            id: row.family_id,
            user_agent: row.user_agent,
            started_at: row.session_started_at.unwrap_or(row.created_at),
            last_active_at: row.created_at,
            expires_at: row.expires_at,
        }
    }
}

/// Độ dài tối đa của `User-Agent` được lưu (khớp cột của m0015).
const USER_AGENT_MAX_CHARS: usize = 255;

pub(crate) struct SessionService {
    database: Database,
    users: Arc<UserService>,
    access_tokens: AccessTokens,
    refresh_token_ttl: chrono::Duration,
    throttle: LoginThrottle,
    mfa: Arc<MfaService>,
    outbox: Outbox,
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
        database: Database,
        users: Arc<UserService>,
        access_tokens: AccessTokens,
        refresh_token_ttl: std::time::Duration,
        throttle: LoginThrottle,
        mfa: Arc<MfaService>,
        outbox: Outbox,
    ) -> Result<Self, Error> {
        let refresh_token_ttl = chrono::Duration::from_std(refresh_token_ttl)
            .map_err(|error| Error::internal(format!("refresh token lifetime: {error}")))?;
        Ok(SessionService {
            database,
            users,
            access_tokens,
            refresh_token_ttl,
            throttle,
            mfa,
            outbox,
        })
    }

    /// Đăng nhập bằng email và mật khẩu. Tài khoản chưa bật MFA: mở một họ refresh
    /// token mới. Đã bật: mở một lượt chờ mã ([`LoginOutcome::MfaRequired`]); token
    /// chỉ cấp ở [`complete_mfa`](Self::complete_mfa).
    ///
    /// Email đang bị khoá vì sai quá nhiều lần thì trả 429 ngay, KHÔNG kiểm mật khẩu
    /// — kẻ đoán không biết được lần đoán nào đúng.
    /// `user_agent`: header `User-Agent` của client, để người dùng nhận ra phiên này.
    pub(crate) async fn login(
        &self,
        email: &str,
        password: &str,
        user_agent: Option<&str>,
    ) -> Result<LoginOutcome, Error> {
        // Lượt thử được đếm TRƯỚC khi kiểm mật khẩu; đúng thì xoá bộ đếm bên dưới.
        self.throttle.reserve(email).await?;
        let (user, hash) = self.users.verify_credentials(email, password).await?;
        if self.mfa.status(user.id).await?.totp_enabled {
            // Mật khẩu đúng: trả lại lượt vừa xin, nhưng chưa xoá các lần sai trước — đúng
            // mật khẩu mới là nửa đường.
            self.throttle.refund(email).await?;
            let mfa_token = challenge::open(self.database.write(), user.id, now()).await?;
            return Ok(LoginOutcome::MfaRequired {
                mfa_token,
                expires_in: challenge::CHALLENGE_TTL_SECONDS.unsigned_abs(),
            });
        }
        self.throttle.clear(email).await?;
        let proof = Proof::Password(hash);
        Ok(LoginOutcome::Tokens(self.start_session(user.id, "password", user_agent, proof).await?))
    }

    /// Người dùng `user_id` (đang đăng nhập) tự đổi mật khẩu, kèm mật khẩu hiện tại.
    ///
    /// Mỗi lần nhập sai mật khẩu hiện tại được tính vào login throttling của email như
    /// một lần đăng nhập sai: ai cầm được access token cũng không đoán mãi được mật
    /// khẩu. Đổi xong thì mọi phiên cũ kết thúc; trả cặp token của một phiên mới để
    /// client đang dùng không phải đăng nhập lại.
    pub(crate) async fn change_password(
        &self,
        user_id: Uuid,
        current: &str,
        new: String,
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
        let (user, _) = self.check_password(user_id, current).await?;
        self.users
            .write_password(
                Actor::user(user_id),
                user_id,
                "new_password",
                new,
                crate::user::PasswordWrite::Change,
            )
            .await?;
        self.throttle.clear(&user.email).await?;
        self.start_session(user_id, "password_change", user_agent, Proof::Active).await
    }

    /// Mở phiên cho người dùng `user_id` vừa chứng minh bằng passkey (đã kiểm ở
    /// `PasskeyService`). Tài khoản bị khoá thì từ chối.
    pub(crate) async fn login_with_passkey(
        &self,
        user_id: Uuid,
        passkey_id: Uuid,
        user_agent: Option<&str>,
    ) -> Result<Tokens, Error> {
        let user =
            self.users.get(user_id).await?.ok_or_else(|| crate::user::user_not_found(user_id))?;
        if !user.is_active() {
            return Err(crate::user::user_disabled());
        }
        self.start_session(user_id, "passkey", user_agent, Proof::Passkey(passkey_id)).await
    }

    /// Kiểm mật khẩu hiện tại của người dùng `user_id` trước một việc nhạy cảm (đổi mật
    /// khẩu, đổi email). Sai thì `auth.wrong_password` và tính vào login throttling.
    ///
    /// Trả kèm hash mật khẩu vừa kiểm: bên cấp quyền mới (API key) đưa nó vào
    /// [`lock_grant`] trong transaction ghi của mình.
    pub(crate) async fn check_password(
        &self,
        user_id: Uuid,
        current: &str,
    ) -> Result<(User, String), Error> {
        let user =
            self.users.get(user_id).await?.ok_or_else(|| crate::user::user_not_found(user_id))?;
        self.throttle.reserve(&user.email).await?;
        match self.users.verify_credentials(&user.email, current).await {
            Ok(verified) => {
                self.throttle.clear(&verified.0.email).await?;
                Ok(verified)
            }
            Err(error) if error.code() == INVALID_CREDENTIALS => Err(Error::bad_request(
                "auth.wrong_password",
                "The current password is incorrect. If you forgot it, reset it by email \
                     (`POST /api/v1/auth/password-reset`).",
            )),
            Err(error) => Err(error),
        }
    }

    /// Bắt đầu bật TOTP cho người dùng `user_id`, kèm mật khẩu hiện tại (sai thì tính vào login
    /// throttling như [`check_password`](Self::check_password)): access token bị lộ không đủ để
    /// gắn ứng dụng authenticator của kẻ gian vào tài khoản rồi khoá chủ thật ở ngoài (D-156).
    pub(crate) async fn start_totp(
        &self,
        user_id: Uuid,
        current_password: &str,
    ) -> Result<crate::mfa::TotpSetup, Error> {
        let (user, _) = self.check_password(user_id, current_password).await?;
        self.mfa.start_totp(user.id, &user.email).await
    }

    /// Người dùng `user_id` tự xoá tài khoản của mình, kèm mật khẩu hiện tại (sai thì
    /// tính vào login throttling như [`check_password`](Self::check_password)).
    pub(crate) async fn delete_own_account(
        &self,
        user_id: Uuid,
        current_password: &str,
    ) -> Result<(), Error> {
        self.check_password(user_id, current_password).await?;
        self.users.delete(Actor::user(user_id), user_id).await
    }

    /// Bước hai của đăng nhập: `code` là mã TOTP hoặc một recovery code. Mỗi lần sai
    /// được tính vào login throttling của email, như một lần sai mật khẩu.
    pub(crate) async fn complete_mfa(
        &self,
        mfa_token: &str,
        code: &str,
        user_agent: Option<&str>,
    ) -> Result<Tokens, Error> {
        let (challenge_id, user_id) =
            challenge::claim_attempt(self.database.write(), mfa_token, now()).await?;
        let user = self.users.get(user_id).await?.ok_or_else(challenge::invalid_mfa_token)?;
        self.throttle.reserve(&user.email).await?;
        if !self.mfa.verify_second_factor(user.id, code).await? {
            return Err(Error::unauthenticated(
                "auth.invalid_mfa_code",
                "The code is wrong, expired or already used. Enter the current code from your \
                 authenticator app, or an unused recovery code.",
            ));
        }

        if !user.is_active() {
            return Err(crate::user::user_disabled());
        }
        self.throttle.clear(&user.email).await?;
        self.start_session(user.id, "password_and_mfa", user_agent, Proof::Challenge(challenge_id))
            .await
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
        let session = SessionDetails { started_at: now, user_agent };
        let (refresh_token, row) = new_refresh_token(user_id, family_id, now, expires_at, session)?;
        let transaction = self.database.begin_write().await?;
        let still_valid = match &proof {
            Proof::Password(hash) => lock_grant(&transaction, user_id, Some(hash)).await?,
            Proof::Active => lock_grant(&transaction, user_id, None).await?,
            Proof::Challenge(id) => {
                challenge::consume(&transaction, *id).await?
                    && lock_grant(&transaction, user_id, None).await?
            }
            Proof::Passkey(id) => {
                crate::passkey::still_registered_in(&transaction, *id, user_id).await?
                    && lock_grant(&transaction, user_id, None).await?
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
        let event = events::SignedIn { before: None, after: Some(json!({"method": method})) };
        let subject = events::user_subject(user_id);
        self.outbox.record(&transaction, Actor::user(user_id), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, "signed in");
        self.tokens(user_id, family_id, refresh_token, expires_at, now)
    }

    /// Đổi refresh token lấy cặp token mới. Refresh token cũ hết dùng được.
    pub(crate) async fn refresh(&self, refresh_token: &str) -> Result<Tokens, Error> {
        let now = now();
        // Đọc trên kết nối ghi: token vừa cấp có thể chưa kịp sang read replica.
        let row = Entity::find()
            .filter(Column::TokenHash.eq(token::hash_refresh_token(refresh_token)))
            .one(self.database.write())
            .await?
            .ok_or_else(invalid_refresh_token)?;
        // Kiểm "đã dùng" TRƯỚC "đã thu hồi": token đã dùng luôn bị báo là dùng lại,
        // kể cả khi một request đồng thời khác vừa thu hồi cả họ — client nhận cùng
        // một mã cho cùng một tình huống.
        if row.used_at.is_some() {
            return Err(self.reuse_detected(&row).await);
        }
        if row.revoked_at.is_some() || row.expires_at <= now {
            return Err(invalid_refresh_token());
        }
        let user = self.users.get(row.user_id).await?.ok_or_else(invalid_refresh_token)?;
        if !user.is_active() {
            revoke_family(self.database.write(), row.family_id).await?;
            return Err(crate::user::user_disabled());
        }

        let expires_at = now + self.refresh_token_ttl;
        // Token mới mang theo thông tin của phiên.
        let session = SessionDetails {
            started_at: row.session_started_at.unwrap_or(row.created_at),
            user_agent: row.user_agent.clone(),
        };
        let (new_token, new_row) =
            new_refresh_token(user.id, row.family_id, now, expires_at, session)?;
        let transaction = self.database.begin_write().await?;
        // Chạm dòng người dùng TRƯỚC (Postgres khoá dòng này tới hết transaction): đổi mật
        // khẩu hay khoá tài khoản cũng ghi dòng này trước khi thu hồi phiên, nên hai việc
        // nối đuôi nhau — token mới không lọt qua lệnh thu hồi (D-079). Tài khoản vừa bị
        // khoá thì dừng ở đây.
        if !lock_active_user(&transaction, user.id).await? {
            transaction.rollback().await?;
            revoke_family(self.database.write(), row.family_id).await?;
            return Err(crate::user::user_disabled());
        }
        // Đánh dấu "đã dùng" CÓ ĐIỀU KIỆN: hai request cùng đổi một token thì chỉ một
        // bên cập nhật được dòng này (database khoá dòng), bên kia thấy 0 dòng.
        let claimed = Entity::update_many()
            .col_expr(Column::UsedAt, Expr::value(Some(now)))
            .filter(Column::Id.eq(row.id))
            .filter(Column::UsedAt.is_null())
            .filter(Column::RevokedAt.is_null())
            .exec(&transaction)
            .await?;
        if claimed.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(self.reuse_detected(&row).await);
        }
        new_row.insert(&transaction).await?;
        transaction.commit().await?;
        self.tokens(user.id, row.family_id, new_token, expires_at, now)
    }

    /// Đăng xuất: thu hồi họ của `refresh_token`. Token lạ hay đã thu hồi cũng coi
    /// như xong — gọi lại nhiều lần vẫn cùng kết quả.
    pub(crate) async fn logout(&self, refresh_token: &str) -> Result<(), Error> {
        let row = Entity::find()
            .filter(Column::TokenHash.eq(token::hash_refresh_token(refresh_token)))
            .one(self.database.write())
            .await?;
        if let Some(row) = row {
            let transaction = self.database.begin_write().await?;
            revoke_family(&transaction, row.family_id).await?;
            let event = events::SignedOut::default();
            let subject = events::user_subject(row.user_id);
            self.outbox.record(&transaction, Actor::user(row.user_id), subject, &event).await?;
            transaction.commit().await?;
            tracing::info!(user_id = %row.user_id, "signed out");
        }
        Ok(())
    }

    /// Người dùng của access token trong header `Authorization` (nguyên giá trị).
    pub(crate) async fn authenticate(&self, authorization: Option<&str>) -> Result<User, Error> {
        let token = bearer_token(authorization)?;
        let user_id = self.access_tokens.verify(token, Utc::now())?;
        let user = self.users.get(user_id).await?.ok_or_else(token::invalid_token)?;
        if !user.is_active() {
            return Err(crate::user::user_disabled());
        }
        Ok(user)
    }

    /// Thu hồi mọi refresh token của người dùng (vd sau khi đổi mật khẩu). Chạy
    /// trong `connection` để đi cùng transaction của bên gọi.
    /// Bỏ mọi lượt chờ mã MFA của `user_id` trong `connection`: đổi mật khẩu thì lượt mở bằng
    /// mật khẩu cũ hết dùng được.
    pub(crate) async fn drop_mfa_challenges<C: ConnectionTrait>(
        connection: &C,
        user_id: Uuid,
    ) -> Result<(), Error> {
        challenge::drop_for_user(connection, user_id).await
    }

    pub(crate) async fn revoke_user<C: ConnectionTrait>(
        connection: &C,
        user_id: Uuid,
    ) -> Result<(), Error> {
        // Chỉ token chưa dùng: token đã đổi lấy token mới thì không dùng lại được nữa, và bảng
        // giữ hàng nghìn dòng như vậy mỗi phiên (một dòng mỗi lần refresh). Gửi lại token đã dùng
        // vẫn bị phát hiện (dòng còn đó với `used_at`) và thu hồi cả họ.
        Entity::update_many()
            .col_expr(Column::RevokedAt, Expr::value(Some(now())))
            .filter(Column::UserId.eq(user_id))
            .filter(Column::UsedAt.is_null())
            .filter(Column::RevokedAt.is_null())
            .exec(connection)
            .await?;
        Ok(())
    }

    /// Một trang phiên còn hiệu lực của người dùng `user_id`, mới bắt đầu trước (mặc
    /// định). Mỗi phiên là refresh token còn dùng được của một họ.
    pub(crate) async fn list_sessions(
        &self,
        user_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<Session>, Error> {
        let select = live_tokens(user_id, now());
        // Họ token sinh bằng UUIDv7 lúc đăng nhập: xếp theo `family_id` là xếp theo lúc
        // phiên bắt đầu, và mỗi họ chỉ có một token còn sống — cursor ổn định.
        let page =
            fetch_page(select, Column::FamilyId, |row| row.family_id, page, self.database.read())
                .await?;
        Ok(page.map(Session::from))
    }

    /// Kết thúc phiên `session_id` của người dùng `user_id` (đăng xuất một thiết bị).
    /// Access token đã cấp cho phiên đó vẫn chạy tới khi hết hạn (mặc định 15 phút).
    pub(crate) async fn revoke_session(
        &self,
        user_id: Uuid,
        session_id: Uuid,
    ) -> Result<(), Error> {
        let transaction = self.database.begin_write().await?;
        // Khoá dòng người dùng trước, như refresh: refresh đang chạy dở xong hẳn rồi mới
        // tới lượt thu hồi, nên token nó vừa cấp cũng bị thu hồi.
        lock_user(&transaction, user_id).await?;
        let revoked = Entity::update_many()
            .col_expr(Column::RevokedAt, Expr::value(Some(now())))
            .filter(Column::UserId.eq(user_id))
            .filter(Column::FamilyId.eq(session_id))
            .filter(Column::RevokedAt.is_null())
            .exec(&transaction)
            .await?;
        if revoked.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(session_not_found(session_id));
        }
        let event =
            events::SignedOut { before: None, after: Some(json!({"session_id": session_id})) };
        let subject = events::user_subject(user_id);
        self.outbox.record(&transaction, Actor::user(user_id), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, "session ended");
        Ok(())
    }

    /// Kết thúc mọi phiên của người dùng `user_id`, trừ phiên `keep` (nếu có). Trả số
    /// phiên đã kết thúc. Dùng cho "đăng xuất mọi thiết bị khác" và cho quản trị viên.
    pub(crate) async fn revoke_sessions(
        &self,
        actor: Actor,
        user_id: Uuid,
        keep: Option<Uuid>,
    ) -> Result<u64, Error> {
        let now = now();
        let transaction = self.database.begin_write().await?;
        lock_user(&transaction, user_id).await?;
        let mut live = live_tokens(user_id, now);
        let mut revoke = Entity::update_many()
            .col_expr(Column::RevokedAt, Expr::value(Some(now)))
            .filter(Column::UserId.eq(user_id))
            .filter(Column::RevokedAt.is_null());
        if let Some(keep) = keep {
            live = live.filter(Column::FamilyId.ne(keep));
            revoke = revoke.filter(Column::FamilyId.ne(keep));
        }
        let ended = live.count(&transaction).await?;
        revoke.exec(&transaction).await?;
        if ended > 0 {
            let event = events::SessionsRevoked {
                before: None,
                after: Some(json!({"count": ended, "kept": keep})),
            };
            let subject = events::user_subject(user_id);
            self.outbox.record(&transaction, actor, subject, &event).await?;
        }
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, sessions = ended, "sessions ended");
        Ok(ended)
    }

    /// Thu hồi cả họ của `row` và trả lỗi báo token bị dùng lại.
    async fn reuse_detected(&self, row: &entity::Model) -> Error {
        tracing::warn!(
            user_id = %row.user_id,
            family_id = %row.family_id,
            "refresh token reused; revoking the whole session"
        );
        let ended = async {
            let transaction = self.database.begin_write().await?;
            revoke_family(&transaction, row.family_id).await?;
            let event = events::RefreshTokenReused::default();
            let subject = events::user_subject(row.user_id);
            self.outbox.record(&transaction, Actor::user(row.user_id), subject, &event).await?;
            transaction.commit().await?;
            Ok::<(), Error>(())
        }
        .await;
        if let Err(error) = ended {
            return error;
        }
        Error::unauthenticated(
            "auth.refresh_token_reused",
            "This refresh token was already used. The session has been ended for safety; sign \
             in again. Use each refresh token once and keep only the newest one.",
        )
    }

    fn tokens(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        refresh_token: String,
        refresh_token_expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<Tokens, Error> {
        Ok(Tokens {
            access_token: self.access_tokens.issue(user_id, now)?,
            expires_in: self.access_tokens.ttl().as_secs(),
            refresh_token,
            refresh_token_expires_at,
            session_id,
        })
    }
}

/// Ghi "không đổi gì" lên dòng người dùng `user_id`: trên Postgres dòng bị khoá tới hết
/// transaction, nên refresh, thu hồi phiên, khoá tài khoản hay đổi mật khẩu của cùng người
/// dùng nối đuôi nhau thay vì chen nhau (D-079). SQLite chỉ có một bên ghi tại một lúc nên
/// vốn đã nối đuôi. Trả số dòng đã chạm.
async fn touch_user<C: ConnectionTrait>(
    connection: &C,
    user_id: Uuid,
    only_active: bool,
) -> Result<u64, Error> {
    use crate::user::entity::{Column as User, Entity as Users};
    let mut update = Users::update_many()
        .col_expr(User::UpdatedAt, Expr::col(User::UpdatedAt))
        .filter(User::Id.eq(user_id));
    if only_active {
        update = update.filter(User::DisabledAt.is_null());
    }
    Ok(update.exec(connection).await?.rows_affected)
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
/// vừa kiểm. Một câu `UPDATE` có điều kiện: chạy như nhau trên SQLite và Postgres, và chờ
/// transaction đang đổi mật khẩu / khoá tài khoản xong rồi mới xét (D-156).
pub(crate) async fn lock_grant<C: ConnectionTrait>(
    connection: &C,
    user_id: Uuid,
    password_hash: Option<&str>,
) -> Result<bool, Error> {
    use crate::user::entity::{Column as User, Entity as Users};
    let mut update = Users::update_many()
        .col_expr(User::UpdatedAt, Expr::col(User::UpdatedAt))
        .filter(User::Id.eq(user_id))
        .filter(User::DisabledAt.is_null());
    if let Some(hash) = password_hash {
        update = update.filter(User::PasswordHash.eq(hash));
    }
    Ok(update.exec(connection).await?.rows_affected > 0)
}

/// Khoá dòng người dùng `user_id` (xem [`touch_user`]).
async fn lock_user<C: ConnectionTrait>(connection: &C, user_id: Uuid) -> Result<(), Error> {
    touch_user(connection, user_id, false).await.map(|_| ())
}

/// Khoá dòng người dùng `user_id` nếu tài khoản còn hoạt động; `false` nếu đã bị khoá
/// (hoặc không còn).
async fn lock_active_user<C: ConnectionTrait>(
    connection: &C,
    user_id: Uuid,
) -> Result<bool, Error> {
    Ok(touch_user(connection, user_id, true).await? > 0)
}

/// Refresh token còn dùng được của người dùng `user_id`: mỗi phiên đúng một dòng.
fn live_tokens(user_id: Uuid, now: DateTime<Utc>) -> sea_orm::Select<Entity> {
    Entity::find()
        .filter(Column::UserId.eq(user_id))
        .filter(Column::UsedAt.is_null())
        .filter(Column::RevokedAt.is_null())
        .filter(Column::ExpiresAt.gt(now))
}

fn session_not_found(id: Uuid) -> Error {
    Error::not_found(
        "auth.session_not_found",
        "You have no active session `{id}`. List your sessions with \
         `GET /api/v1/auth/me/sessions`.",
    )
    .with_param("id", id)
}

/// Thu hồi mọi refresh token của họ `family_id`.
async fn revoke_family<C: ConnectionTrait>(connection: &C, family_id: Uuid) -> Result<(), Error> {
    Entity::update_many()
        .col_expr(Column::RevokedAt, Expr::value(Some(now())))
        .filter(Column::FamilyId.eq(family_id))
        .filter(Column::RevokedAt.is_null())
        .exec(connection)
        .await?;
    Ok(())
}

/// Thông tin của một phiên, chép sang mỗi refresh token của nó.
struct SessionDetails {
    started_at: DateTime<Utc>,
    user_agent: Option<String>,
}

/// Sinh một refresh token mới và dòng database tương ứng (chưa ghi).
fn new_refresh_token(
    user_id: Uuid,
    family_id: Uuid,
    now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    session: SessionDetails,
) -> Result<(String, entity::ActiveModel), Error> {
    let token = token::random_token(REFRESH_TOKEN_BYTES)?;
    let row = entity::ActiveModel {
        id: Set(Uuid::now_v7()),
        family_id: Set(family_id),
        user_id: Set(user_id),
        token_hash: Set(token::hash_refresh_token(&token)),
        expires_at: Set(expires_at),
        used_at: Set(None),
        revoked_at: Set(None),
        created_at: Set(now),
        user_agent: Set(session.user_agent),
        session_started_at: Set(Some(session.started_at)),
    };
    Ok((token, row))
}

/// Lấy token từ giá trị header `Authorization: Bearer <token>` (RFC 6750). Chữ
/// `Bearer` không phân biệt hoa thường.
pub(crate) fn bearer_token(authorization: Option<&str>) -> Result<&str, Error> {
    let missing = || {
        Error::unauthenticated(
            "auth.missing_token",
            "This endpoint needs an access token: send `Authorization: Bearer <access token>`. \
             Get one with `POST /api/v1/auth/login`.",
        )
    };
    let value = authorization.ok_or_else(missing)?.trim();
    let (scheme, token) = value.split_once(' ').ok_or_else(missing)?;
    let token = token.trim();
    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() {
        return Err(missing());
    }
    Ok(token)
}

fn invalid_refresh_token() -> Error {
    Error::unauthenticated(
        "auth.invalid_refresh_token",
        "The refresh token is unknown, expired or revoked. Sign in again \
         (`POST /api/v1/auth/login`).",
    )
}

/// Bây giờ, cắt tới micro giây để hai database lưu cùng một giá trị.
fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user::NewUser;
    use autoxide_kernel::error::ErrorKind;
    use autoxide_testkit::TestDatabase;

    const PASSWORD: &str = "correct horse battery staple";

    /// Service đã có sẵn một người dùng `ada@example.com`.
    async fn setup(test: &TestDatabase) -> (Arc<SessionService>, Arc<UserService>, User) {
        let (sessions, users) = crate::testing::sessions(test).await;
        let user = users
            .create(
                Actor::system(),
                NewUser {
                    email: "ada@example.com".to_owned(),
                    name: "Ada".to_owned(),
                    password: Some(PASSWORD.to_owned()),
                },
            )
            .await
            .unwrap();
        (sessions, users, user)
    }

    fn bearer(tokens: &Tokens) -> String {
        format!("Bearer {}", tokens.access_token)
    }

    /// Đăng xuất mọi phiên chạy cùng lúc với refresh của các phiên đó: không token mới
    /// nào lọt qua — trên cả hai database.
    #[tokio::test]
    async fn dang_xuat_moi_phien_trong_luc_refresh() {
        TestDatabase::run(async |test| {
            let (sessions, _, user) = setup(test).await;
            let mut refresh_tokens = Vec::new();
            for _ in 0..4 {
                let tokens = sessions.login("ada@example.com", PASSWORD, None).await.unwrap();
                refresh_tokens.push(tokens.tokens().refresh_token);
            }
            sessions.throttle.clear("ada@example.com").await.unwrap();

            let mut tasks = Vec::new();
            for token in refresh_tokens {
                let sessions = sessions.clone();
                tasks.push(tokio::spawn(async move { sessions.refresh(&token).await.map(|_| ()) }));
            }
            let revoker = sessions.clone();
            let revoke = tokio::spawn(async move {
                revoker.revoke_sessions(Actor::user(user.id), user.id, None).await.map(|_| ())
            });
            for task in tasks {
                let _ = task.await.unwrap();
            }
            revoke.await.unwrap().unwrap();

            let page = PageRequest::first(10, Default::default());
            let live = sessions.list_sessions(user.id, &page).await.unwrap();
            assert!(live.data.is_empty(), "{} sessions survived", live.data.len());
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn dang_nhap_roi_dung_access_token() {
        TestDatabase::run(async |test| {
            let (sessions, _, user) = setup(test).await;

            let tokens = sessions.login("ADA@example.com", PASSWORD, None).await.unwrap().tokens();

            assert_eq!(tokens.expires_in, 900);
            assert_ne!(tokens.refresh_token, tokens.access_token);
            assert_eq!(sessions.authenticate(Some(&bearer(&tokens))).await.unwrap(), user);
            let wrong =
                sessions.login("ada@example.com", "wrong password", None).await.unwrap_err();
            assert_eq!(wrong.code(), "auth.invalid_credentials");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn doan_sai_nhieu_lan_thi_khoa_ca_mat_khau_dung() {
        TestDatabase::run(async |test| {
            let (sessions, _, _) = setup(test).await;
            // Một lần đúng giữa chừng xoá các lần sai trước.
            for _ in 0..4 {
                sessions.login("ada@example.com", "wrong password", None).await.unwrap_err();
            }
            sessions.login("ada@example.com", PASSWORD, None).await.unwrap().tokens();

            for _ in 0..5 {
                let error =
                    sessions.login("ada@example.com", "wrong password", None).await.unwrap_err();
                assert_eq!(error.code(), "auth.invalid_credentials");
            }
            let locked = sessions.login("ada@example.com", PASSWORD, None).await.unwrap_err();

            assert_eq!(locked.kind(), ErrorKind::TooManyRequests);
            assert_eq!(locked.code(), "auth.login_throttled");
            // Email không tồn tại cũng bị khoá y hệt: không lộ email nào có thật.
            for _ in 0..5 {
                sessions.login("ghost@example.com", "wrong password", None).await.unwrap_err();
            }
            let ghost =
                sessions.login("ghost@example.com", "wrong password", None).await.unwrap_err();
            assert_eq!(ghost.code(), "auth.login_throttled");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn header_authorization_sai_hoac_thieu() {
        TestDatabase::run(async |test| {
            let (sessions, _, _) = setup(test).await;
            let tokens = sessions.login("ada@example.com", PASSWORD, None).await.unwrap().tokens();

            let lower = format!("bearer {}", tokens.access_token);
            assert!(sessions.authenticate(Some(&lower)).await.is_ok());
            for (header, code) in [
                (None, "auth.missing_token"),
                (Some("Bearer"), "auth.missing_token"),
                (Some("Basic YWRhOnB3"), "auth.missing_token"),
                (Some("Bearer not.a.token"), "auth.invalid_token"),
            ] {
                let error = sessions.authenticate(header).await.unwrap_err();
                assert_eq!(error.kind(), ErrorKind::Unauthenticated);
                assert_eq!(error.code(), code, "{header:?}");
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn refresh_xoay_vong_token() {
        TestDatabase::run(async |test| {
            let (sessions, _, user) = setup(test).await;
            let first = sessions.login("ada@example.com", PASSWORD, None).await.unwrap().tokens();

            let second = sessions.refresh(&first.refresh_token).await.unwrap();
            let third = sessions.refresh(&second.refresh_token).await.unwrap();

            assert_ne!(second.refresh_token, first.refresh_token);
            assert_ne!(third.refresh_token, second.refresh_token);
            assert_eq!(sessions.authenticate(Some(&bearer(&third))).await.unwrap(), user);
            let unknown = sessions.refresh("no-such-token").await.unwrap_err();
            assert_eq!(unknown.code(), "auth.invalid_refresh_token");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn dung_lai_refresh_token_thu_hoi_ca_phien() {
        TestDatabase::run(async |test| {
            let (sessions, _, _) = setup(test).await;
            let first = sessions.login("ada@example.com", PASSWORD, None).await.unwrap().tokens();
            let second = sessions.refresh(&first.refresh_token).await.unwrap();
            // Một phiên khác của cùng người dùng không bị ảnh hưởng.
            let other = sessions.login("ada@example.com", PASSWORD, None).await.unwrap().tokens();

            let reused = sessions.refresh(&first.refresh_token).await.unwrap_err();

            assert_eq!(reused.kind(), ErrorKind::Unauthenticated);
            assert_eq!(reused.code(), "auth.refresh_token_reused");
            // Token mới nhất của họ đó cũng mất hiệu lực: kẻ trộm không giữ được phiên.
            let revoked = sessions.refresh(&second.refresh_token).await.unwrap_err();
            assert_eq!(revoked.code(), "auth.invalid_refresh_token");
            assert!(sessions.refresh(&other.refresh_token).await.is_ok());
        })
        .await
        .unwrap();
    }

    /// Nhiều request đổi CÙNG một refresh token cùng lúc: đúng một bên thắng, mọi bên
    /// khác bị coi là dùng lại, và cả họ bị thu hồi — trên cả hai database.
    #[tokio::test]
    async fn refresh_dong_thoi_chi_mot_ben_thang() {
        TestDatabase::run(async |test| {
            let (sessions, _, _) = setup(test).await;
            let tokens = sessions.login("ada@example.com", PASSWORD, None).await.unwrap().tokens();

            let attempts: Vec<_> = (0..6)
                .map(|_| {
                    let sessions = sessions.clone();
                    let token = tokens.refresh_token.clone();
                    tokio::spawn(async move { sessions.refresh(&token).await })
                })
                .collect();
            let mut winners = Vec::new();
            for attempt in attempts {
                match attempt.await.unwrap() {
                    Ok(tokens) => winners.push(tokens),
                    Err(error) => assert_eq!(error.code(), "auth.refresh_token_reused"),
                }
            }

            assert_eq!(winners.len(), 1);
            let winner = sessions.refresh(&winners[0].refresh_token).await.unwrap_err();
            assert_eq!(winner.code(), "auth.invalid_refresh_token");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn dang_xuat_thu_hoi_phien_va_goi_lai_van_on() {
        TestDatabase::run(async |test| {
            let (sessions, _, _) = setup(test).await;
            let first = sessions.login("ada@example.com", PASSWORD, None).await.unwrap().tokens();
            let second = sessions.refresh(&first.refresh_token).await.unwrap();

            sessions.logout(&first.refresh_token).await.unwrap();
            sessions.logout(&first.refresh_token).await.unwrap();
            sessions.logout("no-such-token").await.unwrap();

            let error = sessions.refresh(&second.refresh_token).await.unwrap_err();
            assert_eq!(error.code(), "auth.invalid_refresh_token");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn refresh_token_het_han_khong_dung_duoc() {
        TestDatabase::run(async |test| {
            let (sessions, _, _) = setup(test).await;
            let tokens = sessions.login("ada@example.com", PASSWORD, None).await.unwrap().tokens();
            Entity::update_many()
                .col_expr(Column::ExpiresAt, Expr::value(now()))
                .exec(sessions.database.write())
                .await
                .unwrap();

            let error = sessions.refresh(&tokens.refresh_token).await.unwrap_err();

            assert_eq!(error.code(), "auth.invalid_refresh_token");
        })
        .await
        .unwrap();
    }

    /// Kiểm mật khẩu rồi mới cấp: mật khẩu đổi xen giữa (đặt lại, khoá tài khoản) thì không cấp
    /// được — `lock_grant` so hash vừa kiểm trong transaction cấp (D-156).
    #[tokio::test]
    async fn mat_khau_doi_giua_luc_kiem_va_luc_cap_thi_khong_cap() {
        TestDatabase::run(async |test| {
            let (sessions, users, user) = setup(test).await;
            let (_, verified) = sessions.check_password(user.id, PASSWORD).await.unwrap();
            let write = sessions.database.write();
            assert!(lock_grant(write, user.id, Some(&verified)).await.unwrap());

            users
                .set_password(Actor::system(), user.id, "a brand new passphrase".to_owned())
                .await
                .unwrap();
            assert!(!lock_grant(write, user.id, Some(&verified)).await.unwrap());
            assert!(lock_grant(write, user.id, None).await.unwrap());

            let disable = crate::user::UserChanges { name: None, disabled: Some(true) };
            users.update(Actor::system(), user.id, disable, None).await.unwrap();
            assert!(!lock_grant(write, user.id, None).await.unwrap());
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn doi_mat_khau_thu_hoi_moi_phien() {
        TestDatabase::run(async |test| {
            let (sessions, users, user) = setup(test).await;
            let first = sessions.login("ada@example.com", PASSWORD, None).await.unwrap().tokens();
            let second = sessions.login("ada@example.com", PASSWORD, None).await.unwrap().tokens();

            users
                .set_password(Actor::system(), user.id, "a brand new passphrase".to_owned())
                .await
                .unwrap();

            for tokens in [first, second] {
                let error = sessions.refresh(&tokens.refresh_token).await.unwrap_err();
                assert_eq!(error.code(), "auth.invalid_refresh_token");
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tai_khoan_bi_khoa_mat_quyen() {
        TestDatabase::run(async |test| {
            let (sessions, _, user) = setup(test).await;
            let tokens = sessions.login("ada@example.com", PASSWORD, None).await.unwrap().tokens();
            crate::user::entity_for_tests::disable(sessions.database.write(), user.id).await;

            let me = sessions.authenticate(Some(&bearer(&tokens))).await.unwrap_err();
            let refresh = sessions.refresh(&tokens.refresh_token).await.unwrap_err();

            assert_eq!(me.code(), "auth.user_disabled");
            assert_eq!(refresh.code(), "auth.user_disabled");
        })
        .await
        .unwrap();
    }
}

#[cfg(test)]
mod mfa_login_tests {
    use super::*;
    use autoxide_kernel::error::ErrorKind;
    use autoxide_testkit::TestDatabase;

    use crate::mfa::service::tests::code_at;
    use crate::user::NewUser;

    const PASSWORD: &str = "correct horse battery staple";

    fn unix_now() -> u64 {
        u64::try_from(Utc::now().timestamp()).unwrap()
    }

    /// Ada đã bật MFA. Trả về service, secret TOTP và recovery code.
    async fn setup(test: &TestDatabase) -> (Arc<SessionService>, String, Vec<String>) {
        let services = crate::testing::services(test).await;
        let ada = services
            .users
            .create(
                Actor::system(),
                NewUser {
                    email: "ada@example.com".to_owned(),
                    name: "Ada".to_owned(),
                    password: Some(PASSWORD.to_owned()),
                },
            )
            .await
            .unwrap();
        let setup = services.mfa.start_totp(ada.id, &ada.email).await.unwrap();
        let codes =
            services.mfa.confirm_totp(ada.id, &code_at(&setup.secret, unix_now())).await.unwrap();
        (services.sessions, setup.secret, codes)
    }

    async fn challenge(sessions: &SessionService) -> String {
        match sessions.login("ada@example.com", PASSWORD, None).await.unwrap() {
            LoginOutcome::MfaRequired { mfa_token, expires_in } => {
                assert_eq!(expires_in, 300);
                mfa_token
            }
            LoginOutcome::Tokens(_) => panic!("expected an MFA challenge"),
        }
    }

    #[tokio::test]
    async fn dung_mat_khau_thi_con_phai_nhap_ma() {
        TestDatabase::run(async |test| {
            let (sessions, secret, _) = setup(test).await;

            let mfa_token = challenge(&sessions).await;
            let code = code_at(&secret, unix_now() + 30);
            let tokens = sessions.complete_mfa(&mfa_token, &code, None).await.unwrap();

            assert!(
                sessions
                    .authenticate(Some(&format!("Bearer {}", tokens.access_token)))
                    .await
                    .is_ok()
            );
            // Mỗi lượt chỉ cấp token một lần.
            let again = sessions.complete_mfa(&mfa_token, &code, None).await.unwrap_err();
            assert_eq!(again.code(), "auth.invalid_mfa_token");
            let unknown = sessions.complete_mfa("nope", &code, None).await.unwrap_err();
            assert_eq!(unknown.code(), "auth.invalid_mfa_token");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn dang_nhap_bang_recovery_code() {
        TestDatabase::run(async |test| {
            let (sessions, _, codes) = setup(test).await;

            let mfa_token = challenge(&sessions).await;
            sessions.complete_mfa(&mfa_token, &codes[0], None).await.unwrap();

            let mfa_token = challenge(&sessions).await;
            let reused = sessions.complete_mfa(&mfa_token, &codes[0], None).await.unwrap_err();
            assert_eq!(reused.code(), "auth.invalid_mfa_code");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn doan_ma_sai_bi_gioi_han() {
        TestDatabase::run(async |test| {
            let (sessions, _, _) = setup(test).await;
            let mfa_token = challenge(&sessions).await;

            for _ in 0..5 {
                let error = sessions.complete_mfa(&mfa_token, "000000", None).await.unwrap_err();
                assert_eq!(error.kind(), ErrorKind::Unauthenticated);
                assert_eq!(error.code(), "auth.invalid_mfa_code");
            }

            // Hết lượt nhập sai của token này…
            let used_up = sessions.complete_mfa(&mfa_token, "000000", None).await.unwrap_err();
            assert_eq!(used_up.code(), "auth.invalid_mfa_token");
            // …và các lần sai đã khoá đăng nhập của email, kể cả đúng mật khẩu.
            let locked = sessions.login("ada@example.com", PASSWORD, None).await.unwrap_err();
            assert_eq!(locked.code(), "auth.login_throttled");
        })
        .await
        .unwrap();
    }

    /// Nhiều request hoàn tất CÙNG một lượt với cùng một mã đúng: đúng một bên nhận
    /// token — trên cả hai database.
    #[tokio::test]
    async fn hoan_tat_dong_thoi_chi_mot_ben_nhan_token() {
        TestDatabase::run(async |test| {
            let (sessions, secret, _) = setup(test).await;
            let mfa_token = challenge(&sessions).await;
            let code = code_at(&secret, unix_now() + 30);

            let attempts: Vec<_> = (0..6)
                .map(|_| {
                    let sessions = sessions.clone();
                    let (mfa_token, code) = (mfa_token.clone(), code.clone());
                    tokio::spawn(
                        async move { sessions.complete_mfa(&mfa_token, &code, None).await },
                    )
                })
                .collect();
            let mut signed_in = 0;
            for attempt in attempts {
                if attempt.await.unwrap().is_ok() {
                    signed_in += 1;
                }
            }

            assert_eq!(signed_in, 1);
        })
        .await
        .unwrap();
    }
}

#[cfg(test)]
mod change_password_tests {
    use super::*;
    use crate::user::NewUser;
    use autoxide_testkit::TestDatabase;

    const OLD: &str = "correct horse battery staple";
    const NEW: &str = "a brand new passphrase";

    async fn setup(test: &TestDatabase) -> (Arc<SessionService>, User) {
        let (sessions, users) = crate::testing::sessions(test).await;
        let ada = NewUser {
            email: "ada@example.com".to_owned(),
            name: "Ada".to_owned(),
            password: Some(OLD.to_owned()),
        };
        let user = users.create(Actor::system(), ada).await.unwrap();
        (sessions, user)
    }

    #[tokio::test]
    async fn doi_mat_khau_can_mat_khau_cu_va_cap_phien_moi() {
        TestDatabase::run(async |test| {
            let (sessions, user) = setup(test).await;
            let old_session = sessions.login("ada@example.com", OLD, None).await.unwrap().tokens();

            let wrong = sessions.change_password(user.id, "not my password", NEW.to_owned(), None);
            assert_eq!(wrong.await.unwrap_err().code(), "auth.wrong_password");
            let short = sessions.change_password(user.id, OLD, "short".to_owned(), None);
            assert_eq!(short.await.unwrap_err().code(), "validation_failed");

            let tokens =
                sessions.change_password(user.id, OLD, NEW.to_owned(), None).await.unwrap();

            // Phiên cũ kết thúc; phiên mới dùng tiếp được.
            assert!(sessions.refresh(&old_session.refresh_token).await.is_err());
            sessions.refresh(&tokens.refresh_token).await.unwrap();
            assert!(sessions.login("ada@example.com", OLD, None).await.is_err());
            sessions.login("ada@example.com", NEW, None).await.unwrap().tokens();
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn doan_mat_khau_cu_nhieu_lan_bi_khoa() {
        TestDatabase::run(async |test| {
            let (sessions, user) = setup(test).await;
            for _ in 0..5 {
                let wrong = sessions.change_password(user.id, "guess", NEW.to_owned(), None).await;
                assert_eq!(wrong.unwrap_err().code(), "auth.wrong_password");
            }

            // Đã khoá: kể cả mật khẩu đúng cũng phải chờ.
            let locked = sessions.change_password(user.id, OLD, NEW.to_owned(), None).await;
            assert_eq!(locked.unwrap_err().code(), "auth.login_throttled");
        })
        .await
        .unwrap();
    }
}
