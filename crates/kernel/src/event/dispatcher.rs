//! Event dispatcher: reads undelivered events from the modules' `<module>_events` tables and delivers them
//! to subscribers.
//!
//! Several processes can run the dispatcher at the same time and it stays correct: each event is "claimed"
//! (`claimed_until`) with a conditional `UPDATE` statement, so only one process wins and the others skip
//! it. The statement behaves the same on SQLite and Postgres. If a process claims an event and then dies
//! midway, the claim expires and another process delivers it again.
//!
//! Multiple databases (step 33): each pass walks every assigned database (`DatabaseRegistry::databases`,
//! filtered by `AUTOXIDE_WORKER_PLACEMENTS`); an event carries its tenant (`tenant_id`) and the subscriber
//! gets that tenant's context in [`Delivered::tenant`].

use crate::time::Timestamp;
use futures_util::FutureExt as _;
use jiff::SignedDuration;
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, Order, Query};
use sea_orm::{ConnectionTrait, QueryResult};
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use uuid::Uuid;

use super::{Actor, Delivered, Subscription, table_name};
use crate::database::Database;
use crate::error::Error;
use crate::tenancy::{DatabaseRegistry, PlacementId, TenantId, TenantStatus};

/// The maximum number of delivery attempts for an event; beyond it the event is marked failed
/// (`failed_at`), except account deletion events ([`USER_DELETED`](crate::privacy::USER_DELETED)) and
/// account access revocation events ([`ACCESS_REVOKED`](crate::accounts::ACCESS_REVOKED)), which retry forever.
pub const MAX_ATTEMPTS: i32 = 10;
/// The number of events processed per table in one pass.
const BATCH: u64 = 100;
/// A process holds the delivery of an event for at most this long.
const CLAIM_SECONDS: i64 = 60;
/// One delivery (all subscribers of one event) takes at most this long. It is shorter than the hold time,
/// so no other process re-claims an event being delivered (D-158). A subscriber that needs longer should
/// work in parts (continuing next time) or hand the work off to a job.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(50);
/// With no new events, polls again after this long.
const IDLE_POLL: Duration = Duration::from_secs(1);
/// The maximum length of a stored error.
const MAX_ERROR_LENGTH: usize = 2000;
/// Keeps delivered events this long, then deletes them (the audit log already has its own copy).
const KEEP_DELIVERED: SignedDuration = SignedDuration::from_hours(24 * 7);
/// Cleans up delivered events this often.
const CLEANUP_EVERY: Duration = Duration::from_secs(3600);

