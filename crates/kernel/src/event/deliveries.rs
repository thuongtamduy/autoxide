//! Per-subscriber delivery state (D-20261128-event-per-subscriber): the `kernel_event_deliveries` table holds one
//! row for each (emitting module, event, subscribing module) whose delivery did not succeed in the event's fan-out
//! — the subscriber failed, panicked, ran out of time, or was not reached before the fan-out ran out of time.
//! Only that subscriber is retried, with its own backoff and its own dead-letter (`failed_at`); the others are
//! not held up. A fan-out where every subscriber succeeds writes nothing here.
//!
//! The table is a tenant-level queue of the kernel (like the `<module>_jobs` queues): it exists on every data database,
//! moves with the tenant, and must be empty for a split / merge.

use crate::migration::prelude::*;
use crate::migration::schema;
use crate::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, OnConflict, Order, Query};
use uuid::Uuid;

use super::Delivered;
use crate::database::Database;
use crate::error::Error;

/// The table name.
pub(crate) const TABLE: &str = "kernel_event_deliveries";

/// Why a subscriber still has to receive an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Missed {
    /// The subscriber failed (error, panic or timeout); the text is stored as `last_error`.
    Failed(String),
    /// The fan-out ran out of time before reaching this subscriber; not counted as an attempt.
    NotReached,
}

/// One row of [`TABLE`] due for a retry.
pub(super) struct Retry {
    pub(super) id: Uuid,
    /// The emitting module (the event lives in `<module>_events`).
    pub(super) module: String,
    pub(super) event_id: Uuid,
    pub(super) subscriber: String,
    pub(super) attempts: i32,
}

