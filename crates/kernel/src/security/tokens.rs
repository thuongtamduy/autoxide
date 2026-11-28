//! One-time tokens (D-117): password reset, email verification, newsletter sign-up confirmation…
//!
//! - A module adds `tokens_migration("m00xx_create_tokens", "<module>")` (table `<module>_tokens`).
//! - A token is 32 random bytes (base64url, 43 characters) sent to the user; the database keeps only
//!   the SHA-256 hash — a leaked table still cannot be used as tokens (OWASP).
//! - Each token has a `purpose` (e.g. `password_reset`), a `subject` (usually the ID of an object) and
//!   optional `data` (e.g. the email address being verified).
//! - [`OneTimeTokens::consume`] is ONE statement `UPDATE … WHERE token_hash = ? AND used_at IS NULL
//!   AND expires_at > now`: two requests using the same token, exactly one wins, on both
//!   databases.
//! - Flood protection (using the API as a tool to spam other people's inboxes): the caller checks
//!   [`OneTimeTokens::recent`] before issuing a new token.
//! - Per tenant (step 33): a token carries its tenant; using a token in another tenant is like an unknown token.

use crate::migration::prelude::{
    DbErr, Index, MigrationName, MigrationTrait, SchemaManager, Table,
};
use crate::migration::schema;
use crate::time::Timestamp;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Order, Query};
use uuid::Uuid;

use crate::error::Error;
use crate::secret::{random_token, sha256};
use crate::tenancy::TenantContext;

/// A token just issued: the value sent to the user (stored nowhere) and its expiry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Issued {
    pub id: Uuid,
    pub token: String,
    pub expires_at: Timestamp,
}

/// A token that was just consumed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Consumed {
    pub id: Uuid,
    pub subject: String,
    pub data: Option<String>,
}

/// A token that is still usable (see [`OneTimeTokens::live`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Live {
    pub data: Option<String>,
    pub expires_at: Timestamp,
}

/// Why a token was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenRejection {
    Expired,
    Used,
    Unknown,
}

impl TokenRejection {
    /// The `reason` value of the Problem: `expired`, `used`, `unknown`.
    pub fn as_str(self) -> &'static str {
        match self {
            TokenRejection::Expired => "expired",
            TokenRejection::Used => "used",
            TokenRejection::Unknown => "unknown",
        }
    }
}

/// A token issued recently for an object (so the caller can decide whether to issue another).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Recent {
    pub data: Option<String>,
    pub created_at: Timestamp,
}

/// One-time tokens on the table `<module>_tokens`. Every function takes a connection (`database.write()`
/// or a transaction), so the caller can issue a token in the same transaction as its own change.
#[derive(Debug, Clone)]
pub struct OneTimeTokens {
    table: String,
}

impl OneTimeTokens {
    pub fn new(module: &str) -> Self {
        OneTimeTokens { table: table_name(module) }
    }

