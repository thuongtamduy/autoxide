//! Ghi số liệu: cộng số liệu báo cáo của event theo giờ UTC (chỉ loại có luật dùng), ghi mẫu của số
//! liệu tóm tắt, dọn số liệu cũ.

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::report::FactSource;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};

use super::hour_of;
use crate::entity::{metric_hour, metric_seen, rule};
use crate::service::{Automations, now};

/// Giữ số liệu theo giờ bao lâu (đủ cho mức nền 4 tuần).
const KEEP_DAYS: i64 = 60;
/// Nhớ event số liệu đã cộng bao lâu (event được giao lại trong khoảng này).
const SEEN_DAYS: i64 = 7;

/// Bộ nhận event của một nguồn số liệu báo cáo.
pub(crate) struct FactHandler {
    pub source: FactSource,
}

#[async_trait::async_trait]
impl EventHandler for FactHandler {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let tenant = &event.tenant;
        // Chỉ cộng loại mà khách có luật `metric` dùng (một câu trên index của kích hoạt).
        let mut wanted = Vec::new();
        for kind in self.source.kinds() {
            let used = rule::Entity::find()
                .filter(rule::Column::TriggerType.eq("metric"))
                .filter(rule::Column::TriggerRef.eq(format!("fact:{}", kind.kind)))
                .scoped(tenant)
                .one(tenant.database().read())
                .await?
                .is_some();
            if used {
                wanted.push(kind.kind);
            }
        }
        if wanted.is_empty() {
            return Ok(());
        }
        let facts: Vec<_> = self
            .source
            .extract(event)?
            .into_iter()
            .filter(|fact| wanted.contains(&fact.kind))
            .collect();
        if facts.is_empty() {
            return Ok(());
        }
        let transaction = tenant.database().begin_write().await?;
        // Nhận lại event không cộng hai lần (D-054).
        let seen = metric_seen::Entity::insert(metric_seen::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            event_id: Set(event.id),
            created_at: Set(now()),
        })
        .on_conflict(
            OnConflict::columns([metric_seen::Column::TenantId, metric_seen::Column::EventId])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        if seen == 0 {
            transaction.rollback().await?;
            return Ok(());
        }
        for fact in facts {
            let (value, currency) = match (fact.amount, fact.quantity) {
                (Some(amount), _) => (amount.minor(), amount.currency().code().to_owned()),
                (None, Some(quantity)) => (quantity, String::new()),
                (None, None) => (0, String::new()),
            };
            let source = format!("fact:{}", fact.kind);
            add(&transaction, tenant, &source, hour_of(fact.occurred_at), &currency, value).await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

/// Cộng `value` vào giờ `hour` của `source`: tạo dòng 0 nếu chưa có, rồi cộng — chạy giống nhau trên
/// SQLite và Postgres, hai lần ghi đồng thời không mất số.
async fn add<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    source: &str,
    hour: Timestamp,
    currency: &str,
    value: i64,
) -> Result<(), Error> {
    ensure_row(connection, tenant, source, hour, currency).await?;
    metric_hour::Entity::update_many()
        .col_expr(metric_hour::Column::Value, Expr::col(metric_hour::Column::Value).add(value))
        .col_expr(metric_hour::Column::Samples, Expr::col(metric_hour::Column::Samples).add(1i64))
        .filter(metric_hour::Column::Source.eq(source))
        .filter(metric_hour::Column::Hour.eq(hour))
        .filter(metric_hour::Column::Currency.eq(currency))
        .scoped(tenant)
        .exec(connection)
        .await?;
    Ok(())
}

async fn ensure_row<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    source: &str,
    hour: Timestamp,
    currency: &str,
) -> Result<(), Error> {
    metric_hour::Entity::insert(metric_hour::ActiveModel {
        tenant_id: Set(tenant.id().as_uuid()),
        source: Set(source.to_owned()),
        hour: Set(hour),
        currency: Set(currency.to_owned()),
        value: Set(0),
        samples: Set(0),
    })
    .on_conflict(
        OnConflict::columns([
            metric_hour::Column::TenantId,
            metric_hour::Column::Source,
            metric_hour::Column::Hour,
            metric_hour::Column::Currency,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(connection)
    .await?;
    Ok(())
}

impl Automations {
    /// Ghi mẫu đọc `value` của số liệu tóm tắt `source` ở giờ `hour` (đọc lại trong cùng giờ thì
    /// thay mẫu cũ).
    pub(crate) async fn record_sample(
        &self,
        tenant: &TenantContext,
        source: &str,
        hour: Timestamp,
        currency: &str,
        value: i64,
    ) -> Result<(), Error> {
        let write = tenant.database().write();
        ensure_row(write, tenant, source, hour, currency).await?;
        metric_hour::Entity::update_many()
            .col_expr(metric_hour::Column::Value, Expr::value(value))
            .col_expr(metric_hour::Column::Samples, Expr::value(1i64))
            .filter(metric_hour::Column::Source.eq(source))
            .filter(metric_hour::Column::Hour.eq(hour))
            .filter(metric_hour::Column::Currency.eq(currency))
            .scoped(tenant)
            .exec(write)
            .await?;
        Ok(())
    }

    /// Dọn số liệu theo giờ và dấu event đã cộng quá hạn của `database`.
    pub(crate) async fn prune_metrics(&self, database: &Database) -> Result<(), Error> {
        let now = now();
        // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
        metric_hour::Entity::delete_many()
            .filter(metric_hour::Column::Hour.lt(now - SignedDuration::from_hours(24 * KEEP_DAYS)))
            .exec(database.write())
            .await?;
        // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
        metric_seen::Entity::delete_many()
            .filter(
                metric_seen::Column::CreatedAt.lt(now - SignedDuration::from_hours(24 * SEEN_DAYS)),
            )
            .exec(database.write())
            .await?;
        Ok(())
    }
}
