//! Row locks and conditional updates: one way for every module when several requests edit the same row
//! (D-181).
//!
//! Two tools, behaving the same on SQLite and Postgres:
//!
//! - [`lock`] / [`lock_one`]: read rows and **lock them until the write transaction ends**. Postgres:
//!   `SELECT … FOR UPDATE`; another transaction that wants to lock or edit the same row waits for this one
//!   to finish, then sees the new value. SQLite: a plain `SELECT` is enough, because every write transaction
//!   opened with [`Database::begin_write`](crate::database::Database::begin_write) (`BEGIN
//!   IMMEDIATE`) holds the write permission on the whole database from start to end, so nobody can interleave.
//!   Use it when you must read, check, then write (loop check in a tree, check a permission is still valid…).
//! - [`adjust`]: **conditional add / subtract in a single statement**: `UPDATE … SET c = c + Δ WHERE …`
//!   with a condition that the result stays within bounds, then count the rows changed. This is the standard
//!   way to deduct a balance: no prior read, so no gap between read and write; the balance never drops below
//!   the floor and never overflows `i64` (SQLite silently turns an overflowing integer into a float;
//!   Postgres raises an error; guarding with the condition makes both behave the same).
//!
//! - [`lock_key`]: lock by name until the write transaction ends, when there is no stable row to lock.
//!
//! Lock order against deadlock: when a transaction locks several rows, always lock in the same order (e.g.
//! ascending ID; [`lock`] does so when given an order column), and lock before writing. Two transactions that
//! lock A then B and B then A would wait for each other forever; Postgres detects it and cancels one side
//! with error `40P01` (see [`Database::write_with_retry`](crate::database::Database::write_with_retry)).

use sea_orm::sea_query::{Expr, ExprTrait as _, IntoCondition};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Select,
};
use sha2::{Digest as _, Sha256};

/// Reads and locks every row that `select` picks, until the end of `transaction`. `order` is the sort column
/// used while locking (usually the primary key): several transactions that lock the same group of rows in
/// the same order do not deadlock. `transaction` must be a WRITE transaction (`Database::begin_write`): on
/// Postgres a read-only transaction rejects `FOR UPDATE`, and outside a transaction the lock is released
/// right after the statement (it takes the generic `ConnectionTrait` type so the function works with both
/// `&DatabaseTransaction` and the `&C` of its caller).
pub async fn lock<E: EntityTrait, C: ConnectionTrait>(
    transaction: &C,
    select: Select<E>,
    order: E::Column,
) -> Result<Vec<E::Model>, DbErr> {
    // `lock_exclusive` sinh `FOR UPDATE` trên Postgres và không sinh gì trên SQLite (thư viện
    // bỏ qua mệnh đề khoá ở SQLite) — xem tài liệu đầu file vì sao vậy là đủ.
    select.order_by_asc(order).lock_exclusive().all(transaction).await
}

/// Reads and locks the row that `select` picks (at most one row, e.g. `Entity::find_by_id(id)` with a
/// condition), until the end of `transaction`. `None`: no row matches, including when the row was just
/// changed by another transaction so it no longer matches (Postgres waits for that transaction to finish,
/// then re-evaluates the condition).
pub async fn lock_one<E: EntityTrait, C: ConnectionTrait>(
    transaction: &C,
    select: Select<E>,
) -> Result<Option<E::Model>, DbErr> {
    select.lock_exclusive().one(transaction).await
}

/// Reads the row that `select` picks (at most one) and holds a **shared lock** until the end of `transaction`
/// (step 36): several transactions can hold a shared lock on one row, but a party that wants [`lock_one`]
/// (exclusive lock) or to edit that row waits for them to finish. For example every journal entry holds a
/// shared lock on the accounting period row while closing the books takes an exclusive lock: closing waits
/// for entries being written, and a new entry waits for the close to finish, then sees the period locked.
/// Postgres: `SELECT … FOR SHARE`. SQLite: a plain `SELECT` (every write transaction already holds the write
/// permission on the whole database; see the top of the file).
pub async fn lock_one_shared<E: EntityTrait, C: ConnectionTrait>(
    transaction: &C,
    select: Select<E>,
) -> Result<Option<E::Model>, DbErr> {
    select.lock_shared().one(transaction).await
}

