//! Việc trên một lịch đã đặt: xác nhận, check-in, xong, vắng mặt, huỷ, đổi giờ — theo chính sách
//! của nút (D-20261118-step60-bookings mục 8). Mỗi việc là MỘT câu `UPDATE … WHERE status = <cũ>`:
//! hai người bấm cùng lúc thì một bên được, bên kia nhận `bookings.invalid_status`.
//!
//! Hoàn cọc không gọi `ledger` trong transaction: ghi `refund_pending` + job `bookings.refund_deposit`
//! cùng transaction; job gọi `dyn Ledger::refund` với `reference` cố định.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Event};
use autoxide_kernel::job::EnqueueOptions;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, Set,
};
use uuid::Uuid;

use crate::booking::{
    Booking, CANCELLED, CHECKED_IN, COMPLETED, CONFIRMED, HELD, NO_SHOW, PENDING,
    booking_not_found, check_start, release, reserve, snapshot,
};
use crate::entity::{booking, resource};
use crate::events::{self, Change};
use crate::jobs::RefundDeposit;
use crate::openings::shape_of;
use crate::permissions::{BOOKINGS_MANAGE, BOOKINGS_READ, BOOKINGS_STAFF};
use crate::service::{BookingsService, at, check_text, finish, now};

/// Ai đang làm việc trên lịch.
#[derive(Clone, Copy)]
pub(crate) enum Caller<'a> {
    /// Người đăng nhập: cửa hàng (`bookings.manage`), nhân viên của lịch (`bookings.staff`), hay
    /// chính khách có tài khoản.
    Principal(&'a Principal),
    /// Khách vãng lai đã xác nhận số điện thoại (E.164) bằng mã một lần.
    Guest(&'a str),
}

impl Caller<'_> {
    fn actor(self) -> Actor {
        match self {
            Caller::Principal(principal) => Actor::from(principal),
            Caller::Guest(_) => Actor::public(),
        }
    }
}

/// Vai của người gọi đối với một lịch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Shop,
    /// Chỉ xem (`bookings.read`).
    Viewer,
    Staff,
    Customer,
}

/// Việc cần làm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Confirm,
    CheckIn,
    Complete,
    NoShow,
}

pub(crate) fn invalid_status(status: &str) -> Error {
    Error::conflict(
        "bookings.invalid_status",
        "The booking is `{status}`; this cannot be done now.",
    )
    .with_param("status", status)
}

fn too_late_to_cancel() -> Error {
    Error::conflict(
        "bookings.too_late_to_cancel",
        "The booking has started; it can no longer be cancelled. Contact the shop.",
    )
}

fn too_late_to_change() -> Error {
    Error::conflict(
        "bookings.too_late_to_change",
        "Bookings can be moved only until {hours} hours before the start. Contact the shop.",
    )
}

fn not_started() -> Error {
    Error::conflict(
        "bookings.not_started",
        "The booking has not started yet; mark a no-show after the start time.",
    )
}

fn forbidden() -> Error {
    Error::forbidden(
        "bookings.not_allowed",
        "Only the shop (permission `bookings.manage`) can do this to the booking.",
    )
}

/// Phần cọc hoàn lại khi giữ `retain_percent` phần trăm.
fn refund_of(deposit: i64, retain_percent: i64) -> i64 {
    deposit - deposit.saturating_mul(retain_percent.clamp(0, 100)) / 100
}

impl BookingsService {
    /// Vai của `caller` với lịch `row`; không có vai nào → báo như không có lịch.
    pub(crate) async fn role<C: ConnectionTrait>(
        connection: &C,
        caller: Caller<'_>,
        row: &Booking,
    ) -> Result<Role, Error> {
        match caller {
            Caller::Guest(phone) => {
                if row.customer_user_id.is_none() && row.customer_phone.as_deref() == Some(phone) {
                    return Ok(Role::Customer);
                }
            }
            Caller::Principal(principal) => {
                if principal.require_at(connection, &BOOKINGS_MANAGE, at(row.org_id)).await.is_ok()
                {
                    return Ok(Role::Shop);
                }
                if principal.require_at(connection, &BOOKINGS_READ, at(row.org_id)).await.is_ok() {
                    return Ok(Role::Viewer);
                }
                if principal.require_at(connection, &BOOKINGS_STAFF, at(row.org_id)).await.is_ok() {
                    let mine = resource::Entity::find_by_id(row.resource_id)
                        .one(connection)
                        .await?
                        .is_some_and(|found| {
                            found.tenant_id == row.tenant_id
                                && found.user_id == Some(principal.user_id())
                        });
                    if mine {
                        return Ok(Role::Staff);
                    }
                }
                if row.customer_user_id == Some(principal.user_id()) {
                    return Ok(Role::Customer);
                }
            }
        }
        Err(booking_not_found(row.id))
    }