    /// Issues a token of tenant `tenant` for `subject`, for `purpose`, living for `ttl`.
    pub async fn issue<C: crate::db::Executor>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        purpose: &str,
        subject: &str,
        data: Option<&str>,
        ttl: impl Into<crate::time::Duration>,
    ) -> Result<Issued, Error> {
        let connection = &crate::db::Via::of(connection);
        let token = random_token(32)?;
        let id = Uuid::now_v7();
        let now = now();
        let expires_at = now + ttl.into();
        let insert = Query::insert()
            .into_table(Alias::new(&self.table))
            .columns(
                [
                    "tenant_id",
                    "id",
                    "purpose",
                    "subject",
                    "data",
                    "token_hash",
                    "expires_at",
                    "created_at",
                ]
                .map(Alias::new),
            )
            .values_panic([
                tenant.id().as_uuid().into(),
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

    /// Tokens issued for `subject` (purpose `purpose`) from `since` until now, newest first.
    pub async fn recent<C: crate::db::Executor>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        purpose: &str,
        subject: &str,
        since: Timestamp,
    ) -> Result<Vec<Recent>, Error> {
        let connection = &crate::db::Via::of(connection);
        let select = Query::select()
            .columns([Alias::new("data"), Alias::new("created_at")])
            .from(Alias::new(&self.table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
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
                    created_at: row.try_get::<Timestamp>("", "created_at")?,
                })
            })
            .collect()
    }

    /// Usable tokens (unused, not expired) of `subject` for `purpose`, newest first — e.g.
    /// an email awaiting verification.
    pub async fn live<C: crate::db::Executor>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        purpose: &str,
        subject: &str,
    ) -> Result<Vec<Live>, Error> {
        let connection = &crate::db::Via::of(connection);
        let select = Query::select()
            .columns([Alias::new("data"), Alias::new("expires_at")])
            .from(Alias::new(&self.table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("purpose")).eq(purpose))
            .and_where(Expr::col(Alias::new("subject")).eq(subject))
            .and_where(Expr::col(Alias::new("used_at")).is_null())
            .and_where(Expr::col(Alias::new("expires_at")).gt(now()))
            .order_by(Alias::new("created_at"), Order::Desc)
            .to_owned();
        let rows = connection.query_all(&select).await?;
        rows.iter()
            .map(|row| {
                Ok(Live {
                    data: row.try_get::<Option<String>>("", "data")?,
                    expires_at: row.try_get::<Timestamp>("", "expires_at")?,
                })
            })
            .collect()
    }

    /// Consumes `token` for `purpose`: if not expired and unused, marks it used and returns it; otherwise
    /// `None` (unknown token, wrong purpose, expired or already used — the caller returns one and the same
    /// error).
    pub async fn consume<C: crate::db::Executor>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        purpose: &str,
        token: &str,
    ) -> Result<Option<Consumed>, Error> {
        let connection = &crate::db::Via::of(connection);
        let hash = sha256(token);
        let now = now();
        let tenant = tenant.id().as_uuid();
        let claim = Query::update()
            .table(Alias::new(&self.table))
            .value(Alias::new("used_at"), Some(now))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant))
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
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant))
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

    /// Checks whether `token` is still usable for `purpose` WITHOUT consuming it: returns the owner and data,
    /// or `None`. For cases that must validate input before spending the token (e.g. an invitation: know the
    /// email before asking for a password) — then still call [`consume`](Self::consume) so only one side
    /// wins.
    pub async fn peek<C: crate::db::Executor>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        purpose: &str,
        token: &str,
    ) -> Result<Option<Consumed>, Error> {
        let connection = &crate::db::Via::of(connection);
        let select = Query::select()
            .columns([Alias::new("id"), Alias::new("subject"), Alias::new("data")])
            .from(Alias::new(&self.table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("token_hash")).eq(sha256(token).as_str()))
            .and_where(Expr::col(Alias::new("purpose")).eq(purpose))
            .and_where(Expr::col(Alias::new("used_at")).is_null())
            .and_where(Expr::col(Alias::new("expires_at")).gt(now()))
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

    /// Why `token` is unusable for `purpose` (after [`consume`](Self::consume) returned
    /// `None`): already used, expired or nonexistent. The holder knows their own token; nothing is revealed
    /// about other tokens. A token long expired and already purged (`purge`) is `Unknown`.
    pub async fn rejection<C: crate::db::Executor>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        purpose: &str,
        token: &str,
    ) -> Result<TokenRejection, Error> {
        let connection = &crate::db::Via::of(connection);
        let select = Query::select()
            .columns([Alias::new("used_at"), Alias::new("expires_at")])
            .from(Alias::new(&self.table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("token_hash")).eq(sha256(token).as_str()))
            .and_where(Expr::col(Alias::new("purpose")).eq(purpose))
            .to_owned();
        let Some(row) = connection.query_one(&select).await? else {
            return Ok(TokenRejection::Unknown);
        };
        if row.try_get::<Option<Timestamp>>("", "used_at")?.is_some() {
            return Ok(TokenRejection::Used);
        }
        if row.try_get::<Timestamp>("", "expires_at")? <= now() {
            return Ok(TokenRejection::Expired);
        }
        // Còn hạn, chưa dùng: vừa có bên khác dùng xong giữa hai câu, hay sai mục đích.
        Ok(TokenRejection::Used)
    }

    /// Revokes every unused token of `subject` for `purpose` (e.g. after a password
    /// reset). Returns the number of tokens revoked.
    pub async fn revoke<C: crate::db::Executor>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        purpose: &str,
        subject: &str,
    ) -> Result<u64, Error> {
        let connection = &crate::db::Via::of(connection);
        let update = Query::update()
            .table(Alias::new(&self.table))
            .value(Alias::new("used_at"), Some(now()))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("purpose")).eq(purpose))
            .and_where(Expr::col(Alias::new("subject")).eq(subject))
            .and_where(Expr::col(Alias::new("used_at")).is_null())
            .to_owned();
        Ok(connection.execute(&update).await?.rows_affected())
    }

    /// Deletes every token of `subject`, for every purpose (e.g. when deleting an account or erasing data on
    /// the data subject's request). Returns the number of rows deleted.
    pub async fn forget<C: crate::db::Executor>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        subject: &str,
    ) -> Result<u64, Error> {
        let connection = &crate::db::Via::of(connection);
        let delete = Query::delete()
            .from_table(Alias::new(&self.table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("subject")).eq(subject))
            .to_owned();
        Ok(connection.execute(&delete).await?.rows_affected())
    }

    /// Deletes tokens (all tenants) that expired before `before` (the module's cleanup job) — used tokens go
    /// away when they expire (a few minutes to a day). One condition on an indexed column instead of `OR`
    /// with `used_at` (not indexed, forcing a full table scan — D-158). Returns the number of rows deleted.
    pub async fn purge<C: crate::db::Executor>(
        &self,
        connection: &C,
        before: Timestamp,
    ) -> Result<u64, Error> {
        let connection = &crate::db::Via::of(connection);
        let delete = Query::delete()
            .from_table(Alias::new(&self.table))
            .and_where(Expr::col(Alias::new("expires_at")).lt(before))
            .to_owned();
        Ok(connection.execute(&delete).await?.rows_affected())
    }
}

fn now() -> Timestamp {
    Timestamp::now()
}

fn table_name(module: &str) -> String {
    format!("{module}_tokens")
}

/// Migration that creates the table `<module>_tokens`. A module using [`OneTimeTokens`] adds it to its list of
/// migrations.
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

/// The table `<module>_tokens` after upgrading per tenant (step 33 A4): the token is unique within a tenant;
/// the index for lookup by object starts with `tenant_id`; the cleanup index (all tenants)
/// `…_expires_sweep_idx`. Add it to the module's migrations like
/// [`event::tenant_table`](crate::event::tenant_table).
pub fn tenant_table(module: &str) -> crate::tenancy::migration::TenantTable {
    let table = table_name(module);
    crate::tenancy::migration::TenantTable::new(&table)
        .unique(format!("{table}_token_hash_unique"), ["tenant_id", "token_hash"])
        .index(format!("{table}_subject_idx"), ["tenant_id", "purpose", "subject", "created_at"])
        .sweep_index(format!("{table}_expires_sweep_idx"), ["expires_at"])
        .drop_index(format!("{table}_expires_idx"))
}
