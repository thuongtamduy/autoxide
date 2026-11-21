//! Đặt chỗ: không bao giờ chồng giờ (D-20261118-step60-bookings mục 1, 6, 7).
//!
//! - Sức chứa 1: chèn mọi ô 5 phút của khoảng chiếm (gồm đệm) cho nhân viên và mọi phòng / thiết
//!   bị cần, trong MỘT transaction; trùng khoá duy nhất `(tenant_id, resource_id, slot_start)` →
//!   `bookings.slot_taken`. Hai database chặn như nhau, kể cả khi nhiều người đặt cùng lúc.
//! - Lớp học sức chứa N: người đầu tiên mở buổi (`bookings_sessions`, chiếm ô giờ một lần); chỗ
//!   ngồi đếm bằng `UPDATE … SET booked = booked + n WHERE booked + n <= capacity` → 0 dòng là
//!   `bookings.capacity_full`.
//! - Có cọc: `held` tới khi `ledger` báo đã thu; lượt thu tạo SAU khi transaction ghi xong (không
//!   gọi mạng trong transaction).

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter,
    Set, SqlErr,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::availability::{Plan, Shape, Span, cells};
use crate::entity::{booking, resource, session, slot};
use crate::events::{self, Change};
use crate::openings::shape_of;
use crate::permissions::BOOKINGS_MANAGE;
use crate::service::{BookingsService, at, check_text, finish, now};
use crate::setup::{ServiceView, service_not_found};

pub(crate) const HELD: &str = "held";
pub(crate) const PENDING: &str = "pending";
pub(crate) const CONFIRMED: &str = "confirmed";
pub(crate) const CHECKED_IN: &str = "checked_in";
pub(crate) const COMPLETED: &str = "completed";
pub(crate) const CANCELLED: &str = "cancelled";
pub(crate) const NO_SHOW: &str = "no_show";
pub(crate) const EXPIRED: &str = "expired";

/// Mã nước mặc định khi khách gõ số bắt đầu bằng `0`.
pub(crate) const DEFAULT_COUNTRY: &str = "84";

pub(crate) const STATUSES: [&str; 8] =
    [HELD, PENDING, CONFIRMED, CHECKED_IN, COMPLETED, CANCELLED, NO_SHOW, EXPIRED];

/// Lịch đặt.
pub(crate) type Booking = booking::Model;

/// Ai đặt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    /// Khách tự đặt (có tài khoản hay vãng lai đã xác nhận số điện thoại): có cọc thì giữ tạm.
    Customer,
    /// Cửa hàng đặt hộ (`bookings.manage`): xác nhận ngay, không giữ tạm, không thu cọc (mục 9).
    Shop,
}

/// Khách của lịch.
#[derive(Debug, Clone, Default)]
pub(crate) struct Customer {
    pub user_id: Option<Uuid>,
    pub name: String,
    /// E.164.
    pub phone: Option<String>,
    pub email: Option<String>,
}

/// Một lịch mới.
#[derive(Debug, Clone)]
pub(crate) struct NewBooking {
    pub service_id: Uuid,
    /// `None`: nhân viên bất kỳ còn trống (mục 3).
    pub resource_id: Option<Uuid>,
    pub starts_at: Timestamp,
    pub seats: i32,
    pub customer: Customer,
    pub note: String,
    pub locale: Option<String>,
    /// Cách trả cọc (`card`, `wallet`, `vietqr`…).
    pub payment_method: String,
    /// Trang của cửa hàng mở lại sau khi trả cọc.
    pub return_url: Option<String>,
}

/// Lịch vừa đặt và (khi có cọc) lượt thu để khách trả.
#[derive(Debug, Clone)]
pub(crate) struct Booked {
    pub booking: Booking,
    pub payment: Option<autoxide_ledger::api::Payment>,
}

pub(crate) fn booking_not_found(id: Uuid) -> Error {
    Error::not_found("bookings.booking_not_found", "There is no booking `{id}`.")
        .with_param("id", id)
}

pub(crate) fn slot_taken() -> Error {
    Error::conflict(
        "bookings.slot_taken",
        "This time was just taken by someone else. Ask for the free times again and choose another.",
    )
}

pub(crate) fn outside_hours() -> Error {
    Error::new(
        ErrorKind::Validation,
        "bookings.outside_hours",
        "This time is outside the working hours of the staff member (or room). Choose a free time from the availability.",
    )
}

fn capacity_full() -> Error {
    Error::conflict("bookings.capacity_full", "This class is full. Choose another time.")
}

