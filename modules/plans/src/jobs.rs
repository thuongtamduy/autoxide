//! Job của module `plans`: trả chỗ giữ quá hạn (mỗi phút), đối soát số đang dùng (hằng ngày). Cả
//! hai chỉ chạm bảng mặt điều khiển nên chỉ chạy trên database điều khiển.

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

pub(crate) fn registrations(service: &Arc<PlansService>) -> Vec<JobRegistration> {
    vec![
        JobRegistration::new(EXPIRE_HOLDS, Arc::new(ExpireHolds(service.clone()))),
        JobRegistration::new(RECONCILE_USAGE, Arc::new(ReconcileUsage(service.clone()))),
    ]
}

pub(crate) fn schedules() -> Vec<Schedule> {
    vec![
        Schedule::new(EXPIRE_HOLDS, EXPIRE_HOLDS, Duration::from_secs(60)).on_control_only(),
        Schedule::new(RECONCILE_USAGE, RECONCILE_USAGE, Duration::from_secs(86_400))
            .on_control_only(),
    ]
}
