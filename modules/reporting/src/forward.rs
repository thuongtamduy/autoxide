//! Chặng 2 — đẩy số liệu lên kho của cây (lịch `reporting.forward`, chạy trên TỪNG database) và
//! dọn kho cục bộ (lịch `reporting.prune_local`).
//!
//! Không có transaction xuyên hai database, nên mỗi lô đi theo khuôn "ghi đích idempotent rồi đánh
//! dấu nguồn": một transaction ở database của khách gốc chèn số liệu (trùng thì bỏ qua) và chỉ cộng
//! vào `reporting_daily` dòng thật sự được chèn; commit xong mới đặt `forwarded_at` ở database
//! nguồn. Chết giữa hai bước → lần sau đẩy lại, unique chặn đếm hai lần.
//!
//! Đẩy theo cờ `forwarded_at IS NULL`, không theo "id lớn hơn lần trước": id UUIDv7 sinh lúc ghi,
//! dòng id nhỏ có thể commit muộn hơn dòng id lớn.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::tenancy::{TenantContext, TenantId, ensure_serving};
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use uuid::Uuid;

use crate::entity::{daily, dimension, fact, local_fact};
use crate::service::Reporting;

/// Loại job (và tên lịch) đẩy số liệu.
pub(crate) const FORWARD: &str = "reporting.forward";
/// Loại job (và tên lịch) dọn số liệu cục bộ đã đẩy.
pub(crate) const PRUNE_LOCAL: &str = "reporting.prune_local";

pub(crate) struct ForwardJob(pub Arc<Reporting>);

#[async_trait::async_trait]
impl JobHandler for ForwardJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.forward_database(&job.database).await.map(|_| ())
    }
}

pub(crate) struct PruneLocalJob(pub Arc<Reporting>);

#[async_trait::async_trait]
impl JobHandler for PruneLocalJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.prune_local(&job.database).await.map(|_| ())
    }
}

impl Reporting {
    /// Đẩy mọi số liệu chưa đẩy của `database` lên kho của cây tương ứng; trả số dòng đã đẩy.
    /// Khách cùng gốc đi cùng nhau, gốc nối tiếp nhau (không mở nhiều database cùng lúc). Gốc đang
    /// bảo trì: số liệu của cây đó chờ lần sau, các cây khác vẫn đẩy.
    pub(crate) async fn forward_database(&self, database: &Database) -> Result<u64, Error> {
        let tenants: Vec<Uuid> = local_fact::Entity::find()
            .select_only()
            .column(local_fact::Column::TenantId)
            .filter(local_fact::Column::ForwardedAt.is_null())
            .distinct()
            .into_tuple()
            .all(database.read())
            .await?;
        let mut by_root: BTreeMap<TenantId, Vec<TenantId>> = BTreeMap::new();
        for tenant in tenants.into_iter().map(TenantId::from_uuid) {
            by_root.entry(self.root_of(tenant).await?).or_default().push(tenant);
        }
        let mut moved = 0;
        for (root, sources) in by_root {
            let store = match self.store(root).await {
                Ok(store) => store,
                Err(error) => {
                    tracing::info!(%root, %error, "the report store is not available; facts wait");
                    continue;
                }
            };
            for source in sources {
                moved += self.forward_tenant(database, source, &store).await?;
            }
        }
        Ok(moved)
    }

    /// Ngữ cảnh của khách gốc `root` (chủ kho), khi kho phục vụ được.
    pub(crate) async fn store(&self, root: TenantId) -> Result<TenantContext, Error> {
        let store = self.databases.tenant(root).await?;
        ensure_serving(&store).map_err(|_| crate::service::store_unavailable())?;
        Ok(store)
    }

    /// Đẩy số liệu chưa đẩy của khách `source` (dữ liệu ở `database`) vào kho `store`, theo lô.
    async fn forward_tenant(
        &self,
        database: &Database,
        source: TenantId,
        store: &TenantContext,
    ) -> Result<u64, Error> {
        let mut moved = 0;
        loop {
            let batch = local_fact::Entity::find()
                .filter(local_fact::Column::TenantId.eq(source.as_uuid()))
                .filter(local_fact::Column::ForwardedAt.is_null())
                .order_by_asc(local_fact::Column::Id)
                .limit(self.config.forward_batch)
                .all(database.read())
                .await?;
            if batch.is_empty() {
                return Ok(moved);
            }
            let ids: Vec<Uuid> = batch.iter().map(|row| row.id).collect();
            let full = batch.len() as u64 == self.config.forward_batch;
            write_store(store, &batch).await?;
            // Đích đã commit: giờ mới đánh dấu nguồn.
            local_fact::Entity::update_many()
                .col_expr(local_fact::Column::ForwardedAt, Expr::value(now()))
                .filter(local_fact::Column::TenantId.eq(source.as_uuid()))
                .filter(local_fact::Column::Id.is_in(ids))
                .exec(database.write())
                .await?;
            moved += batch.len() as u64;
            if !full {
                return Ok(moved);
            }
        }
    }

