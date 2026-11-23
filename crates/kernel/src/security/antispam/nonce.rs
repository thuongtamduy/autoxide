//! Phase two of anti-spam: the form token's nonce is inserted into `<module>_antispam_nonces` INSIDE the
//! module's transaction. Hitting the unique constraint means this submission was already sent (the browser was
//! clicked twice, a mobile network resent it): the module returns the earlier result instead of creating a
//! second record. This is idempotency for strangers, since `Idempotency-Key` applies only to authenticated
//! requests.

use crate::migration::prelude::{
    DbErr, Index, MigrationName, MigrationTrait, SchemaManager, Table,
};
use crate::migration::schema;
use crate::time::Timestamp;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, OnConflict, Query};

use crate::error::Error;
use crate::secret::sha256;
use crate::tenancy::TenantContext;

/// Result of inserting a nonce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// First time: a new submission.
    New,
    /// Already submitted; carries the `result` stored the first time (e.g. a record ID) to return.
    Replay { result: Option<String> },
}

pub(crate) fn table_name(module: &str) -> String {
    format!("{module}_antispam_nonces")
}

/// Whether `nonce` has already been accepted (the submission was sent before); a read, no write.
pub(crate) async fn exists<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    module: &str,
    nonce: &str,
) -> Result<bool, Error> {
    let select = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new(table_name(module)))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("nonce_hash")).eq(sha256(nonce)))
        .to_owned();
    Ok(connection.query_one(&select).await?.is_some())
}

/// Inserts `nonce` (hashed) with the `result` of this submission; kept until `expires_at`.
pub(crate) async fn claim<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    module: &str,
    nonce: &str,
    result: Option<&str>,
    expires_at: Timestamp,
) -> Result<Claim, Error> {
    let table = Alias::new(table_name(module));
    let hash = sha256(nonce);
    let id = tenant.id().as_uuid();
    let insert = Query::insert()
        .into_table(table.clone())
        .columns(["tenant_id", "nonce_hash", "result", "expires_at"].map(Alias::new))
        .values_panic([
            id.into(),
            hash.as_str().into(),
            result.map(str::to_owned).into(),
            expires_at.into(),
        ])
        .on_conflict(
            OnConflict::columns([Alias::new("tenant_id"), Alias::new("nonce_hash")])
                .do_nothing()
                .to_owned(),
        )
        .to_owned();
    if connection.execute(&insert).await?.rows_affected() == 1 {
        return Ok(Claim::New);
    }
    let select = Query::select()
        .column(Alias::new("result"))
        .from(table)
        .and_where(Expr::col(Alias::new("tenant_id")).eq(id))
        .and_where(Expr::col(Alias::new("nonce_hash")).eq(hash.as_str()))
        .to_owned();
    let result = match connection.query_one(&select).await? {
        Some(row) => row.try_get::<Option<String>>("", "result")?,
        None => None,
    };
    Ok(Claim::Replay { result })
}

/// Deletes nonces (of every tenant) that expired before `before`.
pub(crate) async fn purge<C: ConnectionTrait>(
    connection: &C,
    module: &str,
    before: Timestamp,
) -> Result<u64, Error> {
    let delete = Query::delete()
        .from_table(Alias::new(table_name(module)))
        .and_where(Expr::col(Alias::new("expires_at")).lt(before))
        .to_owned();
    Ok(connection.execute(&delete).await?.rows_affected())
}

/// Migration that creates the table `<module>_antispam_nonces`. A module that receives public submissions
/// through [`super::SpamGuard`] adds it to its migration list.
pub fn antispam_migration(name: &'static str, module: &'static str) -> Box<dyn MigrationTrait> {
    Box::new(NoncesMigration { name, module })
}

struct NoncesMigration {
    name: &'static str,
    module: &'static str,
}

impl MigrationName for NoncesMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for NoncesMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = table_name(self.module);
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(&table))
                    .col(schema::string_len("nonce_hash", 64).primary_key())
                    .col(schema::string_len_null("result", 255))
                    .col(schema::timestamp_with_time_zone("expires_at"))
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

/// The table `<module>_antispam_nonces` after the per-tenant upgrade (step 33 A4): primary key `(tenant_id,
/// nonce_hash)`; cleanup index (all tenants) `..._expires_sweep_idx`. Add it to the module's migration
/// like [`event::tenant_table`](crate::event::tenant_table).
pub fn tenant_table(module: &str) -> crate::tenancy::migration::TenantTable {
    let table = table_name(module);
    crate::tenancy::migration::TenantTable::new(&table)
        .primary_key(["tenant_id", "nonce_hash"])
        .sweep_index(format!("{table}_expires_sweep_idx"), ["expires_at"])
        .drop_index(format!("{table}_expires_idx"))
}
