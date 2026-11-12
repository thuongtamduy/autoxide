//! Claim job: nhiều worker cùng lấy job mà không ai lấy trùng.
//!
//! Hai database làm việc này khác nhau (D-009), nên giấu sau trait [`Claim`]:
//!
//! - **Postgres** cho nhiều kết nối ghi cùng lúc: chọn job bằng
//!   `SELECT … FOR UPDATE SKIP LOCKED` (bỏ qua dòng worker khác đang khoá) trong câu
//!   `UPDATE … RETURNING`. Các worker lấy những job khác nhau, không ai phải chờ ai.
//! - **SQLite** chỉ có một bên ghi tại một thời điểm: một câu
//!   `UPDATE … WHERE id IN (SELECT … LIMIT n) RETURNING` đã là nguyên tử.
//!
//! Gate chạy test claim đồng thời trên cả hai database, nên cả hai nhánh đều được test.

use chrono::{DateTime, Utc};
use sea_orm::sea_query::{
    Alias, Condition, Expr, ExprTrait as _, LockBehavior, LockType, Order, Query, SelectStatement,
    UpdateStatement,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection};
use uuid::Uuid;

use crate::error::Error;

/// Một job vừa được claim.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimedJob {
    pub id: Uuid,
    pub kind: String,
    pub payload: String,
    /// Đã tính cả lần này.
    pub attempts: i32,
    pub max_attempts: i32,
}

/// Cách claim job của một database.
#[async_trait::async_trait]
pub trait Claim: Send + Sync {
    /// Claim tối đa `limit` job đang chờ của bảng `table` cho worker `worker`, giữ tới
    /// `until`. Mỗi job claim được tăng `attempts` thêm một.
    async fn claim(
        &self,
        connection: &DatabaseConnection,
        table: &str,
        worker: &str,
        now: DateTime<Utc>,
        until: DateTime<Utc>,
        limit: u64,
    ) -> Result<Vec<ClaimedJob>, Error>;
}

/// Claim của Postgres: `FOR UPDATE SKIP LOCKED`.
pub struct PostgresClaim;

/// Claim của SQLite: một câu `UPDATE … RETURNING` (SQLite chỉ có một bên ghi).
pub struct SqliteClaim;

/// Cách claim đúng cho `backend`.
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
        table: &str,
        worker: &str,
        now: DateTime<Utc>,
        until: DateTime<Utc>,
        limit: u64,
    ) -> Result<Vec<ClaimedJob>, Error> {
        let mut candidates = candidates(table, now, limit);
        candidates.lock_with_behavior(LockType::Update, LockBehavior::SkipLocked);
        run(connection, update(table, worker, until, candidates)).await
    }
}

#[async_trait::async_trait]
impl Claim for SqliteClaim {
    async fn claim(
        &self,
        connection: &DatabaseConnection,
        table: &str,
        worker: &str,
        now: DateTime<Utc>,
        until: DateTime<Utc>,
        limit: u64,
    ) -> Result<Vec<ClaimedJob>, Error> {
        run(connection, update(table, worker, until, candidates(table, now, limit))).await
    }
}

/// Điều kiện "đang chờ chạy": chưa xong, chưa hỏng hẳn, tới giờ, không ai đang giữ.
pub(crate) fn pending(now: DateTime<Utc>) -> Condition {
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

fn candidates(table: &str, now: DateTime<Utc>, limit: u64) -> SelectStatement {
    Query::select()
        .column(Alias::new("id"))
        .from(Alias::new(table))
        .cond_where(pending(now))
        .order_by(Alias::new("run_at"), Order::Asc)
        .order_by(Alias::new("id"), Order::Asc)
        .limit(limit)
        .to_owned()
}

fn update(
    table: &str,
    worker: &str,
    until: DateTime<Utc>,
    candidates: SelectStatement,
) -> UpdateStatement {
    Query::update()
        .table(Alias::new(table))
        .value(Alias::new("claimed_by"), worker)
        .value(Alias::new("claimed_until"), until)
        .value(Alias::new("attempts"), Expr::col(Alias::new("attempts")).add(1))
        .and_where(Expr::col(Alias::new("id")).in_subquery(candidates))
        .returning(Query::returning().columns([
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
                id: row.try_get("", "id")?,
                kind: row.try_get("", "kind")?,
                payload: row.try_get("", "payload")?,
                attempts: row.try_get("", "attempts")?,
                max_attempts: row.try_get("", "max_attempts")?,
            })
        })
        .collect()
}
