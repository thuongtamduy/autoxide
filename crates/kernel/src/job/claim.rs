//! Job claiming: several workers take jobs at once without anyone taking the same one.
//!
//! The two databases do this differently (D-009), so it is hidden behind the [`Claim`] trait:
//!
//! - **Postgres** allows many write connections at once: it picks jobs with
//!   `SELECT … FOR UPDATE SKIP LOCKED` (skipping rows locked by other workers) inside an
//!   `UPDATE … RETURNING` statement. Workers take different jobs and nobody waits for anyone.
//! - **SQLite** has only one writer at a time: a single `UPDATE … WHERE id IN (SELECT … LIMIT n) RETURNING`
//!   statement is already atomic.
//!
//! The gate runs the concurrent claim test on both databases, so both branches are tested.
use crate::time::Timestamp;
use sea_orm::sea_query::{
    Alias, Condition, Expr, ExprTrait as _, LockBehavior, LockType, Order, Query, SelectStatement,
    UpdateStatement,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection};
use uuid::Uuid;

use crate::error::Error;
use crate::tenancy::TenantId;

/// A job that was just claimed.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ClaimedJob {
    /// The job's tenant ([`NO_TENANT`](crate::tenancy::NO_TENANT): a job of the whole database).
    pub tenant: TenantId,
    pub id: Uuid,
    pub kind: String,
    pub payload: String,
    /// Including this attempt.
    pub attempts: i32,
    pub max_attempts: i32,
}

/// One claim: at most `limit` pending jobs from table `table` (only of tenant `tenant` if given) for worker
/// `worker`, held until `until`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ClaimRequest<'a> {
    pub table: &'a str,
    pub tenant: Option<TenantId>,
    pub worker: &'a str,
    pub now: Timestamp,
    pub until: Timestamp,
    pub limit: u64,
}

impl<'a> ClaimRequest<'a> {
    /// Up to `limit` due jobs of `table` (any tenant) for `worker`, held until `until`.
    pub fn new(
        table: &'a str,
        worker: &'a str,
        now: Timestamp,
        until: Timestamp,
        limit: u64,
    ) -> Self {
        ClaimRequest { table, tenant: None, worker, now, until, limit }
    }

    /// Only jobs of `tenant`.
    pub const fn tenant(mut self, tenant: TenantId) -> Self {
        self.tenant = Some(tenant);
        self
    }
}

/// How to claim jobs for one database.
#[async_trait::async_trait]
pub trait Claim: Send + Sync {
    /// Claims per `request`. Each claimed job has `attempts` increased by one.
    async fn claim(
        &self,
        connection: &DatabaseConnection,
        request: &ClaimRequest<'_>,
    ) -> Result<Vec<ClaimedJob>, Error>;
}

/// The Postgres claim: `FOR UPDATE SKIP LOCKED`.
pub struct PostgresClaim;

/// The SQLite claim: a single `UPDATE … RETURNING` statement (SQLite has only one writer).
pub struct SqliteClaim;

/// The correct claim method for `backend`.
pub fn claim_for(backend: DatabaseBackend) -> Box<dyn Claim> {
    match backend {
        DatabaseBackend::Postgres => Box::new(PostgresClaim),
        _ => Box::new(SqliteClaim),
    }
}

#[async_trait::async_trait]
impl Claim for PostgresClaim {
    async fn claim(
        &self,
        connection: &DatabaseConnection,
        request: &ClaimRequest<'_>,
    ) -> Result<Vec<ClaimedJob>, Error> {
        let mut candidates = candidates(request);
        candidates.lock_with_behavior(LockType::Update, LockBehavior::SkipLocked);
        run(connection, update(request, candidates)).await
    }
}

#[async_trait::async_trait]
impl Claim for SqliteClaim {
    async fn claim(
        &self,
        connection: &DatabaseConnection,
        request: &ClaimRequest<'_>,
    ) -> Result<Vec<ClaimedJob>, Error> {
        run(connection, update(request, candidates(request))).await
    }
}

/// The "pending run" condition: not finished, not permanently failed, due, and not held by anyone.
pub(crate) fn pending(now: Timestamp) -> Condition {
    Condition::all()
        .add(Expr::col(Alias::new("completed_at")).is_null())
        .add(Expr::col(Alias::new("failed_at")).is_null())
        .add(Expr::col(Alias::new("run_at")).lte(now))
        // Đã chạy đủ số lần mà chưa ghi được kết quả (tiến trình chết giữa chừng — hết bộ nhớ,
        // bị kill): không lấy lại nữa; lượt dọn đưa nó vào dead-letter queue (D-158).
        .add(Expr::col(Alias::new("attempts")).lt(Expr::col(Alias::new("max_attempts"))))
        .add(
            Condition::any()
                .add(Expr::col(Alias::new("claimed_until")).is_null())
                .add(Expr::col(Alias::new("claimed_until")).lt(now)),
        )
}

fn candidates(request: &ClaimRequest<'_>) -> SelectStatement {
    let mut condition = pending(request.now);
    if let Some(tenant) = request.tenant {
        condition = condition.add(Expr::col(Alias::new("tenant_id")).eq(tenant.as_uuid()));
    }
    Query::select()
        .column(Alias::new("id"))
        .from(Alias::new(request.table))
        .cond_where(condition)
        .order_by(Alias::new("run_at"), Order::Asc)
        .order_by(Alias::new("id"), Order::Asc)
        .limit(request.limit)
        .to_owned()
}

fn update(request: &ClaimRequest<'_>, candidates: SelectStatement) -> UpdateStatement {
    Query::update()
        .table(Alias::new(request.table))
        .value(Alias::new("claimed_by"), request.worker)
        .value(Alias::new("claimed_until"), request.until)
        .value(Alias::new("attempts"), Expr::col(Alias::new("attempts")).add(1))
        .and_where(Expr::col(Alias::new("id")).in_subquery(candidates))
        .returning(Query::returning().columns([
            Alias::new("tenant_id"),
            Alias::new("id"),
            Alias::new("kind"),
            Alias::new("payload"),
            Alias::new("attempts"),
            Alias::new("max_attempts"),
        ]))
        .to_owned()
}

async fn run(
    connection: &DatabaseConnection,
    statement: UpdateStatement,
) -> Result<Vec<ClaimedJob>, Error> {
    let backend = connection.get_database_backend();
    let rows = connection.query_all_raw(backend.build(&statement)).await?;
    rows.iter()
        .map(|row| {
            Ok(ClaimedJob {
                tenant: TenantId::from_uuid(row.try_get("", "tenant_id")?),
                id: row.try_get("", "id")?,
                kind: row.try_get("", "kind")?,
                payload: row.try_get("", "payload")?,
                attempts: row.try_get("", "attempts")?,
                max_attempts: row.try_get("", "max_attempts")?,
            })
        })
        .collect()
}
