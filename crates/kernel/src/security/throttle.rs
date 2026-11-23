//! Per-key rate limiting, counted in the database (D-116): correct when running several servers and when
//! many requests arrive at once, on both SQLite and Postgres.
//!
//! Used for: login locks by email (`auth`), form submission limits by hashed IP or by email
//! (anti-spam), limits on confirmation emails sent to one address…
//!
//! - A module adds `throttle_migration("m00xx_create_throttle", "<module>")` to its own
//!   migrations (table `<module>_throttle`), like `jobs_migration`.
//! - Each key has a counting window of length `window`. Exceeding `limit` hits within the window blocks the key
//!   for `lock` (default equal to `window`), then counting restarts from zero.
//! - Counted keys are hashed with SHA-256 before storing: emails or IPs are not stored verbatim in the table.
//! - Each count is ONE "insert or add" statement that returns the count (D-321) — the database queues
//!   concurrent requests itself, and no request overwrites another request's count. [`Throttle::hit`] counts
//!   FIRST and then checks, so N requests fired at once cannot exceed `limit`.
//! - Per tenant (step 33): every function takes the tenant; counts live in the tenant's database, primary key
//!   `(tenant_id, key)` — one tenant being flooded does not lock the users of another tenant.

use crate::time::Timestamp;
use jiff::SignedDuration;
use std::time::Duration;

use crate::migration::prelude::{
    DbErr, Index, MigrationName, MigrationTrait, SchemaManager, Table,
};
use crate::migration::schema;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, OnConflict, Query};

use crate::database::Database;
use crate::error::Error;
use crate::secret::sha256;
use crate::tenancy::TenantContext;

/// Result of one evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Allowed,
    /// Blocked; retry after this many seconds (rounded up).
    Limited {
        retry_after: u64,
    },
}

impl Outcome {
    pub fn is_allowed(self) -> bool {
        self == Outcome::Allowed
    }
}

/// A counter on the table `<module>_throttle`.
#[derive(Clone)]
pub struct Throttle {
    table: String,
    limit: i32,
    window: SignedDuration,
    lock: SignedDuration,
}

impl std::fmt::Debug for Throttle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Throttle")
            .field("table", &self.table)
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

impl Throttle {
    /// Allows `limit` hits per `window` on the table `<module>_throttle`.
    pub fn new(module: &str, limit: u32, window: Duration) -> Result<Self, Error> {
        let limit = <i32 as TryFrom<u32>>::try_from(limit).map_err(Error::internal)?;
        let window = SignedDuration::try_from(window).map_err(Error::internal)?;
        Ok(Throttle { table: table_name(module), limit, window, lock: window })
    }

    /// The module that owns the table (`<module>_throttle`) — so anti-spam reads the nonce table of the same module.
    pub(crate) fn owner(&self) -> Option<&str> {
        self.table.strip_suffix("_throttle")
    }

    /// When the limit is exceeded, blocks for `lock` (instead of `window`).
    pub fn with_lock(mut self, lock: Duration) -> Result<Self, Error> {
        self.lock = SignedDuration::try_from(lock).map_err(Error::internal)?;
        Ok(self)
    }

    /// Whether key `key` of tenant `tenant` is currently blocked, WITHOUT counting.
    pub async fn check(&self, tenant: &TenantContext, key: &str) -> Result<Outcome, Error> {
        let now = now();
        let select = Query::select()
            .column(Alias::new("locked_until"))
            .from(Alias::new(&self.table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("key")).eq(sha256(key)))
            .to_owned();
        let row = tenant.database().write().query_one(&select).await?;
        let until = match row {
            Some(row) => row.try_get::<Option<Timestamp>>("", "locked_until")?,
            None => None,
        };
        Ok(match until {
            Some(until) if until > now => Outcome::Limited { retry_after: seconds_up(until - now) },
            _ => Outcome::Allowed,
        })
    }

    /// Counts once for `key` and then evaluates: if blocked (from before or because this hit exceeded the
    /// limit) it returns [`Outcome::Limited`]. A hit that is already blocked is not counted further.
    ///
    /// Two statements for an allowed hit (D-321): read whether it is blocked (no row lock — a blocked hit
    /// stops here), then one "insert or add" statement that returns the count of THIS very hit. Concurrent hits
    /// each receive a different count, so exactly `limit` hits are allowed.
    pub async fn hit(&self, tenant: &TenantContext, key: &str) -> Result<Outcome, Error> {
        let before = self.check(tenant, key).await?;
        if !before.is_allowed() {
            return Ok(before);
        }
        let hashed = sha256(key);
        let now = now();
        let counted = self.count(tenant, &hashed, now).await?;
        // Một lượt đồng thời khác vừa khoá (và đặt lại số đếm về 0) sau khi lượt này qua bước
        // `check`: số đếm trả về lại nhỏ, nhưng khoá đang có hiệu lực — vẫn phải chặn.
        if let Some(until) = counted.locked_until.filter(|until| *until > now) {
            return Ok(Outcome::Limited { retry_after: seconds_up(until - now) });
        }
        if counted.count > i64::from(self.limit) {
            self.lock_if_exceeded(tenant, &hashed, now).await?;
            return Ok(Outcome::Limited { retry_after: seconds_up(self.lock) });
        }
        Ok(Outcome::Allowed)
    }

