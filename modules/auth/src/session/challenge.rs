//! Lượt đăng nhập chờ lớp thứ hai (MFA): đã đúng mật khẩu, chưa nhập mã.
//!
//! Client nhận một `mfa_token` ngẫu nhiên (database giữ bản băm), sống
//! [`CHALLENGE_TTL_SECONDS`], dùng một lần, cho sai tối đa [`MAX_ATTEMPTS`] lần.

use autoxide_kernel::error::Error;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use self::entity::{Column, Entity};
use crate::token;

/// Thời gian chờ nhập mã.
pub(crate) const CHALLENGE_TTL_SECONDS: i64 = 300;
/// Số lần nhập sai cho một lượt; hết thì phải nhập lại mật khẩu.
const MAX_ATTEMPTS: i32 = 5;

/// Entity của bảng `auth_mfa_challenge` (SeaORM).
mod entity {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_mfa_challenge")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub user_id: Uuid,
        #[sea_orm(unique)]
        pub token_hash: String,
        pub attempts: i32,
        pub expires_at: DateTimeUtc,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// Mở một lượt cho `user_id`; trả về `mfa_token` (chỉ lần này).
pub(crate) async fn open<C: ConnectionTrait>(
    connection: &C,
    user_id: Uuid,
    now: DateTime<Utc>,
) -> Result<String, Error> {
    let mfa_token = token::random_token(32)?;
    entity::ActiveModel {
        id: Set(Uuid::now_v7()),
        user_id: Set(user_id),
        token_hash: Set(token::sha256(&mfa_token)),
        attempts: Set(0),
        expires_at: Set(now + chrono::Duration::seconds(CHALLENGE_TTL_SECONDS)),
        created_at: Set(now),
    }
    .insert(connection)
    .await?;
    Ok(mfa_token)
}

/// Lượt còn hiệu lực của `mfa_token`: (ID lượt, người dùng).
pub(crate) async fn find<C: ConnectionTrait>(
    connection: &C,
    mfa_token: &str,
    now: DateTime<Utc>,
) -> Result<(Uuid, Uuid), Error> {
    let row = Entity::find()
        .filter(Column::TokenHash.eq(token::sha256(mfa_token)))
        .one(connection)
        .await?
        .filter(|row| row.expires_at > now && row.attempts < MAX_ATTEMPTS)
        .ok_or_else(invalid_mfa_token)?;
    Ok((row.id, row.user_id))
}

/// Ghi một lần nhập sai.
pub(crate) async fn record_attempt<C: ConnectionTrait>(
    connection: &C,
    id: Uuid,
) -> Result<(), Error> {
    Entity::update_many()
        .col_expr(Column::Attempts, Expr::col(Column::Attempts).add(1))
        .filter(Column::Id.eq(id))
        .exec(connection)
        .await?;
    Ok(())
}

/// Dùng hết lượt (đã nhập đúng). `false` nếu request khác vừa dùng nó: mỗi lượt cấp
/// token đúng một lần.
pub(crate) async fn consume<C: ConnectionTrait>(connection: &C, id: Uuid) -> Result<bool, Error> {
    let deleted = Entity::delete_many().filter(Column::Id.eq(id)).exec(connection).await?;
    Ok(deleted.rows_affected == 1)
}

pub(crate) fn invalid_mfa_token() -> Error {
    Error::unauthenticated(
        "auth.invalid_mfa_token",
        "This sign-in attempt is unknown, expired or used up. Sign in again with your email \
         and password (`POST /api/v1/auth/login`).",
    )
}
