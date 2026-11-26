//! Writes, on a write transaction only: `insert` (with `ON CONFLICT`), `update` (`set`, bounded `adjust`),
//! `delete`.

use std::marker::PhantomData;

use sea_orm::sea_query::{self, Alias, ExprTrait as _, OnConflict, Query, ReturningClause};
use sea_orm::{DbErr, SqlErr};

use super::handle::{AcceptedBy, Access, Handle, Tx, tenant_of_handle};
use super::record::{Col, ColName, Cond, Expr, Record, Updated, table, tenant_filter};
use super::value::Settable;
use crate::error::Error;

/// The error code of a unique violation: two rows with the same value of a unique index (including, on a
/// tenant handle, a row of another tenant — reported the same way, so the other tenant stays invisible).
pub const UNIQUE_VIOLATION: &str = "db.unique_violation";

/// Whether `error` is a unique violation of an `insert` / `update` ([`UNIQUE_VIOLATION`]); the service turns
/// it into its own code (`notes.title_taken`).
pub fn is_unique_violation(error: &Error) -> bool {
    error.code() == UNIQUE_VIOLATION
}

fn unique_violation() -> Error {
    // Mã viết thẳng (không qua hằng) để `cargo xtask check` thấy nó có bản dịch.
    Error::conflict("db.unique_violation", "A record with the same unique value already exists.")
}

// Lỗi của database khi ghi: vi phạm unique thành mã ổn định cho module; lỗi khác giữ nguyên (lỗi nội bộ).
fn write_error(error: DbErr) -> Error {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => unique_violation().with_source(error),
        _ => Error::from(error),
    }
}

/// Bounds for [`Update::adjust`]: the value AFTER the change stays within `[floor, ceiling]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Bounds {
    /// Lowest value allowed after the change.
    pub floor: i64,
    /// Highest value allowed after the change.
    pub ceiling: i64,
}

impl Bounds {
    /// `[0, i64::MAX]`: never below zero (a stock, a balance).
    pub const NON_NEGATIVE: Bounds = Bounds { floor: 0, ceiling: i64::MAX };
    /// `[i64::MIN, i64::MAX]`: only the overflow guard.
    pub const ANY: Bounds = Bounds { floor: i64::MIN, ceiling: i64::MAX };

    /// `[floor, ceiling]`.
    pub const fn new(floor: i64, ceiling: i64) -> Self {
        Bounds { floor, ceiling }
    }
}

impl<A: Access> Tx<A> {
    /// `INSERT` one row. On a tenant handle, a row whose `tenant_id` is not the handle's tenant is refused
    /// before anything is sent.
    pub fn insert<R: Record>(&self, row: R) -> Insert<'_, A, R>
    where
        R::Scope: AcceptedBy<A>,
    {
        self.insert_many(std::iter::once(row))
    }

    /// `INSERT` several rows in one statement (an empty list does nothing).
    pub fn insert_many<R: Record>(&self, rows: impl IntoIterator<Item = R>) -> Insert<'_, A, R>
    where
        R::Scope: AcceptedBy<A>,
    {
        let tenant = tenant_of_handle::<Self, R>(self);
        let mut statement = Query::insert();
        statement.into_table(table::<R>()).columns(R::COLUMNS.iter().map(|c| Alias::new(*c)));
        let mut error = None;
        let mut empty = true;
        for row in rows {
            empty = false;
            if let (Some(tenant), Some(owner)) = (tenant, row.tenant_of())
                && owner != tenant
            {
                error.get_or_insert_with(|| {
                    Error::internal(format!(
                        "insert into `{}`: the row belongs to another tenant",
                        R::TABLE
                    ))
                });
            }
            let values: Vec<sea_query::Expr> =
                row.values().into_iter().map(|value| sea_query::Expr::val(value.0)).collect();
            // Chỉ lỗi khi số giá trị khác số cột; derive sinh cả hai từ cùng danh sách trường.
            if let Err(failure) = statement.values(values) {
                error.get_or_insert_with(|| Error::internal(failure.to_string()));
            }
        }
        Insert { tx: self, statement, error, empty, tenant, guarded: false, _marker: PhantomData }
    }

    /// `UPDATE` rows of `R` (filtered by tenant on a tenant handle); needs at least one `set`.
    pub fn update<R: Record>(&self) -> Update<'_, A, R>
    where
        R::Scope: AcceptedBy<A>,
    {
        let mut statement = Query::update();
        statement.table(table::<R>());
        if let Some(tenant) = tenant_of_handle::<Self, R>(self) {
            statement.and_where(tenant_filter::<R>(tenant));
        }
        Update { tx: self, statement, has_set: false, _marker: PhantomData }
    }

