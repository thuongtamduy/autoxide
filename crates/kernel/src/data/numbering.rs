//! Gap-free sequential numbering (step 36): document numbers, invoice numbers, voucher numbers and so on
//! must be continuous 1, 2, 3... per sequence, never duplicated and never skipped, even when many requests
//! take numbers at once.
//!
//! A sequence is one row of the `kernel_sequences` table, keyed by tenant (`tenant_id`) and sequence name
//! (`scope`, chosen by the module, e.g. `invoices:<org>:<kind>:<symbol>:<year>`). The table is a
//! tenant-level table of the internal `kernel` module, so it exists in every data database: a number is
//! issued **in the same transaction** as the record that carries it.
//!
//! Why there are no gaps: [`next`] is ONE statement,
//! `INSERT … ON CONFLICT (tenant_id, scope) DO UPDATE SET last_value = last_value + 1 RETURNING last_value`,
//! run inside the caller's write transaction. Postgres locks the sequence row until the transaction ends, so
//! a second issuer waits for the first to finish before adding; SQLite has a single writer anyway
//! (`BEGIN IMMEDIATE`). If the transaction rolls back, the increment rolls back too: the number is not
//! consumed. (Unlike a Postgres `SEQUENCE`, which is faster but skips numbers on rollback, and SQLite has
//! none.)
//!
//! The price: issuers on the same sequence queue up until commit, so keep the transaction short and issue the
//! number last, right before writing the record.
use crate::time::Timestamp;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, OnConflict, Query};

use crate::error::Error;
use crate::tenancy::TenantId;

/// Table name (tenant-level, `kernel` module).
pub(crate) const TABLE: &str = "kernel_sequences";

/// Maximum length of a sequence name.
pub const MAX_SCOPE_LEN: usize = 200;

/// Issues the next number of sequence `scope` of tenant `tenant`: a new sequence starts at 1. `transaction`
/// must be the WRITE transaction (`Database::begin_write`) on the tenant's database, the same transaction
/// as the record that carries the number; outside it the number is still in order, but a later failure of
/// the record leaves a skipped number.
pub async fn next<C: crate::db::Executor>(
    transaction: &C,
    tenant: TenantId,
    scope: &str,
) -> Result<i64, Error> {
    let transaction = &crate::db::Via::of(transaction);
    check_scope(scope)?;
    let table = Alias::new(TABLE);
    let now = Timestamp::now();
    let upsert = Query::insert()
        .into_table(table.clone())
        .columns(["tenant_id", "scope", "last_value", "updated_at"].map(Alias::new))
        .values_panic([tenant.as_uuid().into(), scope.into(), 1i64.into(), now.into()])
        .on_conflict(
            OnConflict::columns([Alias::new("tenant_id"), Alias::new("scope")])
                // Ghi kèm tên bảng: trong `DO UPDATE` tên cột trần lẫn với dòng `excluded`.
                .value(
                    Alias::new("last_value"),
                    Expr::col((table.clone(), Alias::new("last_value"))).add(1i64),
                )
                .value(Alias::new("updated_at"), now)
                .to_owned(),
        )
        .returning_col(Alias::new("last_value"))
        .to_owned();
    let row = transaction
        .query_one(&upsert)
        .await?
        .ok_or_else(|| Error::internal(format!("numbering `{scope}` returned no row")))?;
    Ok(row.try_get::<i64>("", "last_value")?)
}

/// The last number issued in the sequence (0 when none has been issued). For viewing only: to issue a
/// number call [`next`], do not read then add one.
pub async fn current<C: crate::db::Executor>(
    connection: &C,
    tenant: TenantId,
    scope: &str,
) -> Result<i64, Error> {
    let connection = &crate::db::Via::of(connection);
    check_scope(scope)?;
    let select = Query::select()
        .column(Alias::new("last_value"))
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.as_uuid()))
        .and_where(Expr::col(Alias::new("scope")).eq(scope))
        .to_owned();
    match connection.query_one(&select).await? {
        Some(row) => Ok(row.try_get::<i64>("", "last_value")?),
        None => Ok(0),
    }
}

fn check_scope(scope: &str) -> Result<(), Error> {
    if scope.is_empty() || scope.len() > MAX_SCOPE_LEN {
        // Lỗi của lập trình viên module (tên dãy do code đặt), không phải của người gọi API.
        return Err(Error::internal(format!(
            "a numbering scope must be 1 to {MAX_SCOPE_LEN} bytes, got {}",
            scope.len()
        )));
    }
    Ok(())
}

/// Migration that creates the `kernel_sequences` table.
pub(crate) struct CreateSequences;

mod migration {
    use crate::migration::prelude::*;

    impl MigrationName for super::CreateSequences {
        fn name(&self) -> &str {
            "m0016_create_sequences"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for super::CreateSequences {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create()
                        .table(super::TABLE)
                        .col(schema::uuid("tenant_id"))
                        .col(schema::string_len("scope", 200))
                        .col(schema::big_integer("last_value"))
                        .col(schema::timestamp_with_time_zone("updated_at"))
                        .primary_key(Index::create().col("tenant_id").col("scope"))
                        .to_owned(),
                )
                .await
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager.drop_table(Table::drop().table(super::TABLE).to_owned()).await
        }
    }
}