fn payments_unavailable() -> Error {
    Error::unavailable(
        "bookings.payments_unavailable",
        "This service asks for a deposit, but online payments are not set up. Contact the shop.",
    )
}

fn resource_not_offered(id: Uuid) -> Error {
    Error::new(
        ErrorKind::Validation,
        "bookings.resource_not_offered",
        "The resource `{id}` does not give this service.",
    )
    .with_param("id", id)
    .with_field(FieldError::new("resource_id", "invalid", "does not give this service"))
}

/// Vi phạm khoá duy nhất → `slot_taken`; lỗi khác giữ nguyên.
pub(crate) fn taken(error: sea_orm::DbErr) -> Error {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => slot_taken(),
        _ => Error::from(error),
    }
}

/// Những gì audit cần biết về một lịch (không gồm số điện thoại, email).
pub(crate) fn snapshot(row: &Booking) -> Value {
    json!({
        "status": row.status, "service_id": row.service_id, "resource_id": row.resource_id,
        "starts_at": row.starts_at, "ends_at": row.ends_at, "seats": row.seats,
        "customer_user_id": row.customer_user_id, "deposit_minor": row.deposit_minor,
        "currency": row.currency, "deposit_status": row.deposit_status, "refund_minor": row.refund_minor,
    })
}

/// Kiểm đầu vào của khách, gom mọi lỗi (D-083); chuẩn hoá số điện thoại về E.164.
pub(crate) fn check_customer(problems: &mut Vec<FieldError>, customer: &mut Customer, note: &str) {
    check_text(problems, "customer.name", &customer.name, 1, 200);
    if let Some(raw) = customer.phone.take() {
        match PhoneNumber::parse(&raw, DEFAULT_COUNTRY) {
            Some(phone) => customer.phone = Some(phone.as_str().to_owned()),
            None => {
                problems.push(FieldError::new("customer.phone", "invalid", "is not a phone number"))
            }
        }
    }
    customer.email = customer
        .email
        .take()
        .map(|email| email.trim().to_owned())
        .filter(|email| !email.is_empty());
    if customer.email.as_ref().is_some_and(|email| email.len() > 254 || !email.contains('@')) {
        problems.push(FieldError::new("customer.email", "invalid", "is not an email address"));
    }
    if customer.user_id.is_none() && customer.phone.is_none() && customer.email.is_none() {
        problems.push(FieldError::new(
            "customer.phone",
            "required",
            "give a phone number or an email address",
        ));
    }
    check_text(problems, "note", note, 0, 500);
}

/// Giờ bắt đầu phải nằm trên lưới 5 phút.
pub(crate) fn check_start(problems: &mut Vec<FieldError>, field: &str, start: Timestamp) {
    // Đúng phút chia hết cho 5, không lẻ giây (giờ UTC và giờ địa phương cùng lưới 5 phút).
    let on_grid =
        start.as_unix_seconds().rem_euclid(300) == 0 && start.to_jiff().subsec_nanosecond() == 0;
    if !on_grid {
        problems.push(FieldError::new(
            field,
            "invalid",
            "must be on a 5-minute mark, e.g. 09:00 or 09:05",
        ));
    }
    if start <= Timestamp::now() {
        problems.push(FieldError::new(field, "range", "must be in the future"));
    }
}

/// Giữ ô giờ của một lịch trong `transaction`: sức chứa 1 → ô giờ của lịch; lớp học → chỗ trong buổi
/// (mở buổi nếu chưa có). Trả ID buổi (lớp học).
pub(crate) async fn reserve(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    service: &ServiceView,
    resource_id: Uuid,
    start: Timestamp,
    seats: i32,
    booking_id: Uuid,
) -> Result<Option<Uuid>, Error> {
    let shape = shape_of(service);
    let occupied = shape.occupied_span(start);
    let owners: Vec<Uuid> =
        std::iter::once(resource_id).chain(service.required.iter().copied()).collect();
    if service.row.capacity <= 1 {
        insert_cells(transaction, tenant, &owners, occupied, Some(booking_id), None).await?;
        return Ok(None);
    }
    let found = session::Entity::find()
        .filter(session::Column::ServiceId.eq(service.row.id))
        .filter(session::Column::ResourceId.eq(resource_id))
        .filter(session::Column::StartsAt.eq(start))
        .scoped(tenant)
        .one(transaction)
        .await?;
    let session_id = match found {
        Some(found) => found.id,
        None => {
            let id = Uuid::now_v7();
            session::ActiveModel {
                id: Set(id),
                tenant_id: Set(tenant.id().as_uuid()),
                service_id: Set(service.row.id),
                resource_id: Set(resource_id),
                starts_at: Set(start),
                ends_at: Set(shape.service_span(start).end),
                capacity: Set(service.row.capacity),
                booked: Set(0),
            }
            .insert(transaction)
            .await
            .map_err(taken)?;
            insert_cells(transaction, tenant, &owners, occupied, None, Some(id)).await?;
            id
        }
    };
    // Đếm chỗ bằng câu lệnh có điều kiện: hai người cùng lấy chỗ cuối thì một bên được 0 dòng.
    let updated = session::Entity::update_many()
        .col_expr(session::Column::Booked, Expr::col(session::Column::Booked).add(seats))
        .filter(session::Column::Id.eq(session_id))
        .filter(
            Expr::col(session::Column::Booked).add(seats).lte(Expr::col(session::Column::Capacity)),
        )
        .scoped(tenant)
        .exec(transaction)
        .await?;
    if updated.rows_affected == 0 {
        return Err(capacity_full());
    }
    Ok(Some(session_id))
}

