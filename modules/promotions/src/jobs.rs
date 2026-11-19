//! Job định kỳ của `promotions`. Lịch chạy theo database (không có khách): mỗi job tìm khách có
//! việc tới hạn trong database đó rồi mở ngữ cảnh khách (`DatabaseRegistry::tenant`) trước khi gọi
//! service (D-198). Mọi việc đổi có điều kiện: hai worker chạy cùng lúc không làm hai lần.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler, JobRegistration, Schedule};
use autoxide_kernel::tenancy::TenantId;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use uuid::Uuid;

use crate::codes::{GenerateCodes, GenerateCodesJob};
use crate::entity::{promotion, redemption, voucher};
use crate::service::{PromotionsService, now};

pub(crate) const TICK: &str = "promotions.tick";
pub(crate) const EXPIRE_HOLDS: &str = "promotions.expire_holds";
pub(crate) const EXPIRE_VOUCHERS: &str = "promotions.expire_vouchers";

pub(crate) fn registrations(service: &Arc<PromotionsService>) -> Vec<JobRegistration> {
    use autoxide_kernel::job::Job;
    vec![
        JobRegistration::new(TICK, Arc::new(Tick(service.clone()))),
        JobRegistration::new(EXPIRE_HOLDS, Arc::new(ExpireHolds(service.clone()))),
        JobRegistration::new(EXPIRE_VOUCHERS, Arc::new(ExpireVouchers(service.clone()))),
        JobRegistration::new(GenerateCodes::KIND, Arc::new(GenerateCodesJob(service.clone()))),
    ]
}

pub(crate) fn schedules() -> Vec<Schedule> {
    vec![
        Schedule::new(TICK, TICK, Duration::from_secs(60)),
        Schedule::new(EXPIRE_HOLDS, EXPIRE_HOLDS, Duration::from_secs(60)),
        Schedule::new(EXPIRE_VOUCHERS, EXPIRE_VOUCHERS, Duration::from_secs(3600)),
    ]
}

/// Khách (trong database) có dòng khớp; tối đa 500 mỗi lượt.
async fn tenants(database: &Database, rows: Vec<Uuid>) -> Vec<TenantId> {
    let _ = database;
    rows.into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(500)
        .map(TenantId::from_uuid)
        .collect()
}

impl PromotionsService {
    async fn for_each_tenant<F, Fut>(
        &self,
        ids: Vec<TenantId>,
        kind: &str,
        work: F,
    ) -> Result<(), Error>
    where
        F: Fn(autoxide_kernel::tenancy::TenantContext) -> Fut,
        Fut: std::future::Future<Output = Result<u64, Error>>,
    {
        for id in ids {
            let tenant = self.databases.tenant(id).await?;
            if let Err(error) = work(tenant).await {
                tracing::warn!(tenant_id = %id.as_uuid(), job = kind, error = %error, "promotion job failed for a tenant");
            }
        }
        Ok(())
    }
}

struct Tick(Arc<PromotionsService>);

#[async_trait::async_trait]
impl JobHandler for Tick {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let at = now();
        let rows: Vec<Uuid> = promotion::Entity::find()
            .select_only()
            .column(promotion::Column::TenantId)
            .filter(
                sea_orm::Condition::any()
                    .add(
                        sea_orm::Condition::all()
                            .add(promotion::Column::Status.eq("scheduled"))
                            .add(promotion::Column::StartsAt.lte(at)),
                    )
                    .add(
                        sea_orm::Condition::all()
                            .add(promotion::Column::Status.is_in(["scheduled", "active", "paused"]))
                            .add(promotion::Column::EndsAt.lte(at)),
                    ),
            )
            .limit(1000)
            .into_tuple()
            .all(job.database.read())
            .await?;
        let ids = tenants(&job.database, rows).await;
        let service = &self.0;
        service
            .for_each_tenant(ids, TICK, |tenant| async move { service.tick(&tenant).await })
            .await
    }
}

struct ExpireHolds(Arc<PromotionsService>);

#[async_trait::async_trait]
impl JobHandler for ExpireHolds {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let rows: Vec<Uuid> = redemption::Entity::find()
            .select_only()
            .column(redemption::Column::TenantId)
            .filter(redemption::Column::Status.eq("held"))
            .filter(redemption::Column::ExpiresAt.lte(now()))
            .limit(1000)
            .into_tuple()
            .all(job.database.read())
            .await?;
        let ids = tenants(&job.database, rows).await;
        let service = &self.0;
        service
            .for_each_tenant(ids, EXPIRE_HOLDS, |tenant| async move {
                service.expire_holds(&tenant).await
            })
            .await
    }
}

struct ExpireVouchers(Arc<PromotionsService>);

#[async_trait::async_trait]
impl JobHandler for ExpireVouchers {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let rows: Vec<Uuid> = voucher::Entity::find()
            .select_only()
            .column(voucher::Column::TenantId)
            .filter(voucher::Column::Status.eq("available"))
            .filter(voucher::Column::ExpiresAt.lte(now()))
            .limit(1000)
            .into_tuple()
            .all(job.database.read())
            .await?;
        let ids = tenants(&job.database, rows).await;
        let service = &self.0;
        service
            .for_each_tenant(ids, EXPIRE_VOUCHERS, |tenant| async move {
                service.expire_vouchers(&tenant).await
            })
            .await
    }
}
