//! Job của module `bookings`: trả chỗ giữ tạm quá hạn, hoàn cọc qua `ledger`. Mọi job chạy lại được
//! (job chạy ít nhất một lần).

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::job::{Job, JobContext, JobHandler, JobRegistration, Schedule};
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, TenantId};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::booking::{CONFIRMED, HELD, refund_reference, snapshot};
use crate::entity::booking;
use crate::events::{self, Change};
use crate::service::{BookingsService, now};

pub(crate) const EXPIRE_HOLDS: &str = "bookings.expire_holds";
pub(crate) const REFUND_DEPOSIT: &str = "bookings.refund_deposit";

/// Hoàn phần cọc đang chờ của một lịch.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RefundDeposit {
    pub booking_id: Uuid,
}

impl Job for RefundDeposit {
    const KIND: &'static str = REFUND_DEPOSIT;
}

pub(crate) fn registrations(service: &Arc<BookingsService>) -> Vec<JobRegistration> {
    vec![
        JobRegistration::new(EXPIRE_HOLDS, Arc::new(ExpireHolds(service.clone()))),
        JobRegistration::new(REFUND_DEPOSIT, Arc::new(Refund(service.clone()))),
    ]
}

pub(crate) fn schedules() -> Vec<Schedule> {
    vec![Schedule::new(EXPIRE_HOLDS, EXPIRE_HOLDS, Duration::from_secs(60))]
}

impl BookingsService {
    /// Trả chỗ của mọi lịch `held` quá hạn trong `database` (tối đa 500 một lượt).
    pub(crate) async fn expire_due_holds(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        let due: Vec<(Uuid, Uuid)> = booking::Entity::find()
            .select_only()
            .column(booking::Column::TenantId)
            .column(booking::Column::Id)
            .filter(booking::Column::Status.eq(HELD))
            .filter(booking::Column::HoldExpiresAt.lte(now()))
            .limit(500)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut expired = 0;
        for (tenant, id) in due {
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant)).await?;
            match self.expire(&tenant, id).await {
                Ok(true) => expired += 1,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(booking_id = %id, error = %error, "cannot release a held booking")
                }
            }
        }
        Ok(expired)
    }

    /// Hoàn phần cọc đang chờ của lịch `id` qua `ledger`; đã hoàn (hay không còn chờ) thì thôi.
    pub(crate) async fn refund_deposit(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(), Error> {
        let row = Self::booking_row(tenant.database().read(), tenant, id).await?;
        if row.deposit_status != "refund_pending" || row.refund_minor <= 0 {
            return Ok(());
        }
        let (Some(ledger), Some(payment)) = (&self.ledger, row.payment_id) else {
            return Err(Error::internal(
                "a deposit waits for a refund but there is no payment to refund",
            ));
        };
        let amount = Money::from_columns(row.refund_minor, &row.currency)?;
        let refund = autoxide_ledger::api::NewRefund::new(
            payment,
            amount,
            "Booking deposit refund",
            refund_reference(id),
        );
        ledger.refund(tenant, Actor::system(), refund).await?;
        let transaction = tenant.database().begin_write().await?;
        booking::Entity::update_many()
            .col_expr(booking::Column::DepositStatus, Expr::value("refunded"))
            .col_expr(booking::Column::UpdatedAt, Expr::value(now()))
            .filter(booking::Column::Id.eq(id))
            .filter(booking::Column::DepositStatus.eq("refund_pending"))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        tracing::info!(booking_id = %id, "booking deposit refunded");
        Ok(())
    }

    /// `ledger` báo đã thu cọc của lịch `id` với số thực thu `received` (`None`: đủ cọc): lịch còn
    /// giữ và thu đủ → `confirmed`; lịch đã hết hạn / huỷ (tiền về muộn), hay thu thiếu cọc → không
    /// xác nhận, hoàn lại đúng số đã thu (lịch thu thiếu vẫn `held` tới khi hết hạn giữ). Đã xử lý
    /// rồi thì thôi (event đến lại).
    pub(crate) async fn deposit_paid(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        payment_id: Uuid,
        received: Option<Money>,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        // Khoá lịch (D-181): lịch đang bị `expire` (held → expired, trả chỗ) thì chờ nó xong rồi đi
        // nhánh hoàn cọc — không ghi đè `expired` thành `confirmed` trên chỗ đã nhả cho người khác.
        let Some(before) = autoxide_kernel::rows::lock_one(
            &transaction,
            booking::Entity::find_by_id(id).scoped(tenant),
        )
        .await?
        else {
            return Ok(());
        };
        if !matches!(before.deposit_status.as_str(), "pending" | "none")
            || before.deposit_minor <= 0
        {
            return Ok(());
        }
        let deposit = Money::from_columns(before.deposit_minor, &before.currency)?;
        if received.is_some_and(|money| money.currency() != deposit.currency()) {
            // Lượt thu `bookings` tự tạo luôn cùng loại tiền với cọc: khác thì không tự xác nhận
            // hay hoàn theo số đó — để người xử lý ở `ledger`.
            tracing::error!(booking_id = %id, "deposit payment in another currency; left for review");
            return Ok(());
        }
        let covered = received.is_none_or(|money| money.compare(deposit).is_ok_and(|o| o.is_ge()));
        let mut after = before.clone();
        after.payment_id = Some(payment_id);
        after.updated_at = now();
        if before.status == HELD && covered {
            after.status = CONFIRMED.to_owned();
            after.deposit_status = "paid".to_owned();
            after.hold_expires_at = None;
        } else {
            if before.status == HELD {
                tracing::warn!(booking_id = %id, "deposit payment does not cover the deposit");
            }
            after.deposit_status = "refund_pending".to_owned();
            // Hoàn đúng số đã thu (thu thiếu hay thừa), không phải số cọc.
            after.refund_minor = received.map_or(before.deposit_minor, Money::minor);
        }
        let active: booking::ActiveModel =
            sea_orm::IntoActiveModel::into_active_model(after).reset_all();
        let saved = booking::Entity::update(active)
            .validate()?
            .filter(booking::Column::DepositStatus.eq(before.deposit_status.clone()))
            .scoped(tenant)
            .exec(&transaction)
            .await
            .map_err(|error| match error {
                sea_orm::DbErr::RecordNotUpdated => crate::actions::invalid_status("changed"),
                error => Error::from(error),
            })?;
        if saved.status == CONFIRMED && before.status == HELD {
            let event = events::BookingConfirmed(Change {
                before: Some(snapshot(&before)),
                after: Some(snapshot(&saved)),
            });
            self.outbox
                .record(&transaction, tenant, Actor::system(), events::booking_subject(id), &event)
                .await?;
        }
        self.queue_refund(&transaction, tenant, &saved).await?;
        transaction.commit().await?;
        Ok(())
    }
}