async fn insert_cells(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    owners: &[Uuid],
    span: Span,
    booking_id: Option<Uuid>,
    session_id: Option<Uuid>,
) -> Result<(), Error> {
    let starts = cells(span);
    let rows: Vec<slot::ActiveModel> = owners
        .iter()
        .flat_map(|owner| starts.iter().map(move |start| (*owner, *start)))
        .map(|(owner, start)| slot::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            resource_id: Set(owner),
            slot_start: Set(start),
            booking_id: Set(booking_id),
            session_id: Set(session_id),
        })
        .collect();
    if rows.is_empty() {
        return Ok(());
    }
    slot::Entity::insert_many(rows).exec(transaction).await.map_err(taken)?;
    Ok(())
}

/// Trả chỗ của `row` trong `transaction`: xoá ô giờ của lịch, hay bớt chỗ trong buổi (buổi hết
/// người thì xoá buổi và ô giờ của nó).
pub(crate) async fn release(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    row: &Booking,
) -> Result<(), Error> {
    let Some(session_id) = row.session_id else {
        slot::Entity::delete_many()
            .filter(slot::Column::BookingId.eq(row.id))
            .scoped(tenant)
            .exec(transaction)
            .await?;
        return Ok(());
    };
    session::Entity::update_many()
        .col_expr(session::Column::Booked, Expr::col(session::Column::Booked).sub(row.seats))
        .filter(session::Column::Id.eq(session_id))
        .scoped(tenant)
        .exec(transaction)
        .await?;
    let emptied = session::Entity::delete_many()
        .filter(session::Column::Id.eq(session_id))
        .filter(session::Column::Booked.lte(0))
        .scoped(tenant)
        .exec(transaction)
        .await?;
    if emptied.rows_affected > 0 {
        slot::Entity::delete_many()
            .filter(slot::Column::SessionId.eq(session_id))
            .scoped(tenant)
            .exec(transaction)
            .await?;
    }
    Ok(())
}

