//! Token dùng một lần (D-111): đặt lại mật khẩu, xác minh email, xác nhận đăng ký bản tin…
//!
//! - Module thêm `tokens_migration("m00xx_create_tokens", "<module>")` (bảng `<module>_tokens`).
//! - Token là 32 byte ngẫu nhiên (base64url, 43 ký tự) gửi cho người dùng; database chỉ giữ
//!   bản băm SHA-256 — lộ bảng cũng không dùng được token (OWASP).
//! - Mỗi token có `purpose` (vd `password_reset`), `subject` (thường là ID của đối tượng) và
//!   `data` tuỳ chọn (vd địa chỉ email đang xác minh).
//! - [`OneTimeTokens::consume`] là MỘT câu `UPDATE … WHERE token_hash = ? AND used_at IS NULL
//!   AND expires_at > now`: hai request cùng dùng một token thì đúng một bên thắng, trên cả hai
//!   database.
//! - Chống gửi dồn (dùng API làm công cụ spam hộp thư người khác): bên gọi xem
//!   [`OneTimeTokens::recent`] trước khi phát token mới.

use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, Order, Query};
use sea_orm_migration::prelude::{
    DbErr, Index, MigrationName, MigrationTrait, SchemaManager, Table,
};
use sea_orm_migration::schema;
use uuid::Uuid;

use crate::error::Error;
use crate::secret::{random_token, sha256};

/// Token vừa phát: giá trị gửi cho người dùng (không lưu ở đâu cả) và hạn dùng.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issued {
    pub id: Uuid,
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Token vừa được dùng.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consumed {
    pub id: Uuid,
    pub subject: String,
    pub data: Option<String>,
}

