//! Nhắc lịch qua `notifications` (tin giao dịch, D-20261118-step60-bookings mục 10): 24 giờ và 2 giờ
//! trước giờ bắt đầu. Gửi trước với khoá chống trùng, đánh dấu sau: job chạy lại (hay hai máy cùng
//! chạy) không gửi lần hai — `notifications` trả lại tin đã có cho cùng khoá.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler, JobRegistration, Schedule};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use autoxide_notifications::api::{
    Category, Channel, Notification, NotificationKind, NotificationTemplate, Notifier, Recipient,
};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde_json::json;
use uuid::Uuid;

use crate::booking::{Booking, CONFIRMED};
use crate::entity::{booking, service};
use crate::service::{BookingsService, now};

pub(crate) const SEND_REMINDERS: &str = "bookings.send_reminders";

/// Loại thông báo nhắc lịch.
pub(crate) const REMINDER: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::Email, Channel::Sms, Channel::Zns, Channel::InApp, Channel::MobilePush],
    NotificationTemplate::new(
        "bookings.reminder",
        "Reminder: {service} at {time}",
        "Your booking for {service} starts at {time}. To cancel or move it, open your booking.",
        "Reminder: {service} at {time}",
    ),
)
.sample(r#"{"service": "Haircut", "time": "09:00 23/11/2026", "link": "https://shop.example.com/me/bookings"}"#);

pub(crate) fn declare(notifier: &dyn Notifier) -> Result<(), Error> {
    notifier.declare(REMINDER)
}

pub(crate) fn registrations(service: &Arc<BookingsService>) -> Vec<JobRegistration> {
    vec![JobRegistration::new(SEND_REMINDERS, Arc::new(SendReminders(service.clone())))]
}

pub(crate) fn schedules() -> Vec<Schedule> {
    vec![Schedule::new(SEND_REMINDERS, SEND_REMINDERS, StdDuration::from_secs(300))]
}

/// Mốc nhắc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Day,
    TwoHours,
}

impl Mark {
    fn label(self) -> &'static str {
        match self {
            Mark::Day => "24h",
            Mark::TwoHours => "2h",
        }
    }
}

/// Mốc cần nhắc của lịch `row` lúc này, nếu có. Lịch đặt trong vòng 24 giờ chỉ nhận lần 2 giờ.
fn due(row: &Booking, at: Timestamp) -> Option<Mark> {
    let left = row.starts_at - at;
    if left <= SignedDuration::ZERO {
        return None;
    }
    if left <= SignedDuration::from_hours(2) {
        return row.reminded_2h_at.is_none().then_some(Mark::TwoHours);
    }
    let booked_early = row.starts_at - row.created_at > SignedDuration::from_hours(24);
    (left <= SignedDuration::from_hours(24) && booked_early && row.reminded_24h_at.is_none())
        .then_some(Mark::Day)
}

fn recipient(row: &Booking) -> Option<Recipient> {
    if let Some(user) = row.customer_user_id {
        return Some(Recipient::User(user));
    }
    if let Some(phone) = &row.customer_phone {
        return Some(Recipient::Phone(phone.clone()));
    }
    row.customer_email.clone().map(Recipient::Email)
}