impl BookingsService {
    /// Nhân viên có thể nhận lịch lúc `start`: chỉ định (phải làm được dịch vụ) hay mọi người, theo
    /// thứ tự ID, chỉ người trong giờ làm (cả phòng / thiết bị cần). Không ai → `outside_hours`.
    pub(crate) async fn candidates<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        service: &ServiceView,
        resource_id: Option<Uuid>,
        start: Timestamp,
    ) -> Result<Vec<resource::Model>, Error> {
        if let Some(wanted) = resource_id
            && !service.performers.contains(&wanted)
        {
            return Err(resource_not_offered(wanted));
        }
        let shape: Shape = shape_of(service);
        let span = shape.occupied_span(start);
        let loaded = Self::load(
            connection,
            tenant,
            service,
            None,
            resource_id,
            span.start - SignedDuration::from_hours(24),
            span.end,
        )
        .await?;
        let zone_of =
            |org: Uuid| async move { self.settings.timezone(tenant, OrgId::from_uuid(org)).await };
        let rows = resource::Entity::find()
            .filter(
                resource::Column::Id.is_in(loaded.performers.iter().map(|plan| plan.resource_id)),
            )
            .scoped(tenant)
            .all(connection)
            .await?;
        let mut found = Vec::new();
        for plan in &loaded.performers {
            let Some(row) = rows.iter().find(|row| row.id == plan.resource_id) else { continue };
            let zone = zone_of(row.org_id).await?;
            let rooms_ok =
                loaded.required.iter().all(|room: &Plan| room.within_hours(&zone, shape, start));
            if plan.within_hours(&zone, shape, start) && rooms_ok {
                found.push(row.clone());
            }
        }
        if found.is_empty() {
            return Err(outside_hours());
        }
        Ok(found)
    }

    /// Đặt một lịch. Khách: có cọc → `held` + lượt thu; không cọc → `confirmed` (hay `pending` khi
    /// nút đòi duyệt). Cửa hàng: `confirmed`, không cọc.
    pub(crate) async fn book(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        origin: Origin,
        mut input: NewBooking,
    ) -> Result<Booked, Error> {
        let mut problems = Vec::new();
        check_customer(&mut problems, &mut input.customer, &input.note);
        check_start(&mut problems, "starts_at", input.starts_at);
        let read = tenant.database().read();
        let service = Self::service_view(read, tenant, input.service_id).await?;
        if !service.row.active {
            return Err(service_not_found(input.service_id));
        }
        if input.seats < 1 || input.seats > service.row.capacity {
            problems.push(
                FieldError::new("seats", "range", "must be 1 to {capacity}")
                    .with_param("capacity", service.row.capacity),
            );
        }
        finish(problems)?;
        let candidates =
            self.candidates(read, tenant, &service, input.resource_id, input.starts_at).await?;
        let price = service.price()?;
        let total = price.checked_mul(i64::from(input.seats))?;
        let deposit = match origin {
            Origin::Shop => 0,
            Origin::Customer => service.deposit().of(total.minor()),
        };
        if deposit > 0 && self.ledger.is_none() {
            return Err(payments_unavailable());
        }
        let actor = match (origin, principal) {
            (_, Some(principal)) => Actor::from(principal),
            (_, None) => Actor::public(),
        };
        // Thử từng nhân viên trong giờ làm; người đầu tiên còn trống nhận lịch.
        let mut last = slot_taken();
        for candidate in &candidates {
            let policy = self.settings.policy(tenant, OrgId::from_uuid(candidate.org_id)).await?;
            let status = match origin {
                Origin::Shop => CONFIRMED,
                Origin::Customer if deposit > 0 => HELD,
                Origin::Customer if policy.require_approval => PENDING,
                Origin::Customer => CONFIRMED,
            };
            let hold =
                (status == HELD).then(|| now() + SignedDuration::from_mins(policy.hold_minutes));
            let mut attempt = self
                .insert(
                    tenant, principal, origin, &service, candidate, &input, status, total, deposit,
                    hold, actor,
                )
                .await;
            if service.row.capacity > 1
                && attempt.as_ref().is_err_and(|error| error.code() == "bookings.slot_taken")
            {
                // Lớp học: người khác vừa mở cùng buổi (vướng unique khi mở) — lần này thấy buổi đã
                // có và lấy chỗ trong đó, thay vì báo `slot_taken` khi lớp còn chỗ.
                attempt = self
                    .insert(
                        tenant, principal, origin, &service, candidate, &input, status, total,
                        deposit, hold, actor,
                    )
                    .await;
            }
            match attempt {
                Ok(row) => return self.collect_deposit(tenant, actor, &service, row, &input).await,
                Err(error) if error.code() == "bookings.slot_taken" => last = error,
                Err(error) => return Err(error),
            }
        }
        Err(last)
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        origin: Origin,
        service: &ServiceView,
        performer: &resource::Model,
        input: &NewBooking,
        status: &str,
        total: Money,
        deposit: i64,
        hold: Option<Timestamp>,
        actor: Actor,
    ) -> Result<Booking, Error> {
        let id = Uuid::now_v7();
        let transaction = tenant.database().begin_write().await?;
        if origin == Origin::Shop {
            let principal =
                principal.ok_or_else(|| Error::internal("a shop booking needs a principal"))?;
            principal
                .require_write_at(&transaction, &BOOKINGS_MANAGE, at(performer.org_id))
                .await?;
        }
        let session_id =
            reserve(&transaction, tenant, service, performer.id, input.starts_at, input.seats, id)
                .await?;
        let at_now = now();
        let row = booking::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(performer.org_id),
            service_id: Set(service.row.id),
            resource_id: Set(performer.id),
            session_id: Set(session_id),
            seats: Set(input.seats),
            starts_at: Set(input.starts_at),
            ends_at: Set(shape_of(service).service_span(input.starts_at).end),
            status: Set(status.to_owned()),
            customer_user_id: Set(input.customer.user_id),
            customer_name: Set(input.customer.name.trim().to_owned()),
            customer_phone: Set(input.customer.phone.clone()),
            customer_email: Set(input.customer.email.clone()),
            note: Set(input.note.trim().to_owned()),
            locale: Set(input.locale.clone()),
            price_minor: Set(total.minor()),
            currency: Set(total.currency().code().to_owned()),
            deposit_minor: Set(deposit),
            deposit_status: Set(if deposit > 0 { "pending" } else { "none" }.to_owned()),
            refund_minor: Set(0),
            payment_id: Set(None),
            hold_expires_at: Set(hold),
            reminded_24h_at: Set(None),
            reminded_2h_at: Set(None),
            cancelled_at: Set(None),
            cancel_reason: Set(None),
            created_by: Set(principal.map(Principal::user_id)),
            created_at: Set(at_now),
            updated_at: Set(at_now),
        }
        .insert(&transaction)
        .await?;
        let event = events::BookingCreated(Change { before: None, after: Some(snapshot(&row)) });
        self.outbox
            .record(&transaction, tenant, actor, events::booking_subject(id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(booking_id = %id, status, "booking created");
        Ok(row)
    }

    /// Lịch `held`: tạo lượt thu cọc qua `ledger` (NGOÀI transaction). Lỗi → trả chỗ ngay.
    async fn collect_deposit(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        service: &ServiceView,
        row: Booking,
        input: &NewBooking,
    ) -> Result<Booked, Error> {
        if row.status != HELD {
            return Ok(Booked { booking: row, payment: None });
        }
        let Some(ledger) = &self.ledger else {
            return Err(payments_unavailable());
        };
        let amount = Money::from_columns(row.deposit_minor, &row.currency)?;
        let description: String =
            format!("Deposit: {}", service.row.name).chars().take(140).collect();
        let mut payment = autoxide_ledger::api::NewPayment::new(
            OrgId::from_uuid(row.org_id),
            input.payment_method.clone(),
            amount,
            description,
            "bookings",
            deposit_reference(row.id),
        )
        .source_id(row.id.to_string());
        if let Some(url) = &input.return_url {
            payment = payment.return_url(url.clone());
        }
        match ledger.create_payment(tenant, actor, payment).await {
            Ok(created) => {
                let created = created.into_inner();
                let mut active: booking::ActiveModel = row.clone().into();
                active.payment_id = Set(Some(created.id));
                active.updated_at = Set(now());
                let transaction = tenant.database().begin_write().await?;
                let saved = booking::Entity::update(active)
                    .validate()?
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
                transaction.commit().await?;
                Ok(Booked { booking: saved, payment: Some(created) })
            }
            Err(error) => {
                // Không tạo được lượt thu: không giữ chỗ của người khác vô ích.
                if let Err(release_error) = self.expire(tenant, row.id).await {
                    tracing::warn!(booking_id = %row.id, error = %release_error, "cannot release a booking");
                }
                Err(error)
            }
        }
    }

    /// Lịch `held` `id` hết hạn giữ (hay không thu được cọc): `expired`, trả chỗ. Đã đổi trạng thái thì
    /// thôi (chạy lại được).
    pub(crate) async fn expire(&self, tenant: &TenantContext, id: Uuid) -> Result<bool, Error> {
        let transaction = tenant.database().begin_write().await?;
        let Some(before) = booking::Entity::find_by_id(id).scoped(tenant).one(&transaction).await?
        else {
            return Ok(false);
        };
        let updated = booking::Entity::update_many()
            .col_expr(booking::Column::Status, Expr::value(EXPIRED))
            .col_expr(booking::Column::UpdatedAt, Expr::value(now()))
            .filter(booking::Column::Id.eq(id))
            .filter(booking::Column::Status.eq(HELD))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if updated.rows_affected == 0 {
            return Ok(false);
        }
        release(&transaction, tenant, &before).await?;
        let mut after = before.clone();
        after.status = EXPIRED.to_owned();
        let event = events::BookingExpired(Change {
            before: Some(snapshot(&before)),
            after: Some(snapshot(&after)),
        });
        self.outbox
            .record(&transaction, tenant, Actor::system(), events::booking_subject(id), &event)
            .await?;
        transaction.commit().await?;
        tracing::info!(booking_id = %id, "held booking expired");
        Ok(true)
    }
}

/// Mã tham chiếu lượt thu cọc của lịch `id` (duy nhất theo khách và module nguồn).
pub(crate) fn deposit_reference(id: Uuid) -> String {
    format!("bookings.booking:{id}")
}

/// Mã tham chiếu lượt hoàn cọc của lịch `id`: cố định, gọi lại không hoàn hai lần.
pub(crate) fn refund_reference(id: Uuid) -> String {
    format!("bookings.booking:{id}:refund")
}
