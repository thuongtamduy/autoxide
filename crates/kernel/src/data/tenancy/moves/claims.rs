//! Chờ job / event của một khách đang bị giữ ở database nguồn chạy xong — dùng chung cho dời khách
//! giữa placement và tách / gộp khách (bước 38).

use std::time::Duration;

use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};

use super::now;
use crate::database::Database;
use crate::error::Error;
use crate::tenancy::{TenantId, errors};
use crate::transfer::{TableKind, TenantTables};

/// Hỏi lại job / event còn bị giữ ở nguồn sau chừng này.
const CLAIM_POLL: Duration = Duration::from_millis(500);
/// Chờ job / event ở nguồn hết bị giữ tối đa chừng này (job giữ tối đa 5 phút).
const CLAIM_WAIT: Duration = Duration::from_secs(330);

/// Chờ tới khi không job hay event nào của khách `tenant` ở `source` còn bị giữ (worker hay bộ phát
/// đang chạy nó). Worker và bộ phát không nhận việc mới của khách `moving`, nên chỉ phải chờ phần đã
/// nhận trước đó — tối đa thời hạn giữ của job.
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
