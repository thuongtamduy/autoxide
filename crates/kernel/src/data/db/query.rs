//! Reads: `find`, `select` + `group_by`, subqueries, keyset pages.

use std::marker::PhantomData;

use sea_orm::sea_query::{self, LockType, Query, SelectStatement};
use uuid::Uuid;

use super::handle::{AcceptedBy, Handle, tenant_of_handle};
use super::record::{Col, Cond, On, Record, Selection, count, table, tenant_filter};
use super::value::{DbValue, FromRow, Row};
use crate::error::Error;
use crate::pagination::{Order, Page, PageRequest, Pagination};

/// Query entry points, on every handle (`use autoxide_kernel::db::Queries as _;`).
pub trait Queries: Handle + Sized {
    /// `SELECT` rows of `R`. On a tenant handle a tenant record is filtered by the tenant; a tenant record
    /// on a control-plane handle does not compile ([`AcceptedBy`]).
    fn find<R: Record>(&self) -> Find<'_, Self, R>
    where
        R::Scope: AcceptedBy<Self::Access>,
    {
        let mut statement = Query::select();
        statement.from(table::<R>());
        for column in R::COLUMNS {
            statement.expr(super::record::qualified::<R>(column));
        }
        if let Some(tenant) = tenant_of_handle::<Self, R>(self) {
            statement.and_where(tenant_filter::<R>(tenant));
        }
        Find { handle: self, statement, joined: Vec::new(), _marker: PhantomData }
    }

    /// A subquery selecting `column` of `S` (for `Col::in_sub`), filtered by tenant like `find`.
    fn sub<S: Record, T: DbValue>(&self, column: Col<S, T>) -> Sub<S, T>
    where
        S::Scope: AcceptedBy<Self::Access>,
    {
        let mut statement = Query::select();
        statement.from(table::<S>()).expr(column.sq());
        if let Some(tenant) = tenant_of_handle::<Self, S>(self) {
            statement.and_where(tenant_filter::<S>(tenant));
        }
        Sub { statement, _marker: PhantomData }
    }

    /// Hand-written SQL with named parameters (`:name`); see [`Sql`](super::Sql).
    fn sql(&self, text: &str) -> super::sql::Sql<'_, Self> {
        super::sql::Sql::new(self, text)
    }
}

impl<H: Handle> Queries for H {}

/// A subquery built from a handle (already filtered by tenant), for `Col::in_sub`.
pub struct Sub<S, T> {
    pub(crate) statement: SelectStatement,
    _marker: PhantomData<fn() -> (S, T)>,
}

impl<S: Record, T> Sub<S, T> {
    /// Adds a condition.
    #[must_use]
    pub fn filter(mut self, condition: Cond<S>) -> Self {
        self.statement.cond_where(condition.inner);
        self
    }
}

/// A `SELECT` of rows of `R`, built step by step.
pub struct Find<'h, H, R> {
    handle: &'h H,
    statement: SelectStatement,
    joined: Vec<&'static str>,
    _marker: PhantomData<fn() -> R>,
}

impl<'h, H: Handle, R: Record> Find<'h, H, R> {
    /// Adds a condition (AND).
    #[must_use]
    pub fn filter(mut self, condition: Cond<R>) -> Self {
        self.statement.cond_where(condition.inner);
        self
    }

    /// `INNER JOIN S ON …`. `S` is a table of the same module (the derive refuses records of other modules'
    /// tables) and is filtered by tenant too.
    #[must_use]
    pub fn join<S: Record>(mut self, on: On<R, S>) -> Self
    where
        S::Scope: AcceptedBy<H::Access>,
    {
        let mut condition = sea_query::Condition::all().add(on.expr);
        if let Some(tenant) = tenant_of_handle::<H, S>(self.handle) {
            condition = condition.add(tenant_filter::<S>(tenant));
        }
        self.statement.inner_join(table::<S>(), condition);
        self.joined.push(S::TABLE);
        self
    }

    /// A condition on a record joined with [`join`](Self::join).
    pub fn filter_joined<S: Record>(mut self, condition: Cond<S>) -> Result<Self, Error> {
        if !self.joined.contains(&S::TABLE) {
            return Err(Error::internal(format!("`{}` is filtered but not joined", S::TABLE)));
        }
        self.statement.cond_where(condition.inner);
        Ok(self)
    }

    /// `ORDER BY column ASC`.
    #[must_use]
    pub fn order_asc<T: DbValue>(mut self, column: Col<R, T>) -> Self {
        self.statement.order_by_expr(column.sq(), sea_query::Order::Asc);
        self
    }

    /// `ORDER BY column DESC`.
    #[must_use]
    pub fn order_desc<T: DbValue>(mut self, column: Col<R, T>) -> Self {
        self.statement.order_by_expr(column.sq(), sea_query::Order::Desc);
        self
    }

    /// At most `limit` rows. A list returned to a client is a [`page`](Self::page), never a bare limit.
    #[must_use]
    pub fn limit(mut self, limit: u64) -> Self {
        self.statement.limit(limit);
        self
    }

    /// Exclusive row lock until the write transaction ends. Postgres: `FOR UPDATE`. SQLite: nothing more is
    /// needed — a write transaction (`BEGIN IMMEDIATE`) already holds the whole database. Use it on a write
    /// transaction; lock rows in the same order everywhere to avoid deadlocks.
    #[must_use]
    pub fn for_update(mut self) -> Self {
        self.statement.lock(LockType::Update);
        self
    }

