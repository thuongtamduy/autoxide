//! Job và lịch của `shipping`. Mọi job chạy lại được (job chạy ít nhất một lần).

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{Job, JobContext, JobHandler, JobRegistration, Schedule};
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::ShipmentStatus;
use crate::entity::shipment;
use crate::service::{ShippingService, now};

pub(crate) const BOOK: &str = "shipping.book";
pub(crate) const CANCEL: &str = "shipping.cancel";
pub(crate) const PROCESS_WEBHOOK: &str = "shipping.process_webhook";
pub(crate) const RECONCILE_BOOKINGS: &str = "shipping.reconcile_bookings";
pub(crate) const POLL_TRACKING: &str = "shipping.poll_tracking";
pub(crate) const POST_COD: &str = "shipping.post_cod";
pub(crate) const SETTLE_COD: &str = "shipping.settle_cod";
pub(crate) const WRITE_OFF_COD: &str = "shipping.write_off_cod";
pub(crate) const COD_AGING: &str = "shipping.cod_aging";

/// Ghi sổ cái khi giao xong có COD.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PostCod {
    pub shipment_id: Uuid,
}

impl Job for PostCod {
    const KIND: &'static str = POST_COD;
}

/// Ghi sổ cái cho một dòng bảng kê khớp.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SettleCod {
    pub line_id: Uuid,
}

impl Job for SettleCod {
    const KIND: &'static str = SETTLE_COD;
}

/// Ghi sổ cái xoá khoản COD không thu được.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct WriteOffCod {
    pub discrepancy_id: Uuid,
}

impl Job for WriteOffCod {
    const KIND: &'static str = WRITE_OFF_COD;
}

/// Đặt vận đơn với hãng.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BookShipment {
    pub shipment_id: Uuid,
}

impl Job for BookShipment {
    const KIND: &'static str = BOOK;
}

/// Huỷ vận đơn (đơn bị huỷ).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CancelShipment {
    pub shipment_id: Uuid,
}

impl Job for CancelShipment {
    const KIND: &'static str = CANCEL;
}

/// Áp một webhook đã lưu.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ProcessWebhook {
    pub inbox_id: Uuid,
}

impl Job for ProcessWebhook {
    const KIND: &'static str = PROCESS_WEBHOOK;
}

pub(crate) fn registrations(service: &Arc<ShippingService>) -> Vec<JobRegistration> {
    vec![
        JobRegistration::new(BOOK, Arc::new(Book(service.clone()))),
        JobRegistration::new(CANCEL, Arc::new(Cancel(service.clone()))),
        JobRegistration::new(PROCESS_WEBHOOK, Arc::new(Webhook(service.clone()))),
        JobRegistration::new(RECONCILE_BOOKINGS, Arc::new(Reconcile(service.clone()))),
        JobRegistration::new(POLL_TRACKING, Arc::new(Poll(service.clone()))),
        JobRegistration::new(POST_COD, Arc::new(Cod(service.clone()))),
        JobRegistration::new(SETTLE_COD, Arc::new(Settle(service.clone()))),
        JobRegistration::new(WRITE_OFF_COD, Arc::new(WriteOff(service.clone()))),
        JobRegistration::new(COD_AGING, Arc::new(Aging(service.clone()))),
    ]
}

pub(crate) fn schedules() -> Vec<Schedule> {
    vec![
        Schedule::new(RECONCILE_BOOKINGS, RECONCILE_BOOKINGS, Duration::from_secs(5 * 60)),
        Schedule::new(POLL_TRACKING, POLL_TRACKING, Duration::from_secs(30 * 60)),
        Schedule::new(COD_AGING, COD_AGING, Duration::from_secs(24 * 60 * 60)),
    ]
}

fn tenant_of(job: &JobContext) -> Result<&TenantContext, Error> {
    job.tenant.as_ref().ok_or_else(|| Error::internal("shipping job without a tenant"))
}

fn payload<T: for<'de> Deserialize<'de>>(job: &JobContext) -> Result<T, Error> {
    serde_json::from_value(job.payload.clone()).map_err(Error::internal)
}

struct Book(Arc<ShippingService>);

#[async_trait::async_trait]
impl JobHandler for Book {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let BookShipment { shipment_id } = payload(job)?;
        self.0.book(tenant_of(job)?, Actor::system(), shipment_id).await.map(|_| ())
    }
}

struct Cancel(Arc<ShippingService>);

