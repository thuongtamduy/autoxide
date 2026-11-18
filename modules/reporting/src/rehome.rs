//! Dời kho báo cáo khi cây khách hàng đổi (bước 38, tách / gộp khách — D-20261118-step38-relocation).
//!
//! Kernel báo `kernel.relocation_finished` ở khách nhận khi một lượt tách / gộp xong (payload: khách
//! nguồn, khách nhận, nút, khách gốc của cây trước và sau). Bộ nghe chỉ xếp job `reporting.rehome`
//! (handler ngắn, D-158); job chép số liệu của phần đã đi từ kho của gốc cũ sang kho của gốc mới,
//! mang khách nguồn mới (khách mới khi tách, khách nhận khi gộp), tính lại tổng theo ngày của hai
//! bên, rồi xoá ở kho cũ — mỗi kho một transaction, "ghi đích idempotent rồi xoá nguồn": chạy lại
//! không đếm hai lần. Số liệu cục bộ chưa đẩy đi theo dữ liệu (chính sách `ByOrg`) và được đẩy
//! bình thường dưới khách mới. Nhật ký ở `reporting_rehomes` (database của khách nhận).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use autoxide_kernel::tenancy::{OrgId, OrgTree, TenantContext, TenantId};
use chrono::{NaiveDate, SubsecRound, Utc};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entity::{daily, dimension, fact, rehome};
use crate::service::Reporting;

/// Event của kernel mà bộ nghe nhận.
pub(crate) const RELOCATION_FINISHED: &str = "kernel.relocation_finished";
/// Loại job dời kho.
pub(crate) const REHOME: &str = "reporting.rehome";

/// Payload của `kernel.relocation_finished` (phần cần dùng).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Rehome {
    relocation_id: Uuid,
    kind: String,
    source_tenant_id: Uuid,
    target_tenant_id: Option<Uuid>,
    org_id: Option<Uuid>,
    from_root: Option<Uuid>,
    to_root: Option<Uuid>,
}

impl Job for Rehome {
    const KIND: &'static str = REHOME;
}

/// Bộ nghe `kernel.relocation_finished`: xếp job dời kho (chạy lại event không xếp job trùng).
pub(crate) struct RelocationListener {
    pub(crate) jobs: JobQueue,
}

#[async_trait::async_trait]
impl EventHandler for RelocationListener {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let rehome: Rehome =
            serde_json::from_value(event.payload.clone()).map_err(Error::internal)?;
        let options =
            EnqueueOptions::default().unique_key(format!("rehome:{}", rehome.relocation_id));
        self.jobs.enqueue(event.tenant.database().write(), &event.tenant, &rehome, options).await?;
        Ok(())
    }
}

pub(crate) struct RehomeJob(pub Arc<Reporting>);

#[async_trait::async_trait]
impl JobHandler for RehomeJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let rehome: Rehome =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        let tenant =
            job.tenant.clone().ok_or_else(|| Error::internal("rehome job without tenant"))?;
        self.0.rehome(&tenant, &rehome).await.map(|_| ())
    }
}

fn now() -> chrono::DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