/// Locks **by name** until the end of the write `transaction`: when two transactions lock the same `key`,
/// the later one waits for the earlier one to commit / roll back. Use it when the work to serialize is not
/// tied to an existing row, e.g. "every recompute of (tenant, language)" when no row exists yet, or when rows
/// are deleted / re-inserted (ID changes) midway so a row lock no longer reliably hits. After taking the
/// lock, re-read everything you need to check: a fresh read sees what earlier transactions committed.
///
/// Postgres: `pg_advisory_xact_lock` with a 64-bit number taken from the SHA-256 of `key` (a collision
/// between two different names only causes an unneeded wait, not a wrong result). SQLite: does nothing; the
/// write transaction opened with [`Database::begin_write`](crate::database::Database::begin_write)
/// (`BEGIN IMMEDIATE`) already holds the write permission on the whole database. `transaction` must be a
/// WRITE transaction (outside a transaction, the Postgres lock is released right after the statement). A
/// transaction that holds several locks always takes them in the same order (e.g. ascending name) to avoid
/// deadlock. Prefix the name with the module (`catalog.similar:<…>`).
pub async fn lock_key<C: ConnectionTrait>(transaction: &C, key: &str) -> Result<(), DbErr> {
    let backend = transaction.get_database_backend();
    if backend != DbBackend::Postgres {
        return Ok(());
    }
    transaction
        .execute_raw(crate::database::statement(
            backend,
            "SELECT pg_advisory_xact_lock($1)",
            [key_number(key).into()],
        ))
        .await?;
    Ok(())
}

/// Advisory lock number of `key`: the first 8 bytes of its SHA-256, identical on every machine and version.
fn key_number(key: &str) -> i64 {
    let digest = Sha256::digest(key.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(bytes)
}

/// Bounds for [`adjust`]: the column value AFTER the addition must be within `[floor, ceiling]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub floor: i64,
    pub ceiling: i64,
}

impl Bounds {
    /// Non-negative: `[0, i64::MAX]`; the balance must not drop below 0.
    pub const NON_NEGATIVE: Bounds = Bounds { floor: 0, ceiling: i64::MAX };
    /// Overflow guard only: `[i64::MIN, i64::MAX]` (e.g. a balance allowed to go negative down to its own
    /// limit; then set `floor` to that limit).
    pub const ANY: Bounds = Bounds { floor: i64::MIN, ceiling: i64::MAX };
}

/// Adds `delta` (negative to subtract) to the integer column `column` of the rows matching `filter`, only on
/// rows where the result stays within `bounds`. Returns the number of rows changed; for one row by ID, `1`
/// means deducted / added and `0` means the row does not exist or the amount is insufficient (the service
/// tells them apart if needed, e.g. by re-reading).
///
/// A single `UPDATE` statement, no prior read: on Postgres two concurrent deductions on that row queue up
/// (the later request re-evaluates the condition on the new value); on SQLite there is only one writer anyway.
///
/// ```ignore
/// // Deduct 500 from the balance of wallet `id`, not allowing negative.
/// let changed = rows::adjust::<wallet::Entity, _>(
///     &transaction, wallet::Column::BalanceMinor, wallet::Column::Id.eq(id), -500,
///     Bounds::NON_NEGATIVE,
/// ).await?;
/// if changed == 0 { return Err(insufficient_funds()); }
/// ```
pub async fn adjust<E, C>(
    connection: &C,
    column: E::Column,
    filter: impl IntoCondition,
    delta: i64,
    bounds: Bounds,
) -> Result<u64, DbErr>
where
    E: EntityTrait,
    C: ConnectionTrait,
{
    // Điều kiện `floor <= c + Δ <= ceiling` viết lại thành `floor − Δ <= c <= ceiling − Δ`, để
    // database không phải tính `c + Δ` (có thể tràn) khi xét điều kiện. Tính giới hạn bằng
    // `i128` rồi bỏ vế nào vượt khỏi `i64` — vế đó luôn đúng với mọi giá trị `i64` của cột.
    let low = i128::from(bounds.floor) - i128::from(delta);
    let high = i128::from(bounds.ceiling) - i128::from(delta);
    if low > high || low > i128::from(i64::MAX) || high < i128::from(i64::MIN) {
        return Ok(0);
    }
    let mut update = E::update_many()
        .col_expr(column, Expr::col(column).add(delta))
        .filter(filter.into_condition());
    if let Ok(low) = i64::try_from(low) {
        update = update.filter(column.gte(low));
    }
    if let Ok(high) = i64::try_from(high) {
        update = update.filter(column.lte(high));
    }
    Ok(update.exec(connection).await?.rows_affected)
}