#[async_trait::async_trait]
impl JobHandler for Cancel {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let CancelShipment { shipment_id } = payload(job)?;
        match self.0.cancel(tenant_of(job)?, Actor::system(), shipment_id).await {
            // Hãng đã lấy hàng: không huỷ được; `orders` xử lý bằng hoàn hàng.
            Err(error) if error.code() == "shipping.cancel_too_late" => {
                tracing::warn!(shipment_id = %shipment_id, "an order was cancelled after the carrier picked up");
                Ok(())
            }
            other => other.map(|_| ()),
        }
    }
}

struct Webhook(Arc<ShippingService>);

#[async_trait::async_trait]
impl JobHandler for Webhook {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let ProcessWebhook { inbox_id } = payload(job)?;
        self.0.process_webhook(tenant_of(job)?, inbox_id).await
    }
}

/// Quét vận đơn của mọi khách trong database của job theo trạng thái, tối đa 200 một lượt.
async fn due(
    job: &JobContext,
    statuses: &[ShipmentStatus],
    older_than: chrono::Duration,
) -> Result<Vec<(Uuid, Uuid)>, Error> {
    Ok(shipment::Entity::find()
        .select_only()
        .column(shipment::Column::TenantId)
        .column(shipment::Column::Id)
        .filter(shipment::Column::Status.is_in(statuses.iter().map(|status| status.as_str())))
        .filter(shipment::Column::CarrierAccountId.is_not_null())
        .filter(shipment::Column::StatusAt.lte(now() - older_than))
        .limit(200)
        .into_tuple()
        .all(job.database.read())
        .await?)
}

struct Reconcile(Arc<ShippingService>);

#[async_trait::async_trait]
impl JobHandler for Reconcile {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        for (tenant, id) in
            due(job, &[ShipmentStatus::Booking], chrono::Duration::minutes(10)).await?
        {
            let tenant = self.0.databases.tenant(TenantId::from_uuid(tenant)).await?;
            if let Err(error) = self.0.reconcile_booking(&tenant, id).await {
                tracing::warn!(shipment_id = %id, error = %error, "cannot reconcile a stuck booking");
            }
        }
        Ok(())
    }
}

struct Poll(Arc<ShippingService>);

#[async_trait::async_trait]
impl JobHandler for Poll {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let moving = [
            ShipmentStatus::ReadyToShip,
            ShipmentStatus::PickedUp,
            ShipmentStatus::InTransit,
            ShipmentStatus::OutForDelivery,
            ShipmentStatus::DeliveryFailed,
            ShipmentStatus::Returning,
        ];
        for (tenant, id) in due(job, &moving, chrono::Duration::minutes(30)).await? {
            let tenant = self.0.databases.tenant(TenantId::from_uuid(tenant)).await?;
            // Chỉ hãng không gửi webhook mới cần hỏi.
            if !self.0.polls(&tenant, id).await.unwrap_or(false) {
                continue;
            }
            if let Err(error) = self.0.refresh_tracking(&tenant, id).await {
                tracing::warn!(shipment_id = %id, error = %error, "cannot poll the carrier for tracking");
            }
        }
        Ok(())
    }
}

struct Cod(Arc<ShippingService>);

#[async_trait::async_trait]
impl JobHandler for Cod {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let PostCod { shipment_id } = payload(job)?;
        self.0.post_cod(tenant_of(job)?, shipment_id).await
    }
}

struct Settle(Arc<ShippingService>);

#[async_trait::async_trait]
impl JobHandler for Settle {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let SettleCod { line_id } = payload(job)?;
        self.0.post_settlement(tenant_of(job)?, line_id).await
    }
}

struct WriteOff(Arc<ShippingService>);

#[async_trait::async_trait]
impl JobHandler for WriteOff {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let WriteOffCod { discrepancy_id } = payload(job)?;
        self.0.post_write_off(tenant_of(job)?, discrepancy_id).await
    }
}

/// Lịch hằng ngày: khoản COD đã thu mà quá hạn chưa vào bảng kê → chênh lệch `overdue`.
struct Aging(Arc<ShippingService>);

#[async_trait::async_trait]
impl JobHandler for Aging {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenants: Vec<Uuid> = shipment::Entity::find()
            .select_only()
            .column(shipment::Column::TenantId)
            .filter(shipment::Column::CodStatus.eq("collected"))
            .distinct()
            .into_tuple()
            .all(job.database.read())
            .await?;
        for tenant in tenants {
            let tenant = self.0.databases.tenant(TenantId::from_uuid(tenant)).await?;
            if let Err(error) = self.0.cod_aging(&tenant).await {
                tracing::warn!(error = %error, "cannot check overdue cash on delivery");
            }
        }
        Ok(())
    }
}