impl Reporting {
    /// Dời số liệu của lượt `rehome` (ghi nhật ký ở database của `tenant` — khách nhận). Trả số
    /// liệu đã dời.
    pub(crate) async fn rehome(
        &self,
        tenant: &TenantContext,
        rehome: &Rehome,
    ) -> Result<u64, Error> {
        let (Some(target), Some(from_root), Some(to_root)) =
            (rehome.target_tenant_id, rehome.from_root, rehome.to_root)
        else {
            return Ok(0);
        };
        let source = rehome.source_tenant_id;
        // Tách: chỉ các nút của nhánh (nay là cây của khách mới). Gộp: mọi số liệu của khách bị gộp.
        let orgs: Option<Vec<Uuid>> = match (rehome.kind.as_str(), rehome.org_id) {
            ("split", Some(org)) => Some(
                OrgTree::descendants(
                    tenant.database().read(),
                    tenant.id(),
                    &[OrgId::from_uuid(org)],
                )
                .await?
                .into_iter()
                .map(|org| org.as_uuid())
                .collect(),
            ),
            _ => None,
        };
        let log = rehome::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            relocation_id: Set(rehome.relocation_id),
            from_root: Set(from_root),
            to_root: Set(to_root),
            status: Set("running".to_owned()),
            moved_facts: Set(0),
            started_at: Set(now()),
            finished_at: Set(None),
        }
        .insert(tenant.database().write())
        .await?;
        let from_store = self.databases.tenant(TenantId::from_uuid(from_root)).await?;
        let to_store = self.databases.tenant(TenantId::from_uuid(to_root)).await?;
        let mut select = fact::Entity::find()
            .filter(fact::Column::TenantId.eq(from_root))
            .filter(fact::Column::SourceTenantId.eq(source));
        if let Some(orgs) = &orgs {
            select = select.filter(fact::Column::OrgId.is_in(orgs.iter().copied()));
        }
        let facts = select.all(from_store.database().read()).await?;
        let ids: Vec<Uuid> = facts.iter().map(|row| row.id).collect();
        let mut dimensions = Vec::new();
        for chunk in ids.chunks(500) {
            dimensions.extend(
                dimension::Entity::find()
                    .filter(dimension::Column::TenantId.eq(from_root))
                    .filter(dimension::Column::FactId.is_in(chunk.iter().copied()))
                    .all(from_store.database().read())
                    .await?,
            );
        }
        if from_root == to_root {
            // Cùng kho (khoá chính của số liệu chỉ là `id`): đổi khách nguồn tại chỗ.
            let write = to_store.database().begin_write().await?;
            for chunk in ids.chunks(500) {
                fact::Entity::update_many()
                    .col_expr(fact::Column::SourceTenantId, Expr::value(target))
                    .filter(fact::Column::TenantId.eq(from_root))
                    .filter(fact::Column::SourceTenantId.eq(source))
                    .filter(fact::Column::Id.is_in(chunk.iter().copied()))
                    .exec(&write)
                    .await?;
            }
            recompute_daily(&write, to_root, target).await?;
            recompute_daily(&write, from_root, source).await?;
            write.commit().await?;
            return self.finish(tenant, log, facts.len() as u64, rehome.relocation_id).await;
        }
        // Kho mới: chèn (trùng thì bỏ qua), tính lại tổng của khách nguồn mới.
        let write = to_store.database().begin_write().await?;
        for chunk in facts.chunks(100) {
            let rows = chunk.iter().cloned().map(|row| {
                fact::ActiveModel::from(fact::Model {
                    tenant_id: to_root,
                    source_tenant_id: target,
                    ..row
                })
            });
            fact::Entity::insert_many(rows)
                .on_conflict_do_nothing()
                .exec_without_returning(&write)
                .await?;
        }
        for chunk in dimensions.chunks(100) {
            let rows = chunk.iter().cloned().map(|row| {
                dimension::ActiveModel::from(dimension::Model { tenant_id: to_root, ..row })
            });
            dimension::Entity::insert_many(rows)
                .on_conflict_do_nothing()
                .exec_without_returning(&write)
                .await?;
        }
        recompute_daily(&write, to_root, target).await?;
        write.commit().await?;
        // Kho cũ: xoá phần đã có ở kho mới, tính lại tổng của khách nguồn cũ.
        let write = from_store.database().begin_write().await?;
        for chunk in ids.chunks(500) {
            dimension::Entity::delete_many()
                .filter(dimension::Column::TenantId.eq(from_root))
                .filter(dimension::Column::FactId.is_in(chunk.iter().copied()))
                .exec(&write)
                .await?;
            fact::Entity::delete_many()
                .filter(fact::Column::TenantId.eq(from_root))
                .filter(fact::Column::SourceTenantId.eq(source))
                .filter(fact::Column::Id.is_in(chunk.iter().copied()))
                .exec(&write)
                .await?;
        }
        recompute_daily(&write, from_root, source).await?;
        write.commit().await?;
        self.finish(tenant, log, facts.len() as u64, rehome.relocation_id).await
    }

    /// Ghi xong vào nhật ký dời kho.
    async fn finish(
        &self,
        tenant: &TenantContext,
        log: rehome::Model,
        moved: u64,
        relocation: Uuid,
    ) -> Result<u64, Error> {
        let mut done: rehome::ActiveModel = log.into();
        done.status = Set("done".to_owned());
        done.moved_facts = Set(i64::try_from(moved).unwrap_or(i64::MAX));
        done.finished_at = Set(Some(now()));
        done.update(tenant.database().write()).await?;
        tracing::info!(relocation_id = %relocation, moved, "report store rehomed");
        Ok(moved)
    }
}

/// Tính lại tổng theo ngày của khách nguồn `source` trong kho của gốc `root` từ số liệu đang có.
async fn recompute_daily<C: ConnectionTrait>(
    connection: &C,
    root: Uuid,
    source: Uuid,
) -> Result<(), Error> {
    daily::Entity::delete_many()
        .filter(daily::Column::TenantId.eq(root))
        .filter(daily::Column::SourceTenantId.eq(source))
        .exec(connection)
        .await?;
    // `CAST(SUM(…) AS BIGINT)`: Postgres trả `numeric` cho tổng của `bigint`.
    let groups: Vec<(Uuid, String, NaiveDate, String, i64, i64, i64)> = fact::Entity::find()
        .select_only()
        .column(fact::Column::OrgId)
        .column(fact::Column::Kind)
        .column(fact::Column::LocalDay)
        .column_as(Expr::cust("COALESCE(amount_currency, '')"), "currency")
        .column_as(Expr::cust("CAST(COALESCE(SUM(amount_minor), 0) AS BIGINT)"), "amount")
        .column_as(Expr::cust("CAST(COALESCE(SUM(quantity), 0) AS BIGINT)"), "quantity")
        .column_as(Expr::cust("CAST(COUNT(*) AS BIGINT)"), "facts")
        .filter(fact::Column::TenantId.eq(root))
        .filter(fact::Column::SourceTenantId.eq(source))
        .group_by(fact::Column::OrgId)
        .group_by(fact::Column::Kind)
        .group_by(fact::Column::LocalDay)
        .group_by(Expr::cust("COALESCE(amount_currency, '')"))
        .into_tuple()
        .all(connection)
        .await?;
    for chunk in groups.chunks(100) {
        let rows = chunk.iter().cloned().map(
            |(org_id, kind, local_day, currency, amount, quantity, facts)| daily::ActiveModel {
                tenant_id: Set(root),
                source_tenant_id: Set(source),
                org_id: Set(org_id),
                kind: Set(kind),
                local_day: Set(local_day),
                currency: Set(currency),
                amount_minor: Set(amount),
                quantity: Set(quantity),
                fact_count: Set(facts),
            },
        );
        daily::Entity::insert_many(rows)
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
            .exec_without_returning(connection)
            .await?;
    }
    Ok(())
}
