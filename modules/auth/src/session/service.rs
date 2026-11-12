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
use autoxide_kernel::error::Error;
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
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
}

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
    pub(crate) async fn login(&self, email: &str, password: &str) -> Result<LoginOutcome, Error> {
        // Lượt thử được đếm TRƯỚC khi kiểm mật khẩu; đúng thì xoá bộ đếm bên dưới.
        self.throttle.reserve(email).await?;
        let user = self.users.check_credentials(email, password).await?;
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
        Ok(LoginOutcome::Tokens(self.start_session(user.id, "password").await?))
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
    ) -> Result<Tokens, Error> {
        if let Some(problem) = crate::password::check_rules("new_password", &new) {
            return Err(Error::validation(vec![problem]));
        }
        let user = self.check_password(user_id, current).await?;
        self.users
            .write_password(Actor::user(user_id), user_id, "new_password", new, false)
            .await?;
        self.throttle.clear(&user.email).await?;
        self.start_session(user_id, "password_change").await
    }

    /// Mở phiên cho người dùng `user_id` vừa chứng minh bằng passkey (đã kiểm ở
    /// `PasskeyService`). Tài khoản bị khoá thì từ chối.
    pub(crate) async fn login_with_passkey(&self, user_id: Uuid) -> Result<Tokens, Error> {
        let user =
            self.users.get(user_id).await?.ok_or_else(|| crate::user::user_not_found(user_id))?;
        if !user.is_active() {
            return Err(crate::user::user_disabled());
        }
        self.start_session(user_id, "passkey").await
    }

    /// Kiểm mật khẩu hiện tại của người dùng `user_id` trước một việc nhạy cảm (đổi mật
    /// khẩu, đổi email). Sai thì `auth.wrong_password` và tính vào login throttling.
    pub(crate) async fn check_password(&self, user_id: Uuid, current: &str) -> Result<User, Error> {
        let user =
            self.users.get(user_id).await?.ok_or_else(|| crate::user::user_not_found(user_id))?;
        self.throttle.reserve(&user.email).await?;
        match self.users.check_credentials(&user.email, current).await {
            Ok(user) => {
                self.throttle.clear(&user.email).await?;
                Ok(user)
            }
            Err(error) if error.code() == INVALID_CREDENTIALS => Err(Error::bad_request(
                "auth.wrong_password",
                "The current password is incorrect. If you forgot it, reset it by email \
                     (`POST /api/v1/auth/password-reset`).",
            )),
            Err(error) => Err(error),
        }
    }

    /// Bước hai của đăng nhập: `code` là mã TOTP hoặc một recovery code. Mỗi lần sai
    /// được tính vào login throttling của email, như một lần sai mật khẩu.
    pub(crate) async fn complete_mfa(&self, mfa_token: &str, code: &str) -> Result<Tokens, Error> {
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
        if !challenge::consume(self.database.write(), challenge_id).await? {
            return Err(challenge::invalid_mfa_token());
        }
        if !user.is_active() {
            return Err(crate::user::user_disabled());
        }
        self.throttle.clear(&user.email).await?;
        self.start_session(user.id, "password_and_mfa").await
    }

    /// Mở một họ refresh token mới và cấp cặp token đầu tiên.
    /// `method`: cách đã xác thực (`password`, `password_and_mfa`), ghi vào audit.
    async fn start_session(&self, user_id: Uuid, method: &str) -> Result<Tokens, Error> {
        let now = now();
        let expires_at = now + self.refresh_token_ttl;
        let (refresh_token, row) = new_refresh_token(user_id, Uuid::now_v7(), now, expires_at)?;
        let transaction = self.database.begin_write().await?;
        row.insert(&transaction).await?;
        let event = events::SignedIn { before: None, after: Some(json!({"method": method})) };
        let subject = events::user_subject(user_id);
        self.outbox.record(&transaction, Actor::user(user_id), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, "signed in");
        self.tokens(user_id, refresh_token, expires_at, now)
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
        let (new_token, new_row) = new_refresh_token(user.id, row.family_id, now, expires_at)?;
        let transaction = self.database.begin_write().await?;
        // Chạm dòng người dùng TRƯỚC (Postgres khoá dòng này tới hết transaction): đổi mật
        // khẩu hay khoá tài khoản cũng ghi dòng này trước khi thu hồi phiên, nên hai việc
        // nối đuôi nhau — token mới không lọt qua lệnh thu hồi (D-079). Tài khoản vừa bị
        // khoá thì dừng ở đây.
        let active = crate::user::entity::Entity::update_many()
            .col_expr(
                crate::user::entity::Column::UpdatedAt,
                Expr::col(crate::user::entity::Column::UpdatedAt),
            )
            .filter(crate::user::entity::Column::Id.eq(user.id))
            .filter(crate::user::entity::Column::DisabledAt.is_null())
            .exec(&transaction)
            .await?;
        if active.rows_affected == 0 {
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
        self.tokens(user.id, new_token, expires_at, now)
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
    pub(crate) async fn revoke_user<C: ConnectionTrait>(
        connection: &C,
        user_id: Uuid,
    ) -> Result<(), Error> {
        Entity::update_many()
            .col_expr(Column::RevokedAt, Expr::value(Some(now())))
            .filter(Column::UserId.eq(user_id))
            .filter(Column::RevokedAt.is_null())
            .exec(connection)
            .await?;
        Ok(())
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
        refresh_token: String,
        refresh_token_expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<Tokens, Error> {
        Ok(Tokens {
            access_token: self.access_tokens.issue(user_id, now)?,
            expires_in: self.access_tokens.ttl().as_secs(),
            refresh_token,
            refresh_token_expires_at,
        })
    }
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

/// Sinh một refresh token mới và dòng database tương ứng (chưa ghi).
fn new_refresh_token(
    user_id: Uuid,
    family_id: Uuid,
    now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
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

    #[tokio::test]
    async fn dang_nhap_roi_dung_access_token() {
        TestDatabase::run(async |test| {
            let (sessions, _, user) = setup(test).await;

            let tokens = sessions.login("ADA@example.com", PASSWORD).await.unwrap().tokens();

            assert_eq!(tokens.expires_in, 900);
            assert_ne!(tokens.refresh_token, tokens.access_token);
            assert_eq!(sessions.authenticate(Some(&bearer(&tokens))).await.unwrap(), user);
            let wrong = sessions.login("ada@example.com", "wrong password").await.unwrap_err();
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
                sessions.login("ada@example.com", "wrong password").await.unwrap_err();
            }
            sessions.login("ada@example.com", PASSWORD).await.unwrap().tokens();

            for _ in 0..5 {
                let error = sessions.login("ada@example.com", "wrong password").await.unwrap_err();
                assert_eq!(error.code(), "auth.invalid_credentials");
            }
            let locked = sessions.login("ada@example.com", PASSWORD).await.unwrap_err();

            assert_eq!(locked.kind(), ErrorKind::TooManyRequests);
            assert_eq!(locked.code(), "auth.login_throttled");
            // Email không tồn tại cũng bị khoá y hệt: không lộ email nào có thật.
            for _ in 0..5 {
                sessions.login("ghost@example.com", "wrong password").await.unwrap_err();
            }
            let ghost = sessions.login("ghost@example.com", "wrong password").await.unwrap_err();
            assert_eq!(ghost.code(), "auth.login_throttled");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn header_authorization_sai_hoac_thieu() {
        TestDatabase::run(async |test| {
            let (sessions, _, _) = setup(test).await;
            let tokens = sessions.login("ada@example.com", PASSWORD).await.unwrap().tokens();

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
            let first = sessions.login("ada@example.com", PASSWORD).await.unwrap().tokens();

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
            let first = sessions.login("ada@example.com", PASSWORD).await.unwrap().tokens();
            let second = sessions.refresh(&first.refresh_token).await.unwrap();
            // Một phiên khác của cùng người dùng không bị ảnh hưởng.
            let other = sessions.login("ada@example.com", PASSWORD).await.unwrap().tokens();

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
            let tokens = sessions.login("ada@example.com", PASSWORD).await.unwrap().tokens();

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
            let first = sessions.login("ada@example.com", PASSWORD).await.unwrap().tokens();
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
            let tokens = sessions.login("ada@example.com", PASSWORD).await.unwrap().tokens();
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

    #[tokio::test]
    async fn doi_mat_khau_thu_hoi_moi_phien() {
        TestDatabase::run(async |test| {
            let (sessions, users, user) = setup(test).await;
            let first = sessions.login("ada@example.com", PASSWORD).await.unwrap().tokens();
            let second = sessions.login("ada@example.com", PASSWORD).await.unwrap().tokens();

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
            let tokens = sessions.login("ada@example.com", PASSWORD).await.unwrap().tokens();
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
        match sessions.login("ada@example.com", PASSWORD).await.unwrap() {
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
            let tokens = sessions.complete_mfa(&mfa_token, &code).await.unwrap();

            assert!(
                sessions
                    .authenticate(Some(&format!("Bearer {}", tokens.access_token)))
                    .await
                    .is_ok()
            );
            // Mỗi lượt chỉ cấp token một lần.
            let again = sessions.complete_mfa(&mfa_token, &code).await.unwrap_err();
            assert_eq!(again.code(), "auth.invalid_mfa_token");
            let unknown = sessions.complete_mfa("nope", &code).await.unwrap_err();
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
            sessions.complete_mfa(&mfa_token, &codes[0]).await.unwrap();

            let mfa_token = challenge(&sessions).await;
            let reused = sessions.complete_mfa(&mfa_token, &codes[0]).await.unwrap_err();
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
                let error = sessions.complete_mfa(&mfa_token, "000000").await.unwrap_err();
                assert_eq!(error.kind(), ErrorKind::Unauthenticated);
                assert_eq!(error.code(), "auth.invalid_mfa_code");
            }

            // Hết lượt nhập sai của token này…
            let used_up = sessions.complete_mfa(&mfa_token, "000000").await.unwrap_err();
            assert_eq!(used_up.code(), "auth.invalid_mfa_token");
            // …và các lần sai đã khoá đăng nhập của email, kể cả đúng mật khẩu.
            let locked = sessions.login("ada@example.com", PASSWORD).await.unwrap_err();
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
                    tokio::spawn(async move { sessions.complete_mfa(&mfa_token, &code).await })
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
            let old_session = sessions.login("ada@example.com", OLD).await.unwrap().tokens();

            let wrong = sessions.change_password(user.id, "not my password", NEW.to_owned());
            assert_eq!(wrong.await.unwrap_err().code(), "auth.wrong_password");
            let short = sessions.change_password(user.id, OLD, "short".to_owned());
            assert_eq!(short.await.unwrap_err().code(), "validation_failed");

            let tokens = sessions.change_password(user.id, OLD, NEW.to_owned()).await.unwrap();

            // Phiên cũ kết thúc; phiên mới dùng tiếp được.
            assert!(sessions.refresh(&old_session.refresh_token).await.is_err());
            sessions.refresh(&tokens.refresh_token).await.unwrap();
            assert!(sessions.login("ada@example.com", OLD).await.is_err());
            sessions.login("ada@example.com", NEW).await.unwrap().tokens();
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn doan_mat_khau_cu_nhieu_lan_bi_khoa() {
        TestDatabase::run(async |test| {
            let (sessions, user) = setup(test).await;
            for _ in 0..5 {
                let wrong = sessions.change_password(user.id, "guess", NEW.to_owned()).await;
                assert_eq!(wrong.unwrap_err().code(), "auth.wrong_password");
            }

            // Đã khoá: kể cả mật khẩu đúng cũng phải chờ.
            let locked = sessions.change_password(user.id, OLD, NEW.to_owned()).await;
            assert_eq!(locked.unwrap_err().code(), "auth.login_throttled");
        })
        .await
        .unwrap();
    }
}