    /// `DELETE` rows of `R` (filtered by tenant on a tenant handle).
    pub fn delete<R: Record>(&self) -> Delete<'_, A, R>
    where
        R::Scope: AcceptedBy<A>,
    {
        let mut statement = Query::delete();
        statement.from_table(table::<R>());
        if let Some(tenant) = tenant_of_handle::<Self, R>(self) {
            statement.and_where(tenant_filter::<R>(tenant));
        }
        Delete { tx: self, statement, _marker: PhantomData }
    }
}

/// An `INSERT`, see [`Tx::insert`].
pub struct Insert<'t, A, R> {
    tx: &'t Tx<A>,
    statement: sea_query::InsertStatement,
    error: Option<Error>,
    empty: bool,
    tenant: Option<uuid::Uuid>,
    guarded: bool,
    _marker: PhantomData<fn() -> R>,
}

impl<A: Access, R: Record> Insert<'_, A, R> {
    /// `ON CONFLICT (target) DO NOTHING`: a row that collides with an existing one is skipped (count 0).
    #[must_use]
    pub fn on_conflict_do_nothing<const N: usize>(mut self, target: [ColName<R>; N]) -> Self {
        let mut conflict = OnConflict::columns(target.iter().map(|c| Alias::new(c.name)));
        conflict.do_nothing();
        self.statement.on_conflict(conflict);
        self
    }

    /// `ON CONFLICT (target) DO UPDATE SET <update> = excluded.<update>`. On a tenant handle the kernel adds
    /// the condition "the existing row belongs to this tenant": a unique key without `tenant_id` can never
    /// let one tenant overwrite another tenant's row — such a collision is [`UNIQUE_VIOLATION`]. `update`
    /// lists settable columns (`R::X.updated()`), so `tenant_id` cannot be in it:
    ///
    /// ```compile_fail,E0599
    /// # use autoxide_kernel::db::{Record, TenantKey, TenantTx};
    /// #[derive(Record)]
    /// #[record(table = "kernel_doc_note", scope = tenant)]
    /// struct Note { id: uuid::Uuid, tenant_id: TenantKey, code: String }
    /// fn upsert(tx: &TenantTx, note: Note) {
    ///     let _ = tx.insert(note).on_conflict_update([Note::CODE.key()], [Note::TENANT_ID.updated()]);
    /// }
    /// ```
    ///
    /// Control (one token differs, it compiles):
    ///
    /// ```no_run
    /// # use autoxide_kernel::db::{Record, TenantKey, TenantTx};
    /// #[derive(Record)]
    /// #[record(table = "kernel_doc_note", scope = tenant)]
    /// struct Note { id: uuid::Uuid, tenant_id: TenantKey, code: String }
    /// fn upsert(tx: &TenantTx, note: Note) {
    ///     let _ = tx.insert(note).on_conflict_update([Note::CODE.key()], [Note::CODE.updated()]);
    /// }
    /// ```
    #[must_use]
    pub fn on_conflict_update<const N: usize, const M: usize>(
        mut self,
        target: [ColName<R>; N],
        update: [Updated<R>; M],
    ) -> Self {
        let mut conflict = OnConflict::columns(target.iter().map(|c| Alias::new(c.name)));
        conflict.update_columns(update.iter().map(|c| Alias::new(c.name)));
        if let Some(tenant) = self.tenant {
            conflict.action_and_where(tenant_filter::<R>(tenant));
            self.guarded = true;
        }
        self.statement.on_conflict(conflict);
        self
    }

    /// Runs it; the number of rows inserted or updated.
    pub async fn exec(self) -> Result<u64, Error> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if self.empty {
            return Ok(0);
        }
        let connection = self.tx.connection();
        let changed =
            connection.execute(connection.build(&self.statement)).await.map_err(write_error)?;
        if changed == 0 && self.guarded {
            // `DO UPDATE … WHERE tenant_id = <khách>` không đổi dòng nào: dòng đụng thuộc khách khác.
            return Err(unique_violation());
        }
        Ok(changed)
    }
}

/// An `UPDATE`, see [`Tx::update`].
pub struct Update<'t, A, R> {
    tx: &'t Tx<A>,
    statement: sea_query::UpdateStatement,
    has_set: bool,
    _marker: PhantomData<fn() -> R>,
}

