//! Waits for the jobs / events of a tenant held at the source database to finish; shared by moving a tenant
//! between placements and by splitting / merging tenants (step 38).

use std::time::Duration;

use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};

use super::now;
use crate::database::Database;
use crate::error::Error;
use crate::tenancy::{TenantId, errors};
use crate::transfer::{TableKind, TenantTables};

/// Re-checks whether jobs / events are still held at the source after this long.
const CLAIM_POLL: Duration = Duration::from_millis(500);
/// Waits for jobs / events at the source to be released for at most this long (a job is held for at most 5 minutes).
const CLAIM_WAIT: Duration = Duration::from_secs(330);

/// Waits until no job or event of tenant `tenant` at `source` is still held (by a worker or an event
/// dispatcher running it). Workers and dispatchers accept no new work of a `moving` tenant, so only the part
/// accepted earlier must be awaited, at most the job hold period.
pub(crate) async fn wait_claims(
    source: &Database,
    tables: &TenantTables,
    tenant: TenantId,
) -> Result<(), Error> {
    let started = std::time::Instant::now();
    loop {
        let at = now();
        let mut busy = 0u64;
        for (table, kind) in tables.queues() {
            let finished = match kind {
                TableKind::Jobs => "completed_at",
                _ => "delivered_at",
            };
            let select = Query::select()
                .expr_as(Expr::cust("COUNT(*)"), Alias::new("busy"))
                .from(Alias::new(table))
                .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.as_uuid()))
                .and_where(Expr::col(Alias::new("claimed_until")).gt(at))
                .and_where(Expr::col(Alias::new(finished)).is_null())
                .and_where(Expr::col(Alias::new("failed_at")).is_null())
                .to_owned();
            let row = source.write().query_one(&select).await?;
            let count: i64 = match row {
                Some(row) => row.try_get("", "busy")?,
                None => 0,
            };
            busy += <u64 as TryFrom<i64>>::try_from(count).unwrap_or(0);
        }
        if busy == 0 {
            return Ok(());
        }
        if started.elapsed() > CLAIM_WAIT {
            return Err(errors::tenant_move_busy(busy));
        }
        tokio::time::sleep(CLAIM_POLL).await;
    }
}