/// The program's event dispatcher.
pub struct Dispatcher {
    databases: DatabaseRegistry,
    /// The modules that have an event table.
    modules: Vec<&'static str>,
    /// (subscribing module, subscription).
    subscriptions: Vec<(&'static str, Subscription)>,
    wake: Arc<Notify>,
    last_cleanup: std::sync::Mutex<Option<std::time::Instant>>,
    /// Per-tenant entitlements (step 35): subscribers in a module outside the tenant's plan are skipped.
    entitlements: crate::entitlement::EntitlementsHandle,
}

impl Dispatcher {
    pub(crate) fn new(
        databases: DatabaseRegistry,
        modules: Vec<&'static str>,
        subscriptions: Vec<(&'static str, Subscription)>,
        wake: Arc<Notify>,
    ) -> Self {
        Dispatcher {
            databases,
            modules,
            subscriptions,
            wake,
            last_cleanup: std::sync::Mutex::new(None),
            entitlements: crate::entitlement::EntitlementsHandle::default(),
        }
    }

    /// Skips subscribers in a module that the event's tenant plan does not include (step 35).
    pub(crate) fn with_entitlements(
        mut self,
        entitlements: crate::entitlement::EntitlementsHandle,
    ) -> Self {
        self.entitlements = entitlements;
        self
    }

    /// Whether a subscriber in module `subscriber` may receive an event of tenant `tenant`: the module is
    /// `always_enabled`, or the tenant's plan includes the module and the tenant is not read-only due to
    /// subscription.
    async fn admits(
        &self,
        subscriber: &str,
        tenant: &crate::tenancy::TenantContext,
    ) -> Result<bool, Error> {
        if !self.entitlements.is_enforced()
            || self.entitlements.catalog().is_always_enabled(subscriber)
        {
            return Ok(true);
        }
        let snapshot = self.entitlements.snapshot(tenant.id()).await?;
        Ok(snapshot.has_module(subscriber)
            && snapshot.access == crate::entitlement::TenantAccess::Full)
    }

    /// Delivers one pass of every pending event; returns the number of events processed (delivered or
    /// rescheduled).
    pub async fn run_once(&self) -> Result<usize, Error> {
        let databases = self.databases.databases().await?;
        self.cleanup(&databases).await?;
        let mut handled = 0;
        let tables: Vec<String> = self.modules.iter().map(|module| table_name(module)).collect();
        for (placement, _, database) in &databases {
            // Một câu hỏi bảng nào có event chờ, rồi chỉ đọc các bảng đó: lượt rảnh là một câu mỗi database thay vì
            // một câu mỗi module (PT nền, D-20261126-idle-polling).
            let busy = crate::data::busy::tables_with(database, &tables, &pending(now())).await?;
            for (module, busy) in self.modules.iter().zip(busy) {
                if busy {
                    handled += self.run_table(*placement, database, module).await?;
                }
            }
        }
        Ok(handled)
    }

    /// Delivers events until `shutdown` completes. On a database error it logs and retries on the next
    /// pass: the dispatcher must not die because of one database hiccup.
    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        tokio::pin!(shutdown);
        loop {
            let handled = match self.run_once().await {
                Ok(handled) => handled,
                Err(error) => {
                    tracing::error!(error = %error, "event dispatcher failed; retrying");
                    0
                }
            };
            if handled > 0 {
                // Còn event: chạy lượt kế ngay, nhưng vẫn xét lệnh dừng giữa hai lượt — tồn đọng lớn (sau một đợt tải)
                // không được giữ tiến trình sống tới khi giao hết (PT-17). Event chưa giao nằm lại, lần chạy sau giao.
                if (&mut shutdown).now_or_never().is_some() {
                    return;
                }
                continue;
            }
            tokio::select! {
                () = &mut shutdown => return,
                () = self.wake.notified() => {}
                () = tokio::time::sleep(IDLE_POLL) => {}
            }
        }
    }

    /// Deletes events delivered more than [`KEEP_DELIVERED`] ago, once an hour. Permanently failed events
    /// are kept for operators to inspect.
    async fn cleanup(&self, databases: &[(PlacementId, String, Database)]) -> Result<(), Error> {
        {
            let mut last = self.last_cleanup.lock().map_err(|_| Error::internal("cleanup lock"))?;
            if last.is_some_and(|at| at.elapsed() < CLEANUP_EVERY) {
                return Ok(());
            }
            *last = Some(std::time::Instant::now());
        }
        let before = now() - KEEP_DELIVERED;
        for (_, _, database) in databases {
            for module in &self.modules {
                let mut delete = Query::delete();
                delete
                    .from_table(Alias::new(table_name(module)))
                    .and_where(Expr::col(Alias::new("delivered_at")).lt(before));
                database.write().execute(&delete).await?;
            }
        }
        Ok(())
    }