    pub(crate) async fn booking_row<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Booking, Error> {
        booking::Entity::find_by_id(id)
            .scoped(tenant)
            .one(connection)
            .await?
            .ok_or_else(|| booking_not_found(id))
    }

    /// Đọc và khoá lịch `id` tới hết `transaction` ghi (D-181): mọi đường đổi lịch (huỷ, đổi giờ,
    /// xác nhận…) đọc qua đây, nên hai thay đổi cùng lúc nối đuôi nhau trên Postgres — bên sau thấy
    /// giờ / chỗ bên trước vừa ghi, không cùng trả chỗ cũ hai lần. SQLite vốn một bên ghi mỗi lúc.
    pub(crate) async fn lock_booking(
        transaction: &DatabaseTransaction,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Booking, Error> {
        autoxide_kernel::rows::lock_one(transaction, booking::Entity::find_by_id(id).scoped(tenant))
            .await?
            .ok_or_else(|| booking_not_found(id))
    }

    /// Lịch `id` nếu `caller` có vai với nó.
    pub(crate) async fn get_booking(
        &self,
        tenant: &TenantContext,
        caller: Caller<'_>,
        id: Uuid,
    ) -> Result<Booking, Error> {
        let read = tenant.database().read();
        let row = Self::booking_row(read, tenant, id).await?;
        Self::role(read, caller, &row).await?;
        Ok(row)
    }

    /// Ghi trạng thái mới của lịch nếu trạng thái vẫn là `before.status`; ghi event `E`.
    async fn save<E: Event>(
        &self,
        transaction: &DatabaseTransaction,
        tenant: &TenantContext,
        caller: Caller<'_>,
        before: &Booking,
        after: Booking,
        event: fn(Change) -> E,
    ) -> Result<Booking, Error> {
        // `reset_all`: mọi cột thành `Set` — `into_active_model` để chúng `Unchanged` và câu UPDATE bỏ qua.
        let mut active = after.into_active_model().reset_all();
        active.updated_at = Set(now());
        let saved = booking::Entity::update(active)
            .validate()?
            .filter(booking::Column::Status.eq(before.status.clone()))
            .scoped(tenant)
            .exec(transaction)
            .await
            .map_err(|error| match error {
                sea_orm::DbErr::RecordNotUpdated => invalid_status("changed"),
                error => Error::from(error),
            })?;
        let change = Change { before: Some(snapshot(before)), after: Some(snapshot(&saved)) };
        self.outbox
            .record(
                transaction,
                tenant,
                caller.actor(),
                events::booking_subject(before.id),
                &event(change),
            )
            .await?;
        Ok(saved)
    }

    /// Xác nhận, check-in, xong, vắng mặt.
    pub(crate) async fn act(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        action: Action,
    ) -> Result<Booking, Error> {
        // Kiểm permission trước khi tra lịch: người không có quyền nhận 403, không phải 404.
        match action {
            Action::Confirm => principal.require(&BOOKINGS_MANAGE)?,
            _ if principal.has(&BOOKINGS_STAFF) => {}
            _ => principal.require(&BOOKINGS_MANAGE)?,
        }
        let caller = Caller::Principal(principal);
        let row = Self::booking_row(tenant.database().read(), tenant, id).await?;
        let policy = self.settings.policy(tenant, OrgId::from_uuid(row.org_id)).await?;
        let transaction = tenant.database().begin_write().await?;
        let before = Self::lock_booking(&transaction, tenant, id).await?;
        let role = Self::role(&transaction, caller, &before).await?;
        let allowed = match action {
            Action::Confirm => role == Role::Shop,
            Action::CheckIn | Action::Complete | Action::NoShow => {
                matches!(role, Role::Shop | Role::Staff)
            }
        };
        if !allowed {
            return Err(forbidden());
        }
        let permission = if role == Role::Shop { &BOOKINGS_MANAGE } else { &BOOKINGS_STAFF };
        principal.require_write_at(&transaction, permission, at(before.org_id)).await?;
        let from: &[&str] = match action {
            Action::Confirm => &[PENDING],
            Action::CheckIn | Action::NoShow => &[CONFIRMED],
            Action::Complete => &[CONFIRMED, CHECKED_IN],
        };
        if !from.contains(&before.status.as_str()) {
            return Err(invalid_status(&before.status));
        }
        let mut after = before.clone();
        let saved = match action {
            Action::Confirm => {
                after.status = CONFIRMED.to_owned();
                self.save(&transaction, tenant, caller, &before, after, events::BookingConfirmed)
                    .await?
            }
            Action::CheckIn => {
                after.status = CHECKED_IN.to_owned();
                self.save(&transaction, tenant, caller, &before, after, events::BookingCheckedIn)
                    .await?
            }
            Action::Complete => {
                after.status = COMPLETED.to_owned();
                self.save(&transaction, tenant, caller, &before, after, events::BookingCompleted)
                    .await?
            }
            Action::NoShow => {
                if now() < before.starts_at {
                    return Err(not_started());
                }
                after.status = NO_SHOW.to_owned();
                self.settle_deposit(&mut after, policy.no_show_retain_percent);
                let saved = self
                    .save(&transaction, tenant, caller, &before, after, events::BookingNoShow)
                    .await?;
                self.queue_refund(&transaction, tenant, &saved).await?;
                saved
            }
        };
        transaction.commit().await?;
        Ok(saved)
    }

    /// Cọc đã thu: giữ `retain_percent`, phần còn lại chờ hoàn. Chưa thu: không còn gì để thu.
    fn settle_deposit(&self, row: &mut Booking, retain_percent: i64) {
        match row.deposit_status.as_str() {
            "paid" => {
                let refund = refund_of(row.deposit_minor, retain_percent);
                row.refund_minor = refund;
                row.deposit_status =
                    if refund > 0 { "refund_pending" } else { "retained" }.to_owned();
            }
            "pending" => row.deposit_status = "none".to_owned(),
            _ => {}
        }
    }

    /// Có tiền chờ hoàn: job hoàn cọc, cùng transaction với thay đổi.
    pub(crate) async fn queue_refund(
        &self,
        transaction: &DatabaseTransaction,
        tenant: &TenantContext,
        row: &Booking,
    ) -> Result<(), Error> {
        if row.deposit_status == "refund_pending" {
            let options = EnqueueOptions::default().unique_key(format!("refund:{}", row.id));
            self.jobs
                .enqueue(transaction, tenant, &RefundDeposit { booking_id: row.id }, options)
                .await?;
        }
        Ok(())
    }

    /// Huỷ lịch. Cửa hàng: lúc nào cũng được, hoàn đủ cọc. Khách: tới giờ bắt đầu; trong hạn miễn
    /// phí hoàn đủ, sau hạn giữ phần trăm theo chính sách.
    pub(crate) async fn cancel(
        &self,
        tenant: &TenantContext,
        caller: Caller<'_>,
        id: Uuid,
        reason: Option<String>,
    ) -> Result<Booking, Error> {
        let mut problems = Vec::new();
        if let Some(reason) = &reason {
            check_text(&mut problems, "reason", reason, 0, 200);
        }
        finish(problems)?;
        let row = Self::booking_row(tenant.database().read(), tenant, id).await?;
        let policy = self.settings.policy(tenant, OrgId::from_uuid(row.org_id)).await?;
        let transaction = tenant.database().begin_write().await?;
        let before = Self::lock_booking(&transaction, tenant, id).await?;
        let role = Self::role(&transaction, caller, &before).await?;
        if matches!(role, Role::Staff | Role::Viewer) {
            return Err(forbidden());
        }
        if let (Caller::Principal(principal), Role::Shop) = (caller, role) {
            principal.require_write_at(&transaction, &BOOKINGS_MANAGE, at(before.org_id)).await?;
        }
        if ![HELD, PENDING, CONFIRMED].contains(&before.status.as_str()) {
            return Err(invalid_status(&before.status));
        }
        let at_now = now();
        let retain = if role == Role::Shop {
            0
        } else if at_now >= before.starts_at {
            return Err(too_late_to_cancel());
        } else if at_now <= before.starts_at - SignedDuration::from_hours(policy.free_cancel_hours)
        {
            0
        } else {
            policy.late_cancel_retain_percent
        };
        let mut after = before.clone();
        after.status = CANCELLED.to_owned();
        after.cancelled_at = Some(at_now);
        after.cancel_reason =
            reason.map(|reason| reason.trim().to_owned()).filter(|reason| !reason.is_empty());
        after.hold_expires_at = None;
        self.settle_deposit(&mut after, retain);
        let saved = self
            .save(&transaction, tenant, caller, &before, after, events::BookingCancelled)
            .await?;
        release(&transaction, tenant, &before).await?;
        self.queue_refund(&transaction, tenant, &saved).await?;
        transaction.commit().await?;
        tracing::info!(booking_id = %id, refund_minor = saved.refund_minor, "booking cancelled");
        Ok(saved)
    }

    /// Đổi giờ (và tuỳ chọn nhân viên). Cửa hàng: lúc nào cũng được; khách: trong hạn huỷ miễn phí.
    /// Giờ mới đã có người → `slot_taken`, lịch giữ nguyên giờ cũ.
    pub(crate) async fn reschedule(
        &self,
        tenant: &TenantContext,
        caller: Caller<'_>,
        id: Uuid,
        starts_at: Timestamp,
        resource_id: Option<Uuid>,
    ) -> Result<Booking, Error> {
        let mut problems: Vec<FieldError> = Vec::new();
        check_start(&mut problems, "starts_at", starts_at);
        finish(problems)?;
        let read = tenant.database().read();
        let row = Self::booking_row(read, tenant, id).await?;
        let role = Self::role(read, caller, &row).await?;
        if matches!(role, Role::Staff | Role::Viewer) {
            return Err(forbidden());
        }
        let policy = self.settings.policy(tenant, OrgId::from_uuid(row.org_id)).await?;
        if role == Role::Customer
            && now() > row.starts_at - SignedDuration::from_hours(policy.free_cancel_hours)
        {
            return Err(too_late_to_change().with_param("hours", policy.free_cancel_hours));
        }
        let service = Self::service_view(read, tenant, row.service_id).await?;
        let wanted = resource_id.unwrap_or(row.resource_id);
        let candidates = self.candidates(read, tenant, &service, Some(wanted), starts_at).await?;
        let performer = candidates.first().ok_or_else(crate::booking::outside_hours)?;
        let transaction = tenant.database().begin_write().await?;
        let before = Self::lock_booking(&transaction, tenant, id).await?;
        if let (Caller::Principal(principal), Role::Shop) = (caller, role) {
            principal.require_write_at(&transaction, &BOOKINGS_MANAGE, at(before.org_id)).await?;
            principal
                .require_write_at(&transaction, &BOOKINGS_MANAGE, at(performer.org_id))
                .await?;
        }
        if ![PENDING, CONFIRMED].contains(&before.status.as_str()) {
            return Err(invalid_status(&before.status));
        }
        // Trả chỗ cũ rồi giữ chỗ mới trong cùng transaction: giờ mới bị chiếm thì cả hai cùng huỷ.
        release(&transaction, tenant, &before).await?;
        let session_id =
            reserve(&transaction, tenant, &service, performer.id, starts_at, before.seats, id)
                .await?;
        let mut after = before.clone();
        after.starts_at = starts_at;
        after.ends_at = shape_of(&service).service_span(starts_at).end;
        after.resource_id = performer.id;
        after.org_id = performer.org_id;
        after.session_id = session_id;
        after.reminded_24h_at = None;
        after.reminded_2h_at = None;
        let saved = self
            .save(&transaction, tenant, caller, &before, after, events::BookingRescheduled)
            .await?;
        transaction.commit().await?;
        Ok(saved)
    }
}

#[cfg(test)]
mod tests {
    use super::refund_of;

    #[test]
    fn phan_coc_hoan_theo_phan_tram_giu() {
        assert_eq!(refund_of(100_000, 100), 0);
        assert_eq!(refund_of(100_000, 50), 50_000);
        assert_eq!(refund_of(100_000, 0), 100_000);
        assert_eq!(refund_of(99_999, 50), 50_000);
    }
}