    /// Counts once for `key` without evaluating (e.g. recording one failed login).
    pub async fn record(&self, tenant: &TenantContext, key: &str) -> Result<(), Error> {
        let hashed = sha256(key);
        let now = now();
        if self.count(tenant, &hashed, now).await?.count > i64::from(self.limit) {
            self.lock_if_exceeded(tenant, &hashed, now).await?;
        }
        Ok(())
    }

    /// Adds one to the count of the hashed key `hashed` with ONE statement and returns the count after
    /// adding: with no row yet it inserts with 1; when the old window has ended it starts a new window with 1.
    /// `INSERT … ON CONFLICT DO UPDATE … RETURNING` runs on both SQLite (from 3.35) and Postgres; the
    /// database queues concurrent hits on that row. Also returns the row's `locked_until` (the add statement
    /// does not change this column).
    async fn count(
        &self,
        tenant: &TenantContext,
        hashed: &str,
        now: Timestamp,
    ) -> Result<Counted, Error> {
        let table = Alias::new(&self.table);
        // Cột của dòng đang có, ghi kèm tên bảng cho rõ (không lẫn với `excluded`).
        let column = |name: &str| Expr::col((table.clone(), Alias::new(name)));
        let fresh = column("window_started_at").gt(now - self.window);
        let upsert = Query::insert()
            .into_table(table.clone())
            .columns(["tenant_id", "key", "count", "window_started_at"].map(Alias::new))
            .values_panic([tenant.id().as_uuid().into(), hashed.into(), 1.into(), now.into()])
            .on_conflict(
                OnConflict::columns([Alias::new("tenant_id"), Alias::new("key")])
                    .value(
                        Alias::new("count"),
                        Expr::case(fresh.clone(), column("count").add(1)).finally(1),
                    )
                    .value(
                        Alias::new("window_started_at"),
                        Expr::case(fresh, column("window_started_at")).finally(now),
                    )
                    .to_owned(),
            )
            .returning(Query::returning().columns(["count", "locked_until"].map(Alias::new)))
            .to_owned();
        let row = tenant.database().write().query_one(&upsert).await?.ok_or_else(|| {
            Error::internal(format!("counting in `{}` returned no row", self.table))
        })?;
        Ok(Counted {
            count: i64::from(row.try_get::<i32>("", "count")?),
            locked_until: row.try_get::<Option<Timestamp>>("", "locked_until")?,
        })
    }

    /// Takes up to `n` hits of key `key` in the current window with ONE statement (D-375): used for batched
    /// rate limiting — each server takes a batch and spends it gradually in memory, asking again only when it
    /// is used up. The count adds `n` directly (exceeding `limit` is fine: later hits in the window get 0),
    /// so the number of hits granted can be computed from the returned count: `min(n, limit − (count − n))`.
    /// Concurrent hits are queued by the database on the row, so the total granted in one window does not
    /// exceed `limit`. It does not touch `locked_until` (that belongs to [`hit`](Self::hit)).
    pub async fn take(&self, tenant: &TenantContext, key: &str, n: u32) -> Result<Taken, Error> {
        let wanted = <i32 as TryFrom<u32>>::try_from(n).map_err(Error::internal)?;
        let now = now();
        let table = Alias::new(&self.table);
        let column = |name: &str| Expr::col((table.clone(), Alias::new(name)));
        let fresh = column("window_started_at").gt(now - self.window);
        let upsert = Query::insert()
            .into_table(table.clone())
            .columns(["tenant_id", "key", "count", "window_started_at"].map(Alias::new))
            .values_panic([
                tenant.id().as_uuid().into(),
                sha256(key).into(),
                wanted.into(),
                now.into(),
            ])
            .on_conflict(
                OnConflict::columns([Alias::new("tenant_id"), Alias::new("key")])
                    .value(
                        Alias::new("count"),
                        Expr::case(fresh.clone(), column("count").add(wanted)).finally(wanted),
                    )
                    .value(
                        Alias::new("window_started_at"),
                        Expr::case(fresh, column("window_started_at")).finally(now),
                    )
                    .to_owned(),
            )
            .returning(Query::returning().columns(["count", "window_started_at"].map(Alias::new)))
            .to_owned();
        let row = tenant.database().write().query_one(&upsert).await?.ok_or_else(|| {
            Error::internal(format!("taking from `{}` returned no row", self.table))
        })?;
        let count = i64::from(row.try_get::<i32>("", "count")?);
        let started = row.try_get::<Timestamp>("", "window_started_at")?;
        let before = count - i64::from(wanted);
        let granted = (i64::from(self.limit) - before).clamp(0, i64::from(wanted));
        let ends = started + self.window;
        Ok(Taken {
            granted: <u32 as TryFrom<i64>>::try_from(granted).unwrap_or(0),
            window_left: std::time::Duration::try_from(ends - now).unwrap_or_default(),
        })
    }