    /// Delivers pending events in the event table of `module` on `database` (placement `placement`).
    ///
    /// Tenant being moved (`moving`): the event is rescheduled for later without counting as an attempt,
    /// since it travels with the data to the destination. Tenant already on another placement: this row is
    /// a leftover copy at the source and is not delivered.
    async fn run_table(
        &self,
        placement: PlacementId,
        database: &Database,
        module: &'static str,
    ) -> Result<usize, Error> {
        let table = Alias::new(table_name(module));
        let mut select = Query::select();
        select
            .columns([
                "tenant_id",
                "id",
                "name",
                "actor_user_id",
                "actor_api_key_id",
                "actor_kind",
                "actor_access",
                "actor_access_id",
                "subject",
                "payload",
                "occurred_at",
                "attempts",
            ])
            .from(table.clone())
            .cond_where(pending(now()))
            .order_by(Alias::new("id"), Order::Asc)
            .limit(BATCH);
        let rows = database.write().query_all(&select).await?;
        let mut handled = 0;
        for row in rows {
            let id: Uuid = row.try_get("", "id")?;
            let attempts: i32 = row.try_get("", "attempts")?;
            let event = match self.read_event(module, &row).await {
                Ok(event) => event,
                Err(error) => {
                    // Dòng hỏng (sửa tay, dữ liệu lỗi, khách không còn): đánh dấu hỏng hẳn rồi đi
                    // tiếp — không để nó chặn mọi event sau nó mỗi giây (D-158).
                    mark_unreadable(database, &table, id, &error).await?;
                    continue;
                }
            };
            if event.tenant.status() == TenantStatus::Moving {
                postpone(database, &table, event.id).await?;
                continue;
            }
            if event.tenant.placement() != placement {
                let error = Error::internal(format!(
                    "the tenant's data moved to placement {}; this event is a left-over copy at \
                     the old placement and is not delivered",
                    event.tenant.placement()
                ));
                mark_unreadable(database, &table, id, &error).await?;
                continue;
            }
            // Giờ mới cho mỗi lần nhận: giao các event trước có thể đã mất thời gian.
            if !claim(database, &table, event.id, now()).await? {
                continue; // tiến trình khác vừa nhận việc giao event này
            }
            handled += 1;
            match self.deliver(&event).await {
                Ok(()) => mark_delivered(database, &table, event.id).await?,
                Err(error) => mark_failed(database, &table, &event, attempts + 1, &error).await?,
            }
        }
        Ok(handled)
    }

    /// Delivers to every subscriber matching the event name, in order; stops at the first failing
    /// subscriber. A subscriber that panics or runs longer than [`DELIVERY_TIMEOUT`] counts as one failed
    /// attempt (retried with backoff) and does not kill the dispatcher of the whole process (D-158).
    async fn deliver(&self, event: &Delivered) -> Result<(), Error> {
        let all = async {
            for (subscriber, subscription) in &self.subscriptions {
                if !subscription.matches(&event.name) {
                    continue;
                }
                // Gói của khách không có module nhận: bỏ qua (coi như đã giao), không thử lại.
                if !self.admits(subscriber, &event.tenant).await? {
                    metrics::counter!("entitlement_denials_total", "kind" => "event").increment(1);
                    continue;
                }
                let handled = AssertUnwindSafe(subscription.handler.handle(event)).catch_unwind();
                match handled.await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        return Err(Error::internal(format!(
                            "module `{subscriber}`: {}",
                            describe(&error)
                        )));
                    }
                    Err(_) => {
                        return Err(Error::internal(format!(
                            "module `{subscriber}`: the event handler panicked"
                        )));
                    }
                }
            }
            Ok(())
        };
        // Span của lượt giao mang khách của event (bước 39 B7, D-190).
        let span = tracing::info_span!(
            "event_delivery",
            event_id = %event.id,
            name = %event.name,
            tenant_id = %event.tenant.id(),
        );
        let all = tracing::Instrument::instrument(all, span);
        match tokio::time::timeout(DELIVERY_TIMEOUT, all).await {
            Ok(outcome) => outcome,
            Err(_) => Err(Error::internal(format!(
                "delivery took longer than {} seconds",
                DELIVERY_TIMEOUT.as_secs()
            ))),
        }
    }

    /// Reads one event row, with its tenant context.
    async fn read_event(
        &self,
        module: &'static str,
        row: &QueryResult,
    ) -> Result<Delivered, Error> {
        let tenant = TenantId::from_uuid(row.try_get("", "tenant_id")?);
        let tenant = self.databases.tenant(tenant).await?;
        let payload: String = row.try_get("", "payload")?;
        let actor_kind: Option<String> = row.try_get("", "actor_kind")?;
        Ok(Delivered {
            id: row.try_get("", "id")?,
            tenant,
            module,
            name: row.try_get("", "name")?,
            actor: Actor {
                user_id: row.try_get("", "actor_user_id")?,
                api_key_id: row.try_get("", "actor_api_key_id")?,
                public: actor_kind.as_deref() == Some("public"),
                kind: actor_kind.as_deref().and_then(crate::access::ActorKind::parse).filter(
                    |kind| {
                        !matches!(
                            kind,
                            crate::access::ActorKind::Human | crate::access::ActorKind::System
                        )
                    },
                ),
                access: access_of(
                    row.try_get::<Option<String>>("", "actor_access")?.as_deref(),
                    row.try_get("", "actor_access_id")?,
                ),
            },
            subject: row.try_get("", "subject")?,
            payload: serde_json::from_str(&payload).map_err(Error::internal)?,
            occurred_at: row.try_get("", "occurred_at")?,
        })
    }
}