impl<A: Access, R: Record> Update<'_, A, R> {
    /// `SET column = value`. Only [`Settable`] values: the tenant column is not, so it cannot move a row
    /// to another tenant:
    ///
    /// ```compile_fail,E0277
    /// # use autoxide_kernel::db::{Record, TenantKey, TenantTx};
    /// #[derive(Record)]
    /// #[record(table = "kernel_doc_note", scope = tenant)]
    /// struct Note { id: uuid::Uuid, tenant_id: TenantKey }
    /// fn change(tx: &TenantTx, other: TenantKey) { let _ = tx.update::<Note>().set(Note::TENANT_ID, other); }
    /// ```
    ///
    /// Control (one token differs, it compiles):
    ///
    /// ```no_run
    /// # use autoxide_kernel::db::{Record, TenantKey, TenantTx};
    /// #[derive(Record)]
    /// #[record(table = "kernel_doc_note", scope = tenant)]
    /// struct Note { id: uuid::Uuid, tenant_id: TenantKey }
    /// fn change(tx: &TenantTx, other: uuid::Uuid) { let _ = tx.update::<Note>().set(Note::ID, other); }
    /// ```
    #[must_use]
    pub fn set<T: Settable>(mut self, column: Col<R, T>, value: T) -> Self {
        self.statement.value(Alias::new(column.name()), value.into_value().0);
        self.has_set = true;
        self
    }

    /// `SET column = expr` (e.g. `R::COUNT.expr() + Expr::value(1)`).
    #[must_use]
    pub fn set_expr<T: Settable>(mut self, column: Col<R, T>, expr: Expr<R, T>) -> Self {
        self.statement.value(Alias::new(column.name()), expr.inner);
        self.has_set = true;
        self
    }

    /// `SET column = column + delta`, only on rows where the result stays within `bounds` (no prior read, no
    /// overflow; the same on both databases: an out-of-bounds or overflowing change updates 0 rows).
    #[must_use]
    pub fn adjust(mut self, column: Col<R, i64>, delta: i64, bounds: Bounds) -> Self {
        // `floor <= c + Δ <= ceiling` viết lại thành `floor − Δ <= c <= ceiling − Δ` để database không phải tính
        // `c + Δ` (có thể tràn) khi xét điều kiện; vế vượt khỏi `i64` luôn đúng nên bỏ.
        let low = i128::from(bounds.floor) - i128::from(delta);
        let high = i128::from(bounds.ceiling) - i128::from(delta);
        if low > high || low > i128::from(i64::MAX) || high < i128::from(i64::MIN) {
            self.statement.and_where(sea_query::Expr::val(1).eq(0));
        }
        if let Ok(low) = i64::try_from(low) {
            self.statement.and_where(column.sq().gte(low));
        }
        if let Ok(high) = i64::try_from(high) {
            self.statement.and_where(column.sq().lte(high));
        }
        self.statement.value(
            Alias::new(column.name()),
            sea_query::Expr::col(Alias::new(column.name())).add(delta),
        );
        self.has_set = true;
        self
    }

    /// Adds a condition (AND).
    #[must_use]
    pub fn filter(mut self, condition: Cond<R>) -> Self {
        self.statement.cond_where(condition.inner);
        self
    }

    /// Runs it; the number of rows changed.
    pub async fn exec(self) -> Result<u64, Error> {
        if !self.has_set {
            return Err(Error::internal("`update` without `set`"));
        }
        let connection = self.tx.connection();
        connection.execute(connection.build(&self.statement)).await.map_err(write_error)
    }

    /// Runs it and reads the changed rows back (`RETURNING`, on both databases).
    pub async fn returning(mut self) -> Result<Vec<R>, Error> {
        if !self.has_set {
            return Err(Error::internal("`update` without `set`"));
        }
        self.statement.returning(ReturningClause::Columns(
            R::COLUMNS.iter().map(|column| Alias::new(*column).into()).collect(),
        ));
        let connection = self.tx.connection();
        let rows = connection.all(connection.build(&self.statement)).await.map_err(write_error)?;
        rows.iter().map(|row| R::read(&super::Row(row))).collect()
    }
}

/// A `DELETE`, see [`Tx::delete`].
pub struct Delete<'t, A, R> {
    tx: &'t Tx<A>,
    statement: sea_query::DeleteStatement,
    _marker: PhantomData<fn() -> R>,
}

impl<A: Access, R: Record> Delete<'_, A, R> {
    /// Adds a condition (AND).
    #[must_use]
    pub fn filter(mut self, condition: Cond<R>) -> Self {
        self.statement.cond_where(condition.inner);
        self
    }

    /// Runs it; the number of rows deleted.
    pub async fn exec(self) -> Result<u64, Error> {
        let connection = self.tx.connection();
        Ok(connection.execute(connection.build(&self.statement)).await?)
    }
}