/// Ends the fan-out of `event` (row of `events_table`): records a retry row for each subscriber in `missed` and
/// marks the event delivered, in ONE transaction - never "delivered" without the retries.
pub(super) async fn finish_fan_out(
    database: &Database,
    events_table: &Alias,
    module: &str,
    event: &Delivered,
    missed: &[(&'static str, Missed)],
) -> Result<(), Error> {
    let now = Timestamp::now();
    if missed.is_empty() {
        // Mọi bên đã nhận: một `UPDATE` như trước, không mở transaction.
        database.write().execute(&mark_delivered(events_table, event.id, now)).await?;
        metrics::counter!("events_delivered_total").increment(1);
        return Ok(());
    }
    let transaction = database.begin_write().await?;
    for (subscriber, missed) in missed {
        let (attempts, next_attempt_at, error) = match missed {
            Missed::Failed(error) => {
                tracing::warn!(
                    event_id = %event.id,
                    event = %event.name,
                    subscriber = *subscriber,
                    error = %error,
                    "event delivery failed for one subscriber; it alone will retry"
                );
                (1, now + super::dispatcher::backoff(1), error.clone())
            }
            Missed::NotReached => (
                0,
                now,
                format!(
                    "not attempted yet: an earlier subscriber used up the {} second delivery time",
                    super::dispatcher::DELIVERY_TIMEOUT.as_secs()
                ),
            ),
        };
        let mut insert = Query::insert();
        insert
            .into_table(Alias::new(TABLE))
            .columns([
                Alias::new("id"),
                Alias::new("tenant_id"),
                Alias::new("module"),
                Alias::new("event_id"),
                Alias::new("subscriber"),
                Alias::new("attempts"),
                Alias::new("next_attempt_at"),
                Alias::new("last_error"),
                Alias::new("created_at"),
            ])
            .values([
                Uuid::now_v7().into(),
                event.tenant.id().as_uuid().into(),
                module.into(),
                event.id.into(),
                (*subscriber).into(),
                attempts.into(),
                next_attempt_at.into(),
                super::dispatcher::truncate(&error).into(),
                now.into(),
            ])
            .map_err(|error| Error::internal(format!("event delivery insert: {error}")))?
            // Một tiến trình khác đã chia event này (giữ quá hạn rồi bị nhận lại): dòng đã có, giữ dòng cũ. Không để
            // lệnh lỗi rồi bắt lỗi — Postgres huỷ cả transaction sau một lệnh lỗi (D-158).
            .on_conflict(
                OnConflict::columns([
                    Alias::new("tenant_id"),
                    Alias::new("module"),
                    Alias::new("event_id"),
                    Alias::new("subscriber"),
                ])
                .do_nothing()
                .to_owned(),
            );
        transaction.execute(&insert).await?;
    }
    transaction.execute(&mark_delivered(events_table, event.id, now)).await?;
    transaction.commit().await?;
    metrics::counter!("events_delivered_total").increment(1);
    Ok(())
}

/// The statement marking event `id` of `events_table` delivered (fan-out finished).
fn mark_delivered(
    events_table: &Alias,
    id: Uuid,
    now: Timestamp,
) -> sea_orm::sea_query::UpdateStatement {
    Query::update()
        .table(events_table.clone())
        .value(Alias::new("delivered_at"), now)
        .value(Alias::new("claimed_until"), Option::<Timestamp>::None)
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .to_owned()
}

/// Up to `limit` retry rows due now, oldest first.
pub(super) async fn due(database: &Database, limit: u64) -> Result<Vec<Retry>, Error> {
    let mut select = Query::select();
    select
        .columns(["id", "module", "event_id", "subscriber", "attempts"])
        .from(Alias::new(TABLE))
        .cond_where(super::dispatcher::pending(Timestamp::now()))
        .order_by(Alias::new("id"), Order::Asc)
        .limit(limit);
    let rows = database.write().query_all(&select).await?;
    rows.iter()
        .map(|row| {
            Ok(Retry {
                id: row.try_get("", "id")?,
                module: row.try_get("", "module")?,
                event_id: row.try_get("", "event_id")?,
                subscriber: row.try_get("", "subscriber")?,
                attempts: row.try_get("", "attempts")?,
            })
        })
        .collect()
}

/// The retry table as an `Alias`.
pub(super) fn table() -> Alias {
    Alias::new(TABLE)
}

/// Marks retry `id` delivered (or skipped: nothing left to deliver).
pub(super) async fn mark_done(database: &Database, id: Uuid) -> Result<(), Error> {
    let mut update = Query::update();
    update
        .table(table())
        .value(Alias::new("delivered_at"), Timestamp::now())
        .value(Alias::new("claimed_until"), Option::<Timestamp>::None)
        .and_where(Expr::col(Alias::new("id")).eq(id));
    database.write().execute(&update).await?;
    Ok(())
}

/// Retry `retry` failed again: schedules the next attempt with backoff, or moves it to the dead-letter queue
/// after [`MAX_ATTEMPTS`](super::MAX_ATTEMPTS) attempts - except account deletion events, which retry every hour
/// forever (D-149).
pub(super) async fn mark_failed(
    database: &Database,
    retry: &Retry,
    event_name: &str,
    error: &str,
) -> Result<(), Error> {
    let now = Timestamp::now();
    let attempts = retry.attempts + 1;
    let message = super::dispatcher::truncate(error);
    let mut update = Query::update();
    update
        .table(table())
        .value(Alias::new("attempts"), attempts)
        .value(Alias::new("next_attempt_at"), now + super::dispatcher::backoff(attempts))
        .value(Alias::new("claimed_until"), Option::<Timestamp>::None)
        .value(Alias::new("last_error"), message.clone())
        .and_where(Expr::col(Alias::new("id")).eq(retry.id));
    // Xoá dữ liệu của tài khoản đã xoá là nghĩa vụ, không phải việc phụ: không bỏ cuộc, cứ thử lại (tối đa mỗi giờ
    // một lần) tới khi bên nhận này xong (D-149). Tài khoản đã mất nên người dùng không tự xin xoá lại được.
    let never_give_up = event_name == crate::privacy::USER_DELETED;
    if attempts >= super::MAX_ATTEMPTS && !never_give_up {
        update.value(Alias::new("failed_at"), now);
        tracing::error!(
            event_id = %retry.event_id,
            event = %event_name,
            subscriber = %retry.subscriber,
            attempts,
            error = %message,
            "event delivery to this subscriber failed for good; it will not be retried"
        );
        metrics::counter!("events_failed_total").increment(1);
    } else if attempts >= super::MAX_ATTEMPTS {
        tracing::error!(
            event_id = %retry.event_id,
            event = %event_name,
            subscriber = %retry.subscriber,
            attempts,
            error = %message,
            "erasing a deleted user's data keeps failing in this module; it is retried every hour"
        );
    } else {
        tracing::warn!(
            event_id = %retry.event_id,
            event = %event_name,
            subscriber = %retry.subscriber,
            attempts,
            error = %message,
            "event delivery to this subscriber failed; will retry"
        );
    }
    database.write().execute(&update).await?;
    Ok(())
}

/// Retry `id` can never succeed (its event row is gone or unreadable): straight to the dead-letter queue.
pub(super) async fn mark_dead(database: &Database, id: Uuid, error: &str) -> Result<(), Error> {
    let message = super::dispatcher::truncate(error);
    tracing::error!(retry_id = %id, error = %message, "an event retry cannot be delivered; it will not be retried");
    let mut update = Query::update();
    update
        .table(table())
        .value(Alias::new("failed_at"), Timestamp::now())
        .value(Alias::new("claimed_until"), Option::<Timestamp>::None)
        .value(Alias::new("last_error"), message)
        .and_where(Expr::col(Alias::new("id")).eq(id));
    database.write().execute(&update).await?;
    metrics::counter!("events_failed_total").increment(1);
    Ok(())
}

/// Deletes the event rows of `events_table` (module `module`) delivered before `before` that no unfinished retry
/// (pending or dead-letter) still needs, then the retries delivered before `before`.
pub(super) async fn cleanup(
    database: &Database,
    events_table: &str,
    module: &str,
    before: Timestamp,
) -> Result<(), Error> {
    let open = Query::select()
        .column(Alias::new("event_id"))
        .from(table())
        .and_where(Expr::col(Alias::new("module")).eq(module))
        .and_where(Expr::col(Alias::new("delivered_at")).is_null())
        .to_owned();
    let mut delete = Query::delete();
    delete
        .from_table(Alias::new(events_table))
        .and_where(Expr::col(Alias::new("delivered_at")).lt(before))
        .and_where(Expr::col(Alias::new("id")).not_in_subquery(open));
    database.write().execute(&delete).await?;
    Ok(())
}

/// Deletes retries delivered before `before`.
pub(super) async fn cleanup_retries(database: &Database, before: Timestamp) -> Result<(), Error> {
    let mut delete = Query::delete();
    delete.from_table(table()).and_where(Expr::col(Alias::new("delivered_at")).lt(before));
    database.write().execute(&delete).await?;
    Ok(())
}

/// Retry `id` cannot run now (its tenant is moving, its emitting module is turned off): retry after `wait`,
/// without counting an attempt.
pub(super) async fn postpone(
    database: &Database,
    id: Uuid,
    wait: SignedDuration,
) -> Result<(), Error> {
    let mut update = Query::update();
    update
        .table(table())
        .value(Alias::new("next_attempt_at"), Timestamp::now() + wait)
        .value(Alias::new("claimed_until"), Option::<Timestamp>::None)
        .and_where(Expr::col(Alias::new("id")).eq(id));
    database.write().execute(&update).await?;
    Ok(())
}

/// Unfinished retries (pending or dead-letter) of the events of `module`, for the backlog gauges (step 39 A3).
pub(crate) fn open_condition(module: &str) -> Condition {
    Condition::all()
        .add(Expr::col(Alias::new("module")).eq(module))
        .add(Expr::col(Alias::new("delivered_at")).is_null())
}

/// Migration that creates [`TABLE`] (kernel migration list).
pub(crate) struct CreateEventDeliveries;

impl MigrationName for CreateEventDeliveries {
    fn name(&self) -> &str {
        "m0044_create_event_deliveries"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateEventDeliveries {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("module", 64))
                    .col(schema::uuid("event_id"))
                    .col(schema::string_len("subscriber", 64))
                    .col(schema::integer("attempts"))
                    .col(schema::timestamp_with_time_zone("next_attempt_at"))
                    .col(schema::timestamp_with_time_zone_null("claimed_until"))
                    .col(schema::timestamp_with_time_zone_null("delivered_at"))
                    .col(schema::timestamp_with_time_zone_null("failed_at"))
                    .col(schema::text_null("last_error"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        // Mỗi bên nhận một dòng cho một event (chặn hai tiến trình cùng ghi).
        manager
            .create_index(
                Index::create()
                    .name("kernel_event_deliveries_subscriber_idx")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("module"))
                    .col(Alias::new("event_id"))
                    .col(Alias::new("subscriber"))
                    .unique()
                    .to_owned(),
            )
            .await?;
        // Bộ phát quét mọi khách tìm dòng tới hạn, như bảng event (`…_pending_sweep_idx`).
        manager
            .create_index(
                Index::create()
                    .name("kernel_event_deliveries_pending_sweep_idx")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("delivered_at"))
                    .col(Alias::new("failed_at"))
                    .col(Alias::new("next_attempt_at"))
                    .to_owned(),
            )
            .await?;
        // Dọn event và đếm tồn đọng theo module phát.
        manager
            .create_index(
                Index::create()
                    .name("kernel_event_deliveries_module_sweep_idx")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("module"))
                    .col(Alias::new("delivered_at"))
                    .col(Alias::new("event_id"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(TABLE)).to_owned()).await
    }
}
