//! Job của module `plans`: trả chỗ giữ quá hạn (mỗi phút), đối soát số đang dùng (hằng ngày), vòng
//! đời thuê bao (mỗi 5 phút), lập kỳ thanh toán sắp tới và mail nhắc (mỗi giờ). Chỉ chạm bảng mặt
//! điều khiển nên chỉ chạy trên database điều khiển.

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler, JobRegistration, Schedule};

use crate::service::{PlansService, now};

pub(crate) const EXPIRE_HOLDS: &str = "plans.expire_holds";
pub(crate) const RECONCILE_USAGE: &str = "plans.reconcile_usage";

struct ExpireHolds(Arc<PlansService>);

#[async_trait::async_trait]
impl JobHandler for ExpireHolds {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        // Lô 500 mỗi lượt; còn thì lượt sau (mỗi phút) làm tiếp.
        self.0.expire_holds(now(), 500).await.map(|_| ())
    }
}

struct ReconcileUsage(Arc<PlansService>);

#[async_trait::async_trait]
impl JobHandler for ReconcileUsage {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        self.0.reconcile(now()).await.map(|_| ())
    }
}

pub(crate) const LIFECYCLE: &str = "plans.lifecycle";
pub(crate) const RENEWALS: &str = "plans.renewals";
pub(crate) const REMINDERS: &str = "plans.reminders";

/// Chuyển trạng thái thuê bao tới hạn (mỗi 5 phút).
struct Lifecycle(Arc<PlansService>);

#[async_trait::async_trait]
impl JobHandler for Lifecycle {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        self.0.run_lifecycle(now()).await.map(|_| ())
    }
}

/// Lập kỳ thanh toán sắp tới (mỗi giờ).
struct Renewals(Arc<PlansService>);

#[async_trait::async_trait]
impl JobHandler for Renewals {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        self.0.run_renewals(now()).await.map(|_| ())
    }
}

/// Mail nhắc thanh toán (mỗi giờ).
struct Reminders(Arc<PlansService>);

#[async_trait::async_trait]
impl JobHandler for Reminders {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        self.0.run_reminders(now()).await.map(|_| ())
    }
}

pub(crate) fn registrations(service: &Arc<PlansService>) -> Vec<JobRegistration> {
    vec![
        JobRegistration::new(EXPIRE_HOLDS, Arc::new(ExpireHolds(service.clone()))),
        JobRegistration::new(RECONCILE_USAGE, Arc::new(ReconcileUsage(service.clone()))),
        JobRegistration::new(LIFECYCLE, Arc::new(Lifecycle(service.clone()))),
        JobRegistration::new(RENEWALS, Arc::new(Renewals(service.clone()))),
        JobRegistration::new(REMINDERS, Arc::new(Reminders(service.clone()))),
        JobRegistration::new(
            crate::provisioning::PROVISION_JOB,
            Arc::new(crate::provisioning::ProvisionRunner(service.clone())),
        ),
    ]
}

pub(crate) fn schedules() -> Vec<Schedule> {
    let every = Duration::from_secs;
    vec![
        Schedule::new(EXPIRE_HOLDS, EXPIRE_HOLDS, every(60)).on_control_only(),
        Schedule::new(RECONCILE_USAGE, RECONCILE_USAGE, every(86_400)).on_control_only(),
        Schedule::new(LIFECYCLE, LIFECYCLE, every(300)).on_control_only(),
        Schedule::new(RENEWALS, RENEWALS, every(3600)).on_control_only(),
        Schedule::new(REMINDERS, REMINDERS, every(3600)).on_control_only(),
    ]
}
