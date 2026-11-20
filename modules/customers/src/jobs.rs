//! Job theo lịch của điểm thưởng (bước 47 mục 3, 6). Lịch chạy theo database (không có khách): mỗi
//! job tìm khách có việc trong database đó rồi mở ngữ cảnh khách (`DatabaseRegistry::tenant`) trước
//! khi gọi service (D-198). Mọi việc đổi có điều kiện: hai worker cùng chạy không làm hai lần.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler, JobRegistration, Schedule};
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use uuid::Uuid;

use crate::entity::{loyalty_rule, point_hold, voucher_program};
use crate::loyalty::ledger::{POST_LEDGER, PostLedgerJob};
use crate::service::{CustomersService, now};

pub(crate) const EXPIRE_POINTS: &str = "customers.expire_points";
pub(crate) const RELEASE_EXPIRED_HOLDS: &str = "customers.release_expired_holds";
pub(crate) const EVALUATE_TIERS: &str = "customers.evaluate_tiers";
pub(crate) const BIRTHDAY_VOUCHERS: &str = "customers.birthday_vouchers";

const DAY: Duration = Duration::from_secs(24 * 3600);

pub(crate) fn registrations(service: &Arc<CustomersService>) -> Vec<JobRegistration> {
    vec![
        JobRegistration::new(POST_LEDGER, Arc::new(PostLedgerJob(service.clone()))),
        JobRegistration::new(
            crate::engagement::SEND_CART_REMINDER,
            Arc::new(crate::engagement::SendCartReminderJob(service.clone())),
        ),
        JobRegistration::new(
            crate::engagement::PURGE_RECENT_VIEWS,
            Arc::new(crate::engagement::PurgeRecentViewsJob(service.clone())),
        ),
        JobRegistration::new(
            EXPIRE_POINTS,
            Arc::new(Sweep { service: service.clone(), kind: EXPIRE_POINTS }),
        ),
        JobRegistration::new(
            RELEASE_EXPIRED_HOLDS,
            Arc::new(Sweep { service: service.clone(), kind: RELEASE_EXPIRED_HOLDS }),
        ),
        JobRegistration::new(
            EVALUATE_TIERS,
            Arc::new(Sweep { service: service.clone(), kind: EVALUATE_TIERS }),
        ),
        JobRegistration::new(
            BIRTHDAY_VOUCHERS,
            Arc::new(Sweep { service: service.clone(), kind: BIRTHDAY_VOUCHERS }),
        ),
    ]
}

pub(crate) fn schedules() -> Vec<Schedule> {
    vec![
        Schedule::new(EXPIRE_POINTS, EXPIRE_POINTS, DAY),
        // Chỗ giữ điểm của đơn chưa trả: mỗi 5 phút.
        Schedule::new(RELEASE_EXPIRED_HOLDS, RELEASE_EXPIRED_HOLDS, Duration::from_secs(300)),
        Schedule::new(EVALUATE_TIERS, EVALUATE_TIERS, DAY),
        Schedule::new(BIRTHDAY_VOUCHERS, BIRTHDAY_VOUCHERS, DAY),
        Schedule::new(
            crate::engagement::PURGE_RECENT_VIEWS,
            crate::engagement::PURGE_RECENT_VIEWS,
            DAY,
        ),
    ]
}

/// Một lượt quét theo lịch của loại `kind`.
struct Sweep {
    service: Arc<CustomersService>,
    kind: &'static str,
}

impl Sweep {
    /// Khách trong database của job có việc của loại này.
    async fn tenants(&self, job: &JobContext) -> Result<Vec<TenantId>, Error> {
        let read = job.database.read();
        let rows: Vec<Uuid> = match self.kind {
            RELEASE_EXPIRED_HOLDS => {
                point_hold::Entity::find()
                    .filter(point_hold::Column::Status.eq(crate::loyalty::tender::HELD))
                    .filter(point_hold::Column::ExpiresAt.lte(now()))
                    .select_only()
                    .column(point_hold::Column::TenantId)
                    .limit(1000)
                    .into_tuple()
                    .all(read)
                    .await?
            }
            BIRTHDAY_VOUCHERS => {
                voucher_program::Entity::find()
                    .filter(voucher_program::Column::Kind.eq(crate::loyalty::vouchers::BIRTHDAY))
                    .filter(voucher_program::Column::Active.eq(true))
                    .select_only()
                    .column(voucher_program::Column::TenantId)
                    .into_tuple()
                    .all(read)
                    .await?
            }
            // Điểm và hạng: khách có chương trình điểm.
            _ => {
                loyalty_rule::Entity::find()
                    .select_only()
                    .column(loyalty_rule::Column::TenantId)
                    .into_tuple()
                    .all(read)
                    .await?
            }
        };
        Ok(rows.into_iter().collect::<BTreeSet<_>>().into_iter().map(TenantId::from_uuid).collect())
    }

    async fn run_for(&self, tenant: &TenantContext) -> Result<u64, Error> {
        match self.kind {
            EXPIRE_POINTS => self.service.expire_lots(tenant, now()).await,
            RELEASE_EXPIRED_HOLDS => self.service.release_expired_holds(tenant).await,
            EVALUATE_TIERS => self.service.evaluate_tiers(tenant).await,
            _ => self.service.birthday_vouchers(tenant).await,
        }
    }
}

#[async_trait::async_trait]
impl JobHandler for Sweep {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        if let Some(tenant) = &job.tenant {
            self.run_for(tenant).await?;
            return Ok(());
        }
        for id in self.tenants(job).await? {
            let tenant = self.service.databases.tenant(id).await?;
            match self.run_for(&tenant).await {
                Ok(done) if done > 0 => {
                    tracing::info!(tenant_id = %id.as_uuid(), job = self.kind, done, "customers sweep");
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(tenant_id = %id.as_uuid(), job = self.kind, error = %error, "customers sweep failed for a tenant");
                }
            }
        }
        Ok(())
    }
}
