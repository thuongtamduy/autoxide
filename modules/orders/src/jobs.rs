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

/// Hoàn tất đơn đã giao quá hạn đổi trả (mỗi giờ).
pub(crate) const COMPLETE_DELIVERED: &str = "orders.complete_delivered";
/// Huỷ đơn cửa hàng không nhận kịp (mỗi 10 phút).
pub(crate) const EXPIRE_UNACCEPTED: &str = "orders.expire_unaccepted";

/// Nhắc / hết hạn giữ hàng chờ nhận tại cửa hàng (mỗi giờ).
pub(crate) const EXPIRE_PICKUPS: &str = "orders.expire_pickups";

/// Đánh dấu bảo hành hết hạn (mỗi ngày).
pub(crate) const EXPIRE_WARRANTIES: &str = "orders.expire_warranties";

/// Quá hạn trả phần còn lại của đơn trả trước một phần (mỗi giờ, D-232).
pub(crate) const EXPIRE_PREORDER_BALANCES: &str = "orders.expire_preorder_balances";

/// Số bản ghi một lượt của job định kỳ.
pub(crate) const BATCH: u64 = 100;

pub(crate) fn registrations(service: &Arc<OrdersService>) -> Vec<JobRegistration> {
    vec![
        JobRegistration::new(MARK_ABANDONED_CARTS, Arc::new(MarkAbandoned(service.clone()))),
        JobRegistration::new(PURGE_CARTS, Arc::new(Purge(service.clone()))),
        JobRegistration::new(EXPIRE_UNPAID, Arc::new(ExpireUnpaid(service.clone()))),
        JobRegistration::new(RECOVER_CHECKOUTS, Arc::new(Recover(service.clone()))),
        JobRegistration::new(COMPLETE_DELIVERED, Arc::new(Complete(service.clone()))),
        JobRegistration::new(EXPIRE_UNACCEPTED, Arc::new(Unaccepted(service.clone()))),
        JobRegistration::new(EXPIRE_PICKUPS, Arc::new(Pickups(service.clone()))),
        JobRegistration::new(EXPIRE_WARRANTIES, Arc::new(Warranties(service.clone()))),
        JobRegistration::new(EXPIRE_PREORDER_BALANCES, Arc::new(Balances(service.clone()))),
    ]
    .into_iter()
    .chain(crate::reviews::registrations(service))
    .collect()
}

pub(crate) fn schedules() -> Vec<Schedule> {
    vec![
        Schedule::new(MARK_ABANDONED_CARTS, MARK_ABANDONED_CARTS, Duration::from_secs(600)),
        Schedule::new(PURGE_CARTS, PURGE_CARTS, Duration::from_secs(86_400)),
        Schedule::new(EXPIRE_UNPAID, EXPIRE_UNPAID, Duration::from_secs(60)),
        Schedule::new(RECOVER_CHECKOUTS, RECOVER_CHECKOUTS, Duration::from_secs(60)),
        Schedule::new(COMPLETE_DELIVERED, COMPLETE_DELIVERED, Duration::from_secs(3600)),
        Schedule::new(EXPIRE_UNACCEPTED, EXPIRE_UNACCEPTED, Duration::from_secs(600)),
        Schedule::new(EXPIRE_PICKUPS, EXPIRE_PICKUPS, Duration::from_secs(3600)),
        Schedule::new(EXPIRE_WARRANTIES, EXPIRE_WARRANTIES, Duration::from_secs(86_400)),
        Schedule::new(
            EXPIRE_PREORDER_BALANCES,
            EXPIRE_PREORDER_BALANCES,
            Duration::from_secs(3600),
        ),
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

struct Complete(Arc<OrdersService>);

#[async_trait::async_trait]
impl JobHandler for Complete {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.complete_delivered(&job.database).await.map(|_| ())
    }
}

struct Unaccepted(Arc<OrdersService>);

#[async_trait::async_trait]
impl JobHandler for Unaccepted {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.expire_unaccepted(&job.database).await.map(|_| ())
    }
}

struct Pickups(Arc<OrdersService>);

#[async_trait::async_trait]
impl JobHandler for Pickups {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.expire_pickups(&job.database).await.map(|_| ())
    }
}

struct Warranties(Arc<OrdersService>);

#[async_trait::async_trait]
impl JobHandler for Warranties {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.expire_warranties(&job.database).await.map(|_| ())
    }
}

struct Balances(Arc<OrdersService>);

#[async_trait::async_trait]
impl JobHandler for Balances {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.expire_preorder_balances(&job.database).await.map(|_| ())
    }
}