/// The entry path read from the outbox's two columns; an old row (with no value) is direct.
fn access_of(kind: Option<&str>, id: Option<Uuid>) -> crate::access::AccessPath {
    match (kind, id) {
        (Some("delegation"), Some(id)) => crate::access::AccessPath::Delegation(id),
        (Some("support"), Some(id)) => crate::access::AccessPath::Support(id),
        (Some("automation"), Some(id)) => crate::access::AccessPath::Automation(id),
        (Some("oauth_grant"), Some(id)) => crate::access::AccessPath::OAuthGrant(id),
        _ => crate::access::AccessPath::Direct,
    }
}

/// Claims the delivery of event `id`. Returns `false` if another process already claimed it.
async fn claim(
    database: &Database,
    table: &Alias,
    id: Uuid,
    now: Timestamp,
) -> Result<bool, Error> {
    let mut update = Query::update();
    update
        .table(table.clone())
        .value(Alias::new("claimed_until"), now + SignedDuration::from_secs(CLAIM_SECONDS))
        .cond_where(Condition::all().add(Expr::col(Alias::new("id")).eq(id)).add(pending(now)));
    Ok(database.write().execute(&update).await?.rows_affected() == 1)
}

/// The tenant of event `id` is being moved: retry later, without counting an attempt.
async fn postpone(database: &Database, table: &Alias, id: Uuid) -> Result<(), Error> {
    let now = now();
    let mut update = Query::update();
    update
        .table(table.clone())
        .value(Alias::new("next_attempt_at"), now + SignedDuration::from_secs(30))
        .cond_where(Condition::all().add(Expr::col(Alias::new("id")).eq(id)).add(pending(now)));
    database.write().execute(&update).await?;
    Ok(())
}

/// The row of event `id` cannot be read: permanently failed, with the reason.
async fn mark_unreadable(
    database: &Database,
    table: &Alias,
    id: Uuid,
    error: &Error,
) -> Result<(), Error> {
    let message: String = describe(error).chars().take(MAX_ERROR_LENGTH).collect();
    tracing::error!(event_id = %id, error = %message, "an event row cannot be read; it will not be delivered");
    let mut update = Query::update();
    update
        .table(table.clone())
        .value(Alias::new("failed_at"), now())
        .value(Alias::new("last_error"), message)
        .and_where(Expr::col(Alias::new("id")).eq(id));
    database.write().execute(&update).await?;
    metrics::counter!("events_failed_total").increment(1);
    Ok(())
}

async fn mark_delivered(database: &Database, table: &Alias, id: Uuid) -> Result<(), Error> {
    let mut update = Query::update();
    update
        .table(table.clone())
        .value(Alias::new("delivered_at"), now())
        .value(Alias::new("claimed_until"), Option::<Timestamp>::None)
        .and_where(Expr::col(Alias::new("id")).eq(id));
    database.write().execute(&update).await?;
    metrics::counter!("events_delivered_total").increment(1);
    Ok(())
}

