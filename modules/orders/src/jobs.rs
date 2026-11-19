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

/// Huỷ đơn quá hạn thanh toán (mỗi phút).
pub(crate) const EXPIRE_UNPAID: &str = "orders.expire_unpaid";
/// Trả chỗ của lượt đặt treo (mỗi phút).
pub(crate) const RECOVER_CHECKOUTS: &str = "orders.recover_checkouts";

/// Số bản ghi một lượt của job định kỳ.
pub(crate) const BATCH: u64 = 100;

pub(crate) fn registrations(service: &Arc<OrdersService>) -> Vec<JobRegistration> {
    vec![
        JobRegistration::new(MARK_ABANDONED_CARTS, Arc::new(MarkAbandoned(service.clone()))),
        JobRegistration::new(PURGE_CARTS, Arc::new(Purge(service.clone()))),
        JobRegistration::new(EXPIRE_UNPAID, Arc::new(ExpireUnpaid(service.clone()))),
        JobRegistration::new(RECOVER_CHECKOUTS, Arc::new(Recover(service.clone()))),
    ]
}

pub(crate) fn schedules() -> Vec<Schedule> {
    vec![
        Schedule::new(MARK_ABANDONED_CARTS, MARK_ABANDONED_CARTS, Duration::from_secs(600)),
        Schedule::new(PURGE_CARTS, PURGE_CARTS, Duration::from_secs(86_400)),
        Schedule::new(EXPIRE_UNPAID, EXPIRE_UNPAID, Duration::from_secs(60)),
        Schedule::new(RECOVER_CHECKOUTS, RECOVER_CHECKOUTS, Duration::from_secs(60)),
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

struct ExpireUnpaid(Arc<OrdersService>);

#[async_trait::async_trait]
impl JobHandler for ExpireUnpaid {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.expire_unpaid(&job.database).await.map(|_| ())
    }
}

struct Recover(Arc<OrdersService>);

#[async_trait::async_trait]
impl JobHandler for Recover {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.recover_checkouts(&job.database).await.map(|_| ())
    }
}
