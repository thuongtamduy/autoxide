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

use super::LoginThrottle;

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
}

impl SessionService {
    pub(crate) fn new(
        database: Database,
        users: Arc<UserService>,
        access_tokens: AccessTokens,
        refresh_token_ttl: std::time::Duration,
        throttle: LoginThrottle,
    ) -> Result<Self, Error> {
        let refresh_token_ttl = chrono::Duration::from_std(refresh_token_ttl)
            .map_err(|error| Error::internal(format!("refresh token lifetime: {error}")))?;
        Ok(SessionService { database, users, access_tokens, refresh_token_ttl, throttle })
    }

    /// Đăng nhập bằng email và mật khẩu: mở một họ refresh token mới.
    ///
    /// Email đang bị khoá vì sai quá nhiều lần thì trả 429 ngay, KHÔNG kiểm mật khẩu
    /// — kẻ đoán không biết được lần đoán nào đúng.
    pub(crate) async fn login(&self, email: &str, password: &str) -> Result<Tokens, Error> {
        self.throttle.check(email).await?;
        let user = match self.users.check_credentials(email, password).await {
            Ok(user) => user,
            Err(error) => {
                // Chỉ sai mật khẩu mới tính; tài khoản bị khoá hay lỗi máy chủ thì không.
                if error.code() == INVALID_CREDENTIALS {
                    self.throttle.record_failure(email).await?;
                }
                return Err(error);
            }
        };
        self.throttle.clear(email).await?;
        let now = now();
        let expires_at = now + self.refresh_token_ttl;
        let (refresh_token, row) = new_refresh_token(user.id, Uuid::now_v7(), now, expires_at)?;
        row.insert(self.database.write()).await?;
        tracing::info!(user_id = %user.id, "signed in");
        self.tokens(user.id, refresh_token, expires_at, now)
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
            self.revoke_family(row.family_id).await?;
            return Err(crate::user::user_disabled());
        }

        let expires_at = now + self.refresh_token_ttl;
        let (new_token, new_row) = new_refresh_token(user.id, row.family_id, now, expires_at)?;
        let transaction = self.database.begin_write().await?;
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
            self.revoke_family(row.family_id).await?;
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

    async fn revoke_family(&self, family_id: Uuid) -> Result<(), Error> {
        Entity::update_many()
            .col_expr(Column::RevokedAt, Expr::value(Some(now())))
            .filter(Column::FamilyId.eq(family_id))
            .filter(Column::RevokedAt.is_null())
            .exec(self.database.write())
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
        if let Err(error) = self.revoke_family(row.family_id).await {
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
fn bearer_token(authorization: Option<&str>) -> Result<&str, Error> {
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
            .create(NewUser {
                email: "ada@example.com".to_owned(),
                name: "Ada".to_owned(),
                password: Some(PASSWORD.to_owned()),
            })
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

            let tokens = sessions.login("ADA@example.com", PASSWORD).await.unwrap();

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
            sessions.login("ada@example.com", PASSWORD).await.unwrap();

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
            let tokens = sessions.login("ada@example.com", PASSWORD).await.unwrap();

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
            let first = sessions.login("ada@example.com", PASSWORD).await.unwrap();

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
            let first = sessions.login("ada@example.com", PASSWORD).await.unwrap();
            let second = sessions.refresh(&first.refresh_token).await.unwrap();
            // Một phiên khác của cùng người dùng không bị ảnh hưởng.
            let other = sessions.login("ada@example.com", PASSWORD).await.unwrap();

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
            let tokens = sessions.login("ada@example.com", PASSWORD).await.unwrap();

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
            let first = sessions.login("ada@example.com", PASSWORD).await.unwrap();
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
            let tokens = sessions.login("ada@example.com", PASSWORD).await.unwrap();
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
            let first = sessions.login("ada@example.com", PASSWORD).await.unwrap();
            let second = sessions.login("ada@example.com", PASSWORD).await.unwrap();

            users.set_password(user.id, "a brand new passphrase".to_owned()).await.unwrap();

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
            let tokens = sessions.login("ada@example.com", PASSWORD).await.unwrap();
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