    /// Xoá số liệu cục bộ đã đẩy quá hạn giữ; trả số dòng đã xoá.
    pub(crate) async fn prune_local(&self, database: &Database) -> Result<u64, Error> {
        let cutoff = now() - SignedDuration::from_hours(24 * self.config.local_retention_days);
        let result = local_fact::Entity::delete_many()
            .filter(local_fact::Column::ForwardedAt.lt(cutoff))
            .exec(database.write())
            .await?;
        Ok(result.rows_affected)
    }
}

fn now() -> Timestamp {
    Timestamp::now()
}

/// Một transaction ở database của khách gốc: chèn từng số liệu (trùng thì bỏ qua), số liệu thật sự
/// được chèn thì ghi chiều và cộng vào tổng theo ngày.
async fn write_store(store: &TenantContext, batch: &[local_fact::Model]) -> Result<(), Error> {
    let root = store.id().as_uuid();
    let transaction = store.database().begin_write().await?;
    let received = now();
    for row in batch {
        let inserted = fact::Entity::insert(fact::ActiveModel {
            id: Set(row.id),
            tenant_id: Set(root),
            source_tenant_id: Set(row.tenant_id),
            kind: Set(row.kind.clone()),
            org_id: Set(row.org_id),
            occurred_at: Set(row.occurred_at),
            local_day: Set(row.local_day),
            amount_minor: Set(row.amount_minor),
            amount_currency: Set(row.amount_currency.clone()),
            quantity: Set(row.quantity),
            dimensions: Set(row.dimensions.clone()),
            received_at: Set(received),
        })
        // Không nêu cột: khoá chính `id` và unique `(tenant_id, source_tenant_id, id)` đều có thể đụng
        // khi hai lần đẩy chạy cùng lúc; Postgres chỉ bỏ qua đúng ràng buộc đã nêu.
        .on_conflict(OnConflict::new().do_nothing().to_owned())
        .exec_without_returning(&transaction)
        .await?;
        if inserted != 1 {
            continue;
        }
        let dimensions: BTreeMap<String, String> =
            serde_json::from_str(&row.dimensions).map_err(Error::internal)?;
        if !dimensions.is_empty() {
            let rows = dimensions.into_iter().map(|(name, value)| dimension::ActiveModel {
                tenant_id: Set(root),
                fact_id: Set(row.id),
                name: Set(name),
                value: Set(value),
            });
            dimension::Entity::insert_many(rows).exec_without_returning(&transaction).await?;
        }
        add_daily(&transaction, root, row).await?;
    }
    transaction.commit().await?;
    Ok(())
}

/// Cộng một số liệu vào dòng tổng theo ngày: tạo dòng 0 nếu chưa có (trùng thì bỏ qua), rồi cộng
/// bằng `UPDATE … SET x = x + ?` — hai câu chạy như nhau trên SQLite và Postgres, không dùng
/// `INSERT OR REPLACE` (chỉ SQLite, và xoá dòng cũ).
async fn add_daily<C: ConnectionTrait>(
    transaction: &C,
    root: Uuid,
    row: &local_fact::Model,
) -> Result<(), Error> {
    let currency = row.amount_currency.clone().unwrap_or_default();
    daily::Entity::insert(daily::ActiveModel {
        tenant_id: Set(root),
        source_tenant_id: Set(row.tenant_id),
        org_id: Set(row.org_id),
        kind: Set(row.kind.clone()),
        local_day: Set(row.local_day),
        currency: Set(currency.clone()),
        amount_minor: Set(0),
        quantity: Set(0),
        fact_count: Set(0),
    })
    .on_conflict(
        OnConflict::columns([
            daily::Column::TenantId,
            daily::Column::SourceTenantId,
            daily::Column::OrgId,
            daily::Column::Kind,
            daily::Column::LocalDay,
            daily::Column::Currency,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(transaction)
    .await?;
    daily::Entity::update_many()
        .col_expr(
            daily::Column::AmountMinor,
            Expr::col(daily::Column::AmountMinor).add(row.amount_minor.unwrap_or(0)),
        )
        .col_expr(
            daily::Column::Quantity,
            Expr::col(daily::Column::Quantity).add(row.quantity.unwrap_or(0)),
        )
        .col_expr(daily::Column::FactCount, Expr::col(daily::Column::FactCount).add(1i64))
        .filter(daily::Column::TenantId.eq(root))
        .filter(daily::Column::SourceTenantId.eq(row.tenant_id))
        .filter(daily::Column::OrgId.eq(row.org_id))
        .filter(daily::Column::Kind.eq(row.kind.as_str()))
        .filter(daily::Column::LocalDay.eq(row.local_day))
        .filter(daily::Column::Currency.eq(currency))
        .exec(transaction)
        .await?;
    Ok(())
}
