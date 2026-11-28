//! Event dispatcher: reads undelivered events from the modules' `<module>_events` tables and delivers them
//! to subscribers.
//!
//! Several processes can run the dispatcher at the same time and it stays correct: each event is "claimed"
//! (`claimed_until`) with a conditional `UPDATE` statement, so only one process wins and the others skip
//! it. The statement behaves the same on SQLite and Postgres. If a process claims an event and then dies
//! midway, the claim expires and another process delivers it again.
//!
//! Per subscriber (D-20261128-event-per-subscriber): one event is handed to every subscribing module in
//! registration order, and a failing module does not stop the others. The event is then marked delivered; each
//! module that failed (or was not reached in time) gets its own retry row in `kernel_event_deliveries` and is
//! retried alone, with its own backoff and dead-letter.
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

use super::deliveries::{self, Missed};
use super::{Actor, Delivered, Subscription, table_name};
use crate::database::Database;
use crate::error::Error;
use crate::tenancy::{DatabaseRegistry, PlacementId, TenantId, TenantStatus};

/// The maximum number of delivery attempts of an event to ONE subscribing module; beyond it that module's
/// delivery goes to the dead-letter queue (`failed_at` of its `kernel_event_deliveries` row), except account
/// deletion events ([`USER_DELETED`](crate::privacy::USER_DELETED)) and account access revocation events
/// ([`ACCESS_REVOKED`](crate::accounts::ACCESS_REVOKED)), which retry forever. Other modules are not affected.
pub const MAX_ATTEMPTS: i32 = 10;
/// The number of events processed per table in one pass.
const BATCH: u64 = 100;
/// A process holds the delivery of an event for at most this long.
const CLAIM_SECONDS: i64 = 60;
/// One fan-out (all subscribers of one event), and one retry, takes at most this long. It is shorter than the
/// hold time, so no other process re-claims an event being delivered (D-158). A subscriber that needs longer
/// should work in parts (continuing next time) or hand the work off to a job; subscribers not reached in time
/// are retried on their own right after.
pub(super) const DELIVERY_TIMEOUT: Duration = Duration::from_secs(50);
/// With no new events, polls again after this long.
const IDLE_POLL: Duration = Duration::from_secs(1);
/// The maximum length of a stored error.
const MAX_ERROR_LENGTH: usize = 2000;
/// Keeps delivered events this long, then deletes them (the audit log already has its own copy); an event some
/// subscriber still has to receive is kept until it does.
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

    /// Delivers one pass of every pending event and every due per-subscriber retry; returns the number of
    /// events and retries processed (delivered or rescheduled).
    pub async fn run_once(&self) -> Result<usize, Error> {
        let databases = self.databases.databases().await?;
        self.cleanup(&databases).await?;
        let mut handled = 0;
        let mut tables: Vec<String> =
            self.modules.iter().map(|module| table_name(module)).collect();
        tables.push(deliveries::TABLE.to_owned());
        for (placement, _, database) in &databases {
            // Một câu hỏi bảng nào có event chờ, rồi chỉ đọc các bảng đó: lượt rảnh là một câu mỗi database thay vì
            // một câu mỗi module (PT nền, D-20261126-idle-polling). Bảng thử lại theo bên nhận cùng điều kiện "chờ".
            let busy = crate::data::busy::tables_with(database, &tables, &pending(now())).await?;
            for (module, busy) in self.modules.iter().zip(&busy) {
                if *busy {
                    handled += self.run_table(*placement, database, module).await?;
                }
            }
            if busy.last() == Some(&true) {
                handled += self.run_retries(*placement, database).await?;
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

    /// Deletes events delivered more than [`KEEP_DELIVERED`] ago, once an hour, except those a subscriber still
    /// has to receive. Permanently failed events and retries are kept for operators to inspect.
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
                deliveries::cleanup(database, &table_name(module), module, before).await?;
            }
            deliveries::cleanup_retries(database, before).await?;
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
            .columns(EVENT_COLUMNS)
            .from(table.clone())
            .cond_where(pending(now()))
            .order_by(Alias::new("id"), Order::Asc)
            .limit(BATCH);
        let rows = database.write().query_all(&select).await?;
        let mut handled = 0;
        for row in rows {
            let id: Uuid = row.try_get("", "id")?;
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
            let missed = self.fan_out(&event, DELIVERY_TIMEOUT).await;
            deliveries::finish_fan_out(database, &table, module, &event, &missed).await?;
        }
        Ok(handled)
    }

    /// Delivers the due per-subscriber retries on `database` (placement `placement`): each row is one module that
    /// still has to receive one event; only that module is called.
    async fn run_retries(
        &self,
        placement: PlacementId,
        database: &Database,
    ) -> Result<usize, Error> {
        let mut handled = 0;
        for retry in deliveries::due(database, BATCH).await? {
            if !claim(database, &deliveries::table(), retry.id, now()).await? {
                continue; // tiến trình khác vừa nhận dòng này
            }
            handled += 1;
            let Some(module) = self.modules.iter().copied().find(|module| *module == retry.module)
            else {
                // Module phát đang tắt (chế độ nghỉ) hay đã gỡ: bảng event của nó không thuộc bộ phát lúc này. Hoãn,
                // không tính lần thử — bật lại thì giao tiếp; gỡ hẳn thì dòng nằm chờ, hiện ở số đo tồn đọng.
                tracing::warn!(
                    retry_id = %retry.id,
                    module = %retry.module,
                    "the module that emitted this event is not active; its retry waits an hour"
                );
                deliveries::postpone(database, retry.id, SignedDuration::from_hours(1)).await?;
                continue;
            };
            let event = match self.load_event(database, module, retry.event_id).await {
                Ok(Some(event)) => event,
                Ok(None) => {
                    let error = format!(
                        "event {} is no longer in `{}`",
                        retry.event_id,
                        table_name(module)
                    );
                    deliveries::mark_dead(database, retry.id, &error).await?;
                    continue;
                }
                Err(error) => {
                    deliveries::mark_dead(database, retry.id, &describe(&error)).await?;
                    continue;
                }
            };
            if event.tenant.status() == TenantStatus::Moving {
                deliveries::postpone(database, retry.id, SignedDuration::from_secs(30)).await?;
                continue;
            }
            if event.tenant.placement() != placement {
                let error =
                    "the tenant's data moved to another placement; this retry is a left-over copy";
                deliveries::mark_dead(database, retry.id, error).await?;
                continue;
            }
            // Module nhận không còn đăng ký event này (đã gỡ, đang tắt): không còn gì để giao — như module tắt không
            // nhận bù event lúc tắt (bước 70 P9).
            let Some(subscriber) = self
                .subscriptions
                .iter()
                .map(|(subscriber, _)| *subscriber)
                .find(|subscriber| *subscriber == retry.subscriber)
            else {
                tracing::info!(
                    event_id = %event.id,
                    subscriber = %retry.subscriber,
                    "the subscribing module is not installed any more; its event retry is dropped"
                );
                deliveries::mark_done(database, retry.id).await?;
                continue;
            };
            let span = delivery_span(&event);
            let outcome = tracing::Instrument::instrument(
                tokio::time::timeout(DELIVERY_TIMEOUT, self.deliver_to(subscriber, &event)),
                span,
            )
            .await;
            match outcome {
                Ok(Ok(())) => deliveries::mark_done(database, retry.id).await?,
                Ok(Err(error)) => {
                    deliveries::mark_failed(database, &retry, &event.name, &describe(&error))
                        .await?;
                }
                Err(_) => {
                    let error = timed_out(subscriber);
                    deliveries::mark_failed(database, &retry, &event.name, &error).await?;
                }
            }
        }
        Ok(handled)
    }

    /// Reads event `id` of `module`; `None` when the row is gone.
    async fn load_event(
        &self,
        database: &Database,
        module: &'static str,
        id: Uuid,
    ) -> Result<Option<Delivered>, Error> {
        let mut select = Query::select();
        select
            .columns(EVENT_COLUMNS)
            .from(Alias::new(table_name(module)))
            .and_where(Expr::col(Alias::new("id")).eq(id));
        match database.write().query_one(&select).await? {
            Some(row) => Ok(Some(self.read_event(module, &row).await?)),
            None => Ok(None),
        }
    }

    /// Hands `event` to every subscribing module, in registration order, and returns the modules that still have
    /// to receive it. A module that fails, panics or runs past the fan-out's [`DELIVERY_TIMEOUT`] is
    /// [`Missed::Failed`]; modules not reached before that deadline are [`Missed::NotReached`]. Neither stops the
    /// modules after it, nor kills the dispatcher of the whole process (D-158).
    ///
    /// A subscriber that hangs still makes the ones after it wait, up to the deadline; the one cut off by the
    /// deadline counts one failed attempt even if it started late. `budget` is [`DELIVERY_TIMEOUT`] outside tests.
    async fn fan_out(&self, event: &Delivered, budget: Duration) -> Vec<(&'static str, Missed)> {
        let all = async {
            let deadline = tokio::time::Instant::now() + budget;
            let mut missed = Vec::new();
            for subscriber in self.subscribers_of(&event.name) {
                let left = deadline.saturating_duration_since(tokio::time::Instant::now());
                if left.is_zero() {
                    missed.push((subscriber, Missed::NotReached));
                    continue;
                }
                match tokio::time::timeout(left, self.deliver_to(subscriber, event)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => missed.push((subscriber, Missed::Failed(describe(&error)))),
                    Err(_) => missed.push((subscriber, Missed::Failed(timed_out(subscriber)))),
                }
            }
            missed
        };
        tracing::Instrument::instrument(all, delivery_span(event)).await
    }

    /// The modules subscribing to event `name`, each once, in registration order.
    fn subscribers_of(&self, name: &str) -> Vec<&'static str> {
        let mut found: Vec<&'static str> = Vec::new();
        for (subscriber, subscription) in &self.subscriptions {
            if subscription.matches(name) && !found.contains(subscriber) {
                found.push(subscriber);
            }
        }
        found
    }

    /// Delivers `event` to the subscriptions of module `subscriber` that match it, in order; stops at the
    /// first failing one (the module is retried as a whole; it tolerates receiving the event again, D-054).
    async fn deliver_to(&self, subscriber: &'static str, event: &Delivered) -> Result<(), Error> {
        // Gói của khách không có module nhận: bỏ qua (coi như đã giao), không thử lại.
        if !self.admits(subscriber, &event.tenant).await? {
            metrics::counter!("entitlement_denials_total", "kind" => "event").increment(1);
            return Ok(());
        }
        for (module, subscription) in &self.subscriptions {
            if *module != subscriber || !subscription.matches(&event.name) {
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
    let message = truncate(&describe(error));
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

/// The "pending delivery" condition: not delivered, not permanently failed, due for a retry, and not held
/// by anyone. The same for event rows and per-subscriber retry rows.
pub(super) fn pending(now: Timestamp) -> Condition {
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
pub(super) fn backoff(attempts: i32) -> SignedDuration {
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

/// `text` cut to [`MAX_ERROR_LENGTH`] characters, for `last_error`.
pub(super) fn truncate(text: &str) -> String {
    text.chars().take(MAX_ERROR_LENGTH).collect()
}

/// The error of a subscriber that ran past the delivery time.
fn timed_out(subscriber: &str) -> String {
    format!(
        "module `{subscriber}`: delivery took longer than {} seconds",
        DELIVERY_TIMEOUT.as_secs()
    )
}

/// The span of one delivery, carrying the event's tenant (step 39 B7, D-190).
fn delivery_span(event: &Delivered) -> tracing::Span {
    tracing::info_span!(
        "event_delivery",
        event_id = %event.id,
        name = %event.name,
        tenant_id = %event.tenant.id(),
    )
}

/// The columns of an event row the dispatcher reads.
const EVENT_COLUMNS: [&str; 11] = [
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
];

fn now() -> Timestamp {
    Timestamp::now()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::{ConfigReader, ConfigSource, DatabaseConfig};
    use crate::event::EventHandler;
    use crate::tenancy::{TenancyConfig, TenantContext};
    use autoxide_testkit::TestDatabase;

    // Bên nhận chờ `wait` rồi nhận.
    struct Sleeper(Duration);

    #[async_trait::async_trait]
    impl EventHandler for Sleeper {
        async fn handle(&self, _event: &Delivered) -> Result<(), Error> {
            tokio::time::sleep(self.0).await;
            Ok(())
        }
    }

    // Hết thời gian của lượt: bên đang chạy bị cắt (một lần lỗi), bên sau chưa tới lượt (`NotReached`, thử lại
    // ngay không tính lần thử); bên trước đã nhận thì không có trong danh sách.
    #[tokio::test]
    async fn het_thoi_gian_luot_ben_sau_chua_toi_luot() {
        TestDatabase::run(async |test| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let mut reader = ConfigReader::new(&source);
            let config = DatabaseConfig::read(&mut reader);
            let config = reader.finish(config).unwrap().0;
            let database = Database::connect(&config).await.unwrap();
            let registry =
                DatabaseRegistry::new(database.clone(), config, TenancyConfig::default());
            let quick: Arc<dyn EventHandler> = Arc::new(Sleeper(Duration::ZERO));
            let stuck: Arc<dyn EventHandler> = Arc::new(Sleeper(Duration::from_secs(30)));
            let subscriptions = vec![
                ("first", Subscription::new("notes.note_created", quick.clone())),
                ("stuck", Subscription::new("notes.note_created", stuck)),
                ("later", Subscription::new("notes.note_created", quick)),
            ];
            let dispatcher =
                Dispatcher::new(registry, vec!["notes"], subscriptions, Arc::new(Notify::new()));
            let event = Delivered::new(
                TenantContext::default_on(database),
                "notes",
                "notes.note_created",
                serde_json::json!({}),
            );

            let missed = dispatcher.fan_out(&event, Duration::from_millis(200)).await;
            assert_eq!(missed.len(), 2, "{missed:?}");
            assert_eq!(missed[0].0, "stuck");
            assert!(
                matches!(&missed[0].1, Missed::Failed(error) if error.contains("took longer")),
                "{missed:?}"
            );
            assert_eq!(missed[1], ("later", Missed::NotReached));
        })
        .await
        .unwrap();
    }

    #[test]
    fn backoff_tang_dan_va_co_tran() {
        assert_eq!(backoff(1), SignedDuration::from_secs(2));
        assert_eq!(backoff(3), SignedDuration::from_secs(8));
        assert_eq!(backoff(30), SignedDuration::from_secs(3600));
    }
}