    /// Gives back one counted hit (e.g. the attempt turned out not to be wrong): a real user is not penalized unfairly.
    pub async fn refund(&self, tenant: &TenantContext, key: &str) -> Result<(), Error> {
        let update = Query::update()
            .table(Alias::new(&self.table))
            .value(Alias::new("count"), Expr::col(Alias::new("count")).sub(1))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("key")).eq(sha256(key)))
            .and_where(Expr::col(Alias::new("count")).gt(0))
            .to_owned();
        tenant.database().write().execute(&update).await?;
        Ok(())
    }

    /// Forgets every counted hit of `key` (e.g. a correct login).
    pub async fn clear(&self, tenant: &TenantContext, key: &str) -> Result<(), Error> {
        let delete = Query::delete()
            .from_table(Alias::new(&self.table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("key")).eq(sha256(key)))
            .to_owned();
        tenant.database().write().execute(&delete).await?;
        Ok(())
    }

    /// Deletes the rows (all tenants) of `database` whose window ended before `before` and that are no longer
    /// blocked. Modules call it in their own periodic cleanup job. Returns the number of rows deleted.
    pub async fn purge(&self, database: &Database, before: Timestamp) -> Result<u64, Error> {
        let now = now();
        let delete = Query::delete()
            .from_table(Alias::new(&self.table))
            .cond_where(
                Condition::all().add(Expr::col(Alias::new("window_started_at")).lt(before)).add(
                    Condition::any()
                        .add(Expr::col(Alias::new("locked_until")).is_null())
                        .add(Expr::col(Alias::new("locked_until")).lt(now)),
                ),
            )
            .to_owned();
        Ok(database.write().execute(&delete).await?.rows_affected())
    }

    /// When the threshold is exceeded, blocks until `now + lock` and restarts counting from zero, so once
    /// unblocked the full number of attempts is available again as new.
    async fn lock_if_exceeded(
        &self,
        tenant: &TenantContext,
        hashed: &str,
        now: Timestamp,
    ) -> Result<(), Error> {
        let update = Query::update()
            .table(Alias::new(&self.table))
            .value(Alias::new("locked_until"), Some(now + self.lock))
            .value(Alias::new("count"), 0)
            .value(Alias::new("window_started_at"), now)
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("key")).eq(hashed))
            // Lượt vượt mức mới chặn: `limit` lượt đầu vẫn được.
            .and_where(Expr::col(Alias::new("count")).gt(self.limit))
            .to_owned();
        let locked = tenant.database().write().execute(&update).await?.rows_affected();
        if locked > 0 {
            metrics::counter!("kernel_throttle_locks_total", "table" => self.table.clone())
                .increment(1);
        }
        Ok(())
    }
}

/// Result of [`Throttle::take`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Taken {
    /// Number of hits granted, from 0 up to the number requested.
    pub granted: u32,
    /// Time left in the window: granted hits can only be used within this span; when 0 is granted, retry after
    /// this long.
    pub window_left: Duration,
}

/// Result of the "insert or add" statement: this hit's own count and the row's lock expiry (if any).
struct Counted {
    count: i64,
    locked_until: Option<Timestamp>,
}

/// Number of seconds (rounded up, at least 1) of `duration`.
fn seconds_up(duration: SignedDuration) -> u64 {
    let millis = Ord::max(i64::try_from(duration.as_millis()).unwrap_or(i64::MAX), 0);
    Ord::max(<u64 as TryFrom<i64>>::try_from((millis + 999) / 1000).unwrap_or(1), 1)
}

fn now() -> Timestamp {
    Timestamp::now()
}

fn table_name(module: &str) -> String {
    format!("{module}_throttle")
}

/// Migration that creates the table `<module>_throttle`. A module that uses [`Throttle`] adds it to its list of
/// migrations.
pub fn throttle_migration(name: &'static str, module: &'static str) -> Box<dyn MigrationTrait> {
    Box::new(ThrottleMigration { name, module })
}

struct ThrottleMigration {
    name: &'static str,
    module: &'static str,
}

impl MigrationName for ThrottleMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for ThrottleMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = table_name(self.module);
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(&table))
                    // SHA-256 (base64url) của khoá đếm.
                    .col(schema::string_len("key", 64).primary_key())
                    .col(schema::integer("count"))
                    .col(schema::timestamp_with_time_zone("window_started_at"))
                    .col(schema::timestamp_with_time_zone_null("locked_until"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name(format!("{table}_window_idx"))
                    .table(Alias::new(&table))
                    .col("window_started_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new(table_name(self.module))).to_owned())
            .await
    }
}

/// The table `<module>_throttle` after upgrading per tenant (step 33 A4): primary key `(tenant_id, key)`;
/// cleanup index (all tenants) `…_window_sweep_idx`. Added to the module's migration like
/// [`event::tenant_table`](crate::event::tenant_table).
pub fn tenant_table(module: &str) -> crate::tenancy::migration::TenantTable {
    let table = table_name(module);
    crate::tenancy::migration::TenantTable::new(&table)
        .primary_key(["tenant_id", "key"])
        .sweep_index(format!("{table}_window_sweep_idx"), ["window_started_at"])
        .drop_index(format!("{table}_window_idx"))
}