struct ExpireHolds(Arc<BookingsService>);

#[async_trait::async_trait]
impl JobHandler for ExpireHolds {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.expire_due_holds(&job.database).await.map(|_| ())
    }
}

struct Refund(Arc<BookingsService>);

#[async_trait::async_trait]
impl JobHandler for Refund {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let payload: RefundDeposit =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        let tenant =
            job.tenant.as_ref().ok_or_else(|| Error::internal("refund job without a tenant"))?;
        self.0.refund_deposit(tenant, payload.booking_id).await
    }
}

/// Nghe `ledger.payment_succeeded` của lượt thu cọc (`source_module = bookings`).
pub(crate) struct PaymentListener(pub Arc<BookingsService>);

#[async_trait::async_trait]
impl EventHandler for PaymentListener {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        if event.payload["source_module"] != "bookings" {
            return Ok(());
        }
        let parse = |key: &str| event.payload[key].as_str().and_then(|id| id.parse::<Uuid>().ok());
        let (Some(id), Some(payment)) = (parse("source_id"), parse("payment_id")) else {
            return Ok(());
        };
        // Số thực thu (`ledger` ghi cả khoản thiếu khi đối soát `accept_received`). Event không mang
        // số (bản cũ) thì coi như thu đủ cọc — như trước.
        let received = match event.payload.get("amount") {
            Some(amount) if !amount.is_null() => {
                Some(serde_json::from_value::<Money>(amount.clone()).map_err(Error::internal)?)
            }
            _ => None,
        };
        self.0.deposit_paid(&event.tenant, id, payment, received).await
    }
}