/// Token phát gần đây cho một đối tượng (để bên gọi quyết định có phát thêm không).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recent {
    pub data: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Token dùng một lần trên bảng `<module>_tokens`. Mọi hàm nhận kết nối (`database.write()`
/// hay một transaction), để bên gọi phát token cùng transaction với thay đổi của mình.
#[derive(Debug, Clone)]
pub struct OneTimeTokens {
    table: String,
}

impl OneTimeTokens {
    pub fn new(module: &str) -> Self {
        OneTimeTokens { table: table_name(module) }
    }

    /// Phát một token cho `subject`, dùng cho `purpose`, sống `ttl`.
    pub async fn issue<C: ConnectionTrait>(
        &self,
        connection: &C,
        purpose: &str,
        subject: &str,
        data: Option<&str>,
        ttl: chrono::Duration,
    ) -> Result<Issued, Error> {
        let token = random_token(32)?;
        let id = Uuid::now_v7();
        let now = now();
        let expires_at = now + ttl;
        let insert = Query::insert()
            .into_table(Alias::new(&self.table))
            .columns(
                ["id", "purpose", "subject", "data", "token_hash", "expires_at", "created_at"]
                    .map(Alias::new),
            )
            .values_panic([
                id.into(),
                purpose.into(),
                subject.into(),
                data.map(str::to_owned).into(),
                sha256(&token).into(),
                expires_at.into(),
                now.into(),
            ])
            .to_owned();
        connection.execute(&insert).await?;
        Ok(Issued { id, token, expires_at })
    }

    /// Token đã phát cho `subject` (mục đích `purpose`) từ `since` tới giờ, mới nhất trước.
    pub async fn recent<C: ConnectionTrait>(
        &self,
        connection: &C,
        purpose: &str,
        subject: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<Recent>, Error> {
        let select = Query::select()
            .columns([Alias::new("data"), Alias::new("created_at")])
            .from(Alias::new(&self.table))
            .and_where(Expr::col(Alias::new("purpose")).eq(purpose))
            .and_where(Expr::col(Alias::new("subject")).eq(subject))
            .and_where(Expr::col(Alias::new("created_at")).gt(since))
            .order_by(Alias::new("created_at"), Order::Desc)
            .to_owned();
        let rows = connection.query_all(&select).await?;
        rows.iter()
            .map(|row| {
                Ok(Recent {
                    data: row.try_get::<Option<String>>("", "data")?,
                    created_at: row.try_get::<DateTime<Utc>>("", "created_at")?,
                })
            })
            .collect()
    }

    /// Dùng `token` cho `purpose`: còn hạn, chưa dùng thì đánh dấu đã dùng và trả về; không thì
    /// `None` (token lạ, sai mục đích, hết hạn hay đã dùng — bên gọi trả cùng một lỗi).
    pub async fn consume<C: ConnectionTrait>(
        &self,
        connection: &C,
        purpose: &str,
        token: &str,
    ) -> Result<Option<Consumed>, Error> {
        let hash = sha256(token);
        let now = now();
        let claim = Query::update()
            .table(Alias::new(&self.table))
            .value(Alias::new("used_at"), Some(now))
            .and_where(Expr::col(Alias::new("token_hash")).eq(hash.as_str()))
            .and_where(Expr::col(Alias::new("purpose")).eq(purpose))
            .and_where(Expr::col(Alias::new("used_at")).is_null())
            .and_where(Expr::col(Alias::new("expires_at")).gt(now))
            .to_owned();
        if connection.execute(&claim).await?.rows_affected() != 1 {
            return Ok(None);
        }
        let select = Query::select()
            .columns([Alias::new("id"), Alias::new("subject"), Alias::new("data")])
            .from(Alias::new(&self.table))
            .and_where(Expr::col(Alias::new("token_hash")).eq(hash.as_str()))
            .to_owned();
        let Some(row) = connection.query_one(&select).await? else {
            return Ok(None);
        };
        Ok(Some(Consumed {
            id: row.try_get::<Uuid>("", "id")?,
            subject: row.try_get::<String>("", "subject")?,
            data: row.try_get::<Option<String>>("", "data")?,
        }))
    }

    /// Mọi token chưa dùng của `subject` cho `purpose` hết hiệu lực (vd sau khi đặt lại mật
    /// khẩu). Trả số token bị thu hồi.
    pub async fn revoke<C: ConnectionTrait>(
        &self,
        connection: &C,
        purpose: &str,
        subject: &str,
    ) -> Result<u64, Error> {
        let update = Query::update()
            .table(Alias::new(&self.table))
            .value(Alias::new("used_at"), Some(now()))
            .and_where(Expr::col(Alias::new("purpose")).eq(purpose))
            .and_where(Expr::col(Alias::new("subject")).eq(subject))
            .and_where(Expr::col(Alias::new("used_at")).is_null())
            .to_owned();
        Ok(connection.execute(&update).await?.rows_affected())
    }

    /// Xoá token đã hết hạn hay đã dùng trước `before` (job dọn dẹp của module). Trả số dòng
    /// đã xoá.
    pub async fn purge<C: ConnectionTrait>(
        &self,
        connection: &C,
        before: DateTime<Utc>,
    ) -> Result<u64, Error> {
        let delete = Query::delete()
            .from_table(Alias::new(&self.table))
            .cond_where(
                Condition::any()
                    .add(Expr::col(Alias::new("expires_at")).lt(before))
                    .add(Expr::col(Alias::new("used_at")).lt(before)),
            )
            .to_owned();
        Ok(connection.execute(&delete).await?.rows_affected())
    }
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

fn table_name(module: &str) -> String {
    format!("{module}_tokens")
}

/// Migration tạo bảng `<module>_tokens`. Module dùng [`OneTimeTokens`] thêm nó vào danh sách
/// migration của mình.
pub fn tokens_migration(name: &'static str, module: &'static str) -> Box<dyn MigrationTrait> {
    Box::new(TokensMigration { name, module })
}

struct TokensMigration {
    name: &'static str,
    module: &'static str,
}

impl MigrationName for TokensMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for TokensMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = table_name(self.module);
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(&table))
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("purpose", 64))
                    .col(schema::string_len("subject", 255))
                    .col(schema::text_null("data"))
                    // SHA-256 (base64url) của token; token thật không được lưu.
                    .col(schema::string_len("token_hash", 64).unique_key())
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone_null("used_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name(format!("{table}_subject_idx"))
                    .table(Alias::new(&table))
                    .col("purpose")
                    .col("subject")
                    .col("created_at")
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name(format!("{table}_expires_idx"))
                    .table(Alias::new(&table))
                    .col("expires_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new(table_name(self.module))).to_owned())
            .await
    }
}