/// Events whose delivery never gives up (retried at most hourly after [`MAX_ATTEMPTS`]).
const NEVER_GIVE_UP: [&str; 2] = [crate::privacy::USER_DELETED, crate::accounts::ACCESS_REVOKED];

/// Records a failed delivery: schedules the next attempt with backoff, or marks it permanently failed.
async fn mark_failed(
    database: &Database,
    table: &Alias,
    event: &Delivered,
    attempts: i32,
    error: &Error,
) -> Result<(), Error> {
    let now = now();
    let message: String = describe(error).chars().take(MAX_ERROR_LENGTH).collect();
    let mut update = Query::update();
    update
        .table(table.clone())
        .value(Alias::new("attempts"), attempts)
        .value(Alias::new("next_attempt_at"), now + backoff(attempts))
        .value(Alias::new("claimed_until"), Option::<Timestamp>::None)
        .value(Alias::new("last_error"), message.clone())
        .and_where(Expr::col(Alias::new("id")).eq(event.id));
    // Xoá dữ liệu của tài khoản đã xoá là nghĩa vụ, không phải việc phụ: không bỏ cuộc,
    // cứ thử lại (tối đa mỗi giờ một lần) tới khi mọi module xong (D-149). Tài khoản đã
    // mất nên người dùng không tự xin xoá lại được. Cắt cửa vào tài khoản cũng vậy: bên nhận dừng thứ
    // người lạ có thể đã dựng khi giữ tài khoản (luật tự động hoá), event bỏ cuộc thì không còn đường nào
    // khác dừng chúng (D-20261128-access-revoked-retry).
    let never_give_up = NEVER_GIVE_UP.contains(&event.name.as_str());
    if attempts >= MAX_ATTEMPTS && !never_give_up {
        update.value(Alias::new("failed_at"), now);
        tracing::error!(
            event_id = %event.id,
            event = %event.name,
            attempts,
            error = %message,
            "event delivery failed for good; it will not be retried"
        );
        metrics::counter!("events_failed_total").increment(1);
    } else if attempts >= MAX_ATTEMPTS {
        tracing::error!(
            event_id = %event.id,
            event = %event.name,
            attempts,
            error = %message,
            "an event that must reach every subscriber keeps failing; it is retried every hour"
        );
    } else {
        tracing::warn!(
            event_id = %event.id,
            event = %event.name,
            attempts,
            error = %message,
            "event delivery failed; will retry"
        );
    }
    database.write().execute(&update).await?;
    Ok(())
}

/// The "pending delivery" condition: not delivered, not permanently failed, due for a retry, and not held
/// by anyone.
fn pending(now: Timestamp) -> Condition {
    Condition::all()
        .add(Expr::col(Alias::new("delivered_at")).is_null())
        .add(Expr::col(Alias::new("failed_at")).is_null())
        .add(Expr::col(Alias::new("next_attempt_at")).lte(now))
        .add(
            Condition::any()
                .add(Expr::col(Alias::new("claimed_until")).is_null())
                .add(Expr::col(Alias::new("claimed_until")).lt(now)),
        )
}

/// The wait before attempt number `attempts + 1`: 2, 4, 8... seconds, at most one hour.
fn backoff(attempts: i32) -> SignedDuration {
    let exponent = u32::try_from(attempts.clamp(1, 12)).unwrap_or(12);
    SignedDuration::from_secs(std::cmp::min(i64::from(2u32.pow(exponent)), 3600))
}

/// An error with its root cause (an internal error carries only a generic message; the cause is what helps
/// to fix it).
fn describe(error: &Error) -> String {
    match std::error::Error::source(error) {
        Some(source) => format!("{}: {source}", error.code()),
        None => format!("{}: {}", error.code(), error.message()),
    }
}

fn now() -> Timestamp {
    Timestamp::now()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_tang_dan_va_co_tran() {
        assert_eq!(backoff(1), SignedDuration::from_secs(2));
        assert_eq!(backoff(3), SignedDuration::from_secs(8));
        assert_eq!(backoff(30), SignedDuration::from_secs(3600));
    }
}
