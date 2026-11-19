//! Job và lịch định kỳ của `orders`. Lịch chạy trên cả database (mọi khách): mỗi lượt một lô,
//! mỗi bản ghi mở ngữ cảnh khách bằng `DatabaseRegistry` rồi gọi service.

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler, JobRegistration, Schedule};

use crate::service::OrdersService;

/// Đánh dấu giỏ bỏ dở và phát `orders.cart_abandoned` (mỗi 10 phút).
pub(crate) const MARK_ABANDONED_CARTS: &str = "orders.mark_abandoned_carts";
/// Xoá giỏ quá hạn (mỗi ngày).
pub(crate) const PURGE_CARTS: &str = "orders.purge_carts";

/// Số bản ghi một lượt của job định kỳ.
pub(crate) const BATCH: u64 = 100;

pub(crate) fn registrations(service: &Arc<OrdersService>) -> Vec<JobRegistration> {
    vec![
        JobRegistration::new(MARK_ABANDONED_CARTS, Arc::new(MarkAbandoned(service.clone()))),
        JobRegistration::new(PURGE_CARTS, Arc::new(Purge(service.clone()))),
    ]
}

pub(crate) fn schedules() -> Vec<Schedule> {
    vec![
        Schedule::new(MARK_ABANDONED_CARTS, MARK_ABANDONED_CARTS, Duration::from_secs(600)),
        Schedule::new(PURGE_CARTS, PURGE_CARTS, Duration::from_secs(86_400)),
    ]
}

struct MarkAbandoned(Arc<OrdersService>);

#[async_trait::async_trait]
impl JobHandler for MarkAbandoned {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.mark_abandoned(&job.database).await.map(|_| ())
    }
}

struct Purge(Arc<OrdersService>);

#[async_trait::async_trait]
impl JobHandler for Purge {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.purge_carts(&job.database).await.map(|_| ())
    }
}