impl BookingsService {
    /// Gửi nhắc tới hạn cho mọi lịch đã xác nhận trong `database` (tối đa 500 một lượt).
    pub(crate) async fn send_due_reminders(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        if self.notifier.is_none() {
            return Ok(0);
        }
        let at = now();
        let due_rows: Vec<(Uuid, Uuid)> = booking::Entity::find()
            .select_only()
            .column(booking::Column::TenantId)
            .column(booking::Column::Id)
            .filter(booking::Column::Status.eq(CONFIRMED))
            .filter(booking::Column::StartsAt.gt(at))
            .filter(booking::Column::StartsAt.lte(at + SignedDuration::from_hours(24)))
            .limit(500)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut sent = 0;
        for (tenant, id) in due_rows {
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant)).await?;
            match self.remind(&tenant, id).await {
                Ok(true) => sent += 1,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(booking_id = %id, error = %error, "cannot send a booking reminder")
                }
            }
        }
        Ok(sent)
    }

    /// Nhắc lịch `id` nếu tới mốc và chưa nhắc.
    pub(crate) async fn remind(&self, tenant: &TenantContext, id: Uuid) -> Result<bool, Error> {
        let Some(notifier) = &self.notifier else {
            return Ok(false);
        };
        let read = tenant.database().read();
        let Some(row) = booking::Entity::find_by_id(id).scoped(tenant).one(read).await? else {
            return Ok(false);
        };
        let at = now();
        let (Some(mark), Some(recipient)) = (due(&row, at), recipient(&row)) else {
            return Ok(false);
        };
        let name = service::Entity::find_by_id(row.service_id)
            .scoped(tenant)
            .one(read)
            .await?
            .map(|found| found.name)
            .unwrap_or_default();
        let zone = self.settings.timezone(tenant, OrgId::from_uuid(row.org_id)).await?;
        let time = row.starts_at.in_zone(&zone).strftime("%H:%M %d/%m/%Y").to_string();
        let path = if row.customer_user_id.is_some() {
            format!("/api/v1/bookings/me/bookings/{id}/calendar.ics")
        } else {
            format!("/api/v1/bookings/bookings/{id}/calendar.ics")
        };
        // Khoá theo giờ bắt đầu: đổi giờ thì nhắc lại theo giờ mới.
        let dedupe =
            format!("bookings.reminder:{id}:{}:{}", mark.label(), row.starts_at.as_unix_seconds());
        let mut notification = Notification::new(
            REMINDER.key(),
            recipient,
            json!({"service": name, "time": time}),
            dedupe,
        )
        .link(path);
        if let Some(locale) = &row.locale {
            notification = notification.locale(locale.clone());
        }
        let column = match mark {
            Mark::Day => booking::Column::Reminded24hAt,
            Mark::TwoHours => booking::Column::Reminded2hAt,
        };
        // Giành mốc nhắc bằng câu UPDATE có điều kiện TRƯỚC khi gửi: hai job chạy cùng lúc thì chỉ một
        // bên được 1 dòng. Gửi lỗi thì trả mốc lại để lần sau gửi tiếp (khoá chống trùng của
        // `notifications` chặn tin thứ hai nếu lần trước thật ra đã ghi).
        let mark_at = |value: Option<Timestamp>, only_if: Option<Timestamp>| {
            let mut update = booking::Entity::update_many()
                .col_expr(column, Expr::value(value))
                .filter(booking::Column::Id.eq(id))
                .scoped(tenant);
            update = match only_if {
                Some(claimed) => update.filter(column.eq(claimed)),
                None => update.filter(column.is_null()),
            };
            update
        };
        let transaction = tenant.database().begin_write().await?;
        let claimed = mark_at(Some(at), None).exec(&transaction).await?;
        transaction.commit().await?;
        if claimed.rows_affected == 0 {
            return Ok(false);
        }
        if let Err(error) = notifier.notify(tenant.id(), notification).await {
            let transaction = tenant.database().begin_write().await?;
            mark_at(None, Some(at)).exec(&transaction).await?;
            transaction.commit().await?;
            return Err(error);
        }
        Ok(true)
    }
}

struct SendReminders(Arc<BookingsService>);

#[async_trait::async_trait]
impl JobHandler for SendReminders {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.send_due_reminders(&job.database).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(starts_in_hours: i64, booked_hours_before: i64) -> Booking {
        let at = Timestamp::now();
        let starts_at = at + SignedDuration::from_hours(starts_in_hours);
        Booking {
            id: Uuid::nil(),
            tenant_id: Uuid::nil(),
            org_id: Uuid::nil(),
            service_id: Uuid::nil(),
            resource_id: Uuid::nil(),
            session_id: None,
            seats: 1,
            starts_at,
            ends_at: starts_at + SignedDuration::from_hours(1),
            status: CONFIRMED.to_owned(),
            customer_user_id: None,
            customer_name: "A".to_owned(),
            customer_phone: Some("+84912345678".to_owned()),
            customer_email: None,
            note: String::new(),
            locale: None,
            price_minor: 0,
            currency: "VND".to_owned(),
            deposit_minor: 0,
            deposit_status: "none".to_owned(),
            refund_minor: 0,
            payment_id: None,
            hold_expires_at: None,
            reminded_24h_at: None,
            reminded_2h_at: None,
            cancelled_at: None,
            cancel_reason: None,
            created_by: None,
            created_at: starts_at - SignedDuration::from_hours(booked_hours_before),
            updated_at: at,
        }
    }

    #[test]
    fn moc_nhac_dung_luc() {
        let at = Timestamp::now();
        assert_eq!(due(&row(30, 72), at), None);
        assert_eq!(due(&row(20, 72), at), Some(Mark::Day));
        // Đặt trong vòng 24 giờ: bỏ lần 24 giờ.
        assert_eq!(due(&row(20, 21), at), None);
        assert_eq!(due(&row(1, 72), at), Some(Mark::TwoHours));
        let mut done = row(1, 72);
        done.reminded_2h_at = Some(at);
        assert_eq!(due(&done, at), None);
        assert_eq!(due(&row(-1, 72), at), None);
    }
}