    /// Shared row lock until the write transaction ends (Postgres `FOR SHARE`; SQLite as
    /// [`for_update`](Self::for_update)): several transactions may hold it, a writer of the row waits.
    #[must_use]
    pub fn for_share(mut self) -> Self {
        self.statement.lock(LockType::Share);
        self
    }

    /// Every matching row.
    pub async fn all(self) -> Result<Vec<R>, Error> {
        read_all(self.handle, &self.statement, R::read).await
    }

    /// The first matching row.
    pub async fn one(mut self) -> Result<Option<R>, Error> {
        self.statement.limit(1);
        Ok(self.all().await?.into_iter().next())
    }

    /// Whether any row matches.
    pub async fn exists(self) -> Result<bool, Error> {
        Ok(self.select((count::<R>(),)).all().await?.first().is_some_and(|(n,)| *n > 0))
    }

    /// The number of matching rows.
    pub async fn count(self) -> Result<i64, Error> {
        Ok(self.select((count::<R>(),)).all().await?.first().map_or(0, |(n,)| *n))
    }

    /// Selects expressions instead of whole rows: `select((R::A, sum(R::B)))`, then `group_by`. Each row
    /// reads as the tuple of the selected types.
    pub fn select<S: Selection<R>>(mut self, selection: S) -> Grouped<'h, H, R, S::Out> {
        self.statement.clear_selects();
        for expr in selection.exprs() {
            self.statement.expr(expr);
        }
        Grouped { find: self, _marker: PhantomData }
    }

    /// One page ordered by the ID column `id` (UUIDv7, so newest first under `Order::Desc`), with the same
    /// keyset rules as the rest of the API (`limit`, `cursor`, `order`, optional `total`). The query must not
    /// already have an order or a limit.
    pub async fn page(self, id: Col<R, Uuid>, request: &PageRequest) -> Result<Page<R>, Error>
    where
        R: HasId,
    {
        let total = if request.wants_total() {
            let mut counted = self.statement.clone();
            counted.clear_selects();
            counted.expr(sea_query::Expr::cust("COUNT(*)"));
            let rows = read_all(self.handle, &counted, <(i64,)>::from_row).await?;
            Some(rows.first().map_or(0, |(n,)| u64::try_from(*n).unwrap_or_default()))
        } else {
            None
        };
        let mut statement = self.statement;
        match (request.id_cursor()?, request.order()) {
            (None, _) => {}
            (Some(cursor), Order::Asc) => {
                statement.cond_where(id.gt(cursor).inner);
            }
            (Some(cursor), Order::Desc) => {
                statement.cond_where(id.lt(cursor).inner);
            }
        }
        let direction = match request.order() {
            Order::Asc => sea_query::Order::Asc,
            Order::Desc => sea_query::Order::Desc,
        };
        statement.order_by_expr(id.sq(), direction);
        // Lấy dư MỘT dòng để biết còn trang sau mà không cần câu đếm.
        statement.limit(u64::from(request.limit()) + 1);
        let mut rows = read_all(self.handle, &statement, R::read).await?;
        let limit = request.limit() as usize;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let next_cursor = if has_more { rows.last().map(|row| row.id().to_string()) } else { None };
        Ok(Page {
            data: rows,
            pagination: Pagination::new(request.limit(), next_cursor).with_total(total),
        })
    }
}

/// A record whose primary key is a UUID column `id` (every module table: UUIDv7 ids, data rule).
pub trait HasId: Record {
    /// The id of the row.
    fn id(&self) -> Uuid;
}

/// A `select((…))` with optional `group_by`, read as tuples.
pub struct Grouped<'h, H, R, Out> {
    find: Find<'h, H, R>,
    _marker: PhantomData<fn() -> Out>,
}

impl<H: Handle, R: Record, Out: FromRow> Grouped<'_, H, R, Out> {
    /// `GROUP BY column`.
    #[must_use]
    pub fn group_by<T: DbValue>(mut self, column: Col<R, T>) -> Self {
        self.find.statement.add_group_by([column.sq()]);
        self
    }

    /// `ORDER BY column ASC`.
    #[must_use]
    pub fn order_asc<T: DbValue>(mut self, column: Col<R, T>) -> Self {
        self.find.statement.order_by_expr(column.sq(), sea_query::Order::Asc);
        self
    }

    /// `ORDER BY column DESC`.
    #[must_use]
    pub fn order_desc<T: DbValue>(mut self, column: Col<R, T>) -> Self {
        self.find.statement.order_by_expr(column.sq(), sea_query::Order::Desc);
        self
    }

    /// Every row.
    pub async fn all(self) -> Result<Vec<Out>, Error> {
        read_all(self.find.handle, &self.find.statement, Out::from_row).await
    }
}

pub(crate) async fn read_all<H: Handle, T>(
    handle: &H,
    statement: &SelectStatement,
    read: impl Fn(&Row<'_>) -> Result<T, Error>,
) -> Result<Vec<T>, Error> {
    let connection = handle.connection();
    let rows = connection.all(connection.build(statement)).await?;
    rows.iter().map(|row| read(&Row(row))).collect()
}
