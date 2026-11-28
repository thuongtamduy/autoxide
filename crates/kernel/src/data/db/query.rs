//! Reads: `find`, `select` + `group_by`, subqueries, keyset pages, row locks (write transactions only).

use std::marker::PhantomData;

use sea_orm::sea_query::{self, LockType, Query, SelectStatement};
use uuid::Uuid;

use super::handle::{AcceptedBy, Access, Handle, Tx, tenant_of_handle};
use super::record::{
    Col, Cond, On, Record, Scope as _, Selection, SubOrigin, Subs, count, foreign_sub,
    global_check, global_refused, table, tenant_filter,
};
use super::value::{DbValue, FromRow, Row, SortKey, UuidKey};
use crate::error::Error;
use crate::pagination::{Order, Page, PageRequest, Pagination, SortValue};

/// The "column" name in the cursor of [`Find::page_tiers`] (the same as the old `fetch_page_tiers`).
const TIER: &str = "tier";

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
        let error = global_check::<R>().map(global_refused);
        Find {
            handle: self,
            statement,
            joined: Vec::new(),
            error,
            within: None,
            _marker: PhantomData,
        }
    }

    /// A subquery selecting `column` of `S` (for `Col::in_sub`), filtered by tenant like `find`. It remembers
    /// the tenant of this handle: a query of another handle refuses it.
    fn sub<S: Record, T: DbValue>(&self, column: Col<S, T>) -> Sub<S, T>
    where
        S::Scope: AcceptedBy<Self::Access>,
    {
        let mut statement = Query::select();
        statement.from(table::<S>()).expr(column.sq());
        if let Some(tenant) = tenant_of_handle::<Self, S>(self) {
            statement.and_where(tenant_filter::<S>(tenant));
        }
        // Subquery trên bảng của khách mang khách của handle; trên bảng control-plane thì không gắn khách.
        let subs = match global_check::<S>() {
            Some(table) => vec![SubOrigin::Refused(table)],
            None if S::Scope::TENANT => vec![SubOrigin::Handle(self.access().tenant())],
            None => Vec::new(),
        };
        Sub { statement, subs, _marker: PhantomData }
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
    pub(crate) subs: Subs,
    _marker: PhantomData<fn() -> (S, T)>,
}

impl<S: Record, T> Sub<S, T> {
    /// Adds a condition.
    #[must_use]
    pub fn filter(mut self, condition: Cond<S>) -> Self {
        // Subquery lồng bên trong phải cùng khách với query ngoài cùng: gom lên để query ngoài cùng kiểm.
        self.subs.extend(condition.subs);
        self.statement.cond_where(condition.inner);
        self
    }
}

/// A `SELECT` of rows of `R`, built step by step.
pub struct Find<'h, H, R> {
    handle: &'h H,
    statement: SelectStatement,
    joined: Vec<&'static str>,
    error: Option<Error>,
    within: Option<std::time::Duration>,
    _marker: PhantomData<fn() -> R>,
}

impl<'h, H: Handle, R: Record> Find<'h, H, R> {
    // Điều kiện mang subquery của handle khác (khách khác, hay "mọi khách") thì từ chối khi chạy.
    fn check(&mut self, subs: &[SubOrigin]) {
        if self.error.is_none() {
            self.error = foreign_sub(subs, self.handle.access().tenant());
        }
    }

    /// Adds a condition (AND).
    #[must_use]
    pub fn filter(mut self, condition: Cond<R>) -> Self {
        self.check(&condition.subs);
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
        if self.error.is_none() {
            self.error = global_check::<S>().map(global_refused);
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
        self.check(&condition.subs);
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

    /// Stops the query when it runs longer than `limit`, or when the caller stops waiting: the database ends it
    /// and [`all`](Self::all) returns [`QUERY_TIMEOUT`](super::QUERY_TIMEOUT). Every way of reading honours it
    /// (`all`, `one`, `exists`, `count`, `select`, and `page`, whose count and rows each get `limit`). For reads on a read handle only
    /// (on a write transaction the read fails). Use it for queries whose cost depends on what a caller sends
    /// (public search).
    #[must_use]
    pub fn within(mut self, limit: std::time::Duration) -> Self {
        self.within = Some(limit);
        self
    }

    /// Every matching row.
    pub async fn all(self) -> Result<Vec<R>, Error> {
        if let Some(error) = self.error {
            return Err(error);
        }
        read_rows(self.handle, &self.statement, self.within, R::read).await
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
        for (expr, subs) in selection.exprs() {
            self.check(&subs);
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
        self.page_keyed(id, HasId::id, request).await
    }

    /// Like [`page`](Self::page), keyed by another UUIDv7 column than `id` (e.g. a table whose primary key is
    /// composite and has a `uid` column for the cursor): `key_of` reads that column from a row. The column may be
    /// declared nullable (`Option<Uuid>`, a column added to an existing table and filled by a migration) but must
    /// be filled on every row: the two databases place `NULL` differently.
    pub async fn page_keyed<K: UuidKey>(
        self,
        key: Col<R, K>,
        key_of: impl Fn(&R) -> Uuid,
        request: &PageRequest,
    ) -> Result<Page<R>, Error> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let total = self.total(request).await?;
        let mut statement = self.statement;
        match (request.id_cursor()?, request.order()) {
            (None, _) => {}
            (Some(cursor), Order::Asc) => {
                statement.cond_where(key.gt(K::from_uuid(cursor)).inner);
            }
            (Some(cursor), Order::Desc) => {
                statement.cond_where(key.lt(K::from_uuid(cursor)).inner);
            }
        }
        let direction = match request.order() {
            Order::Asc => sea_query::Order::Asc,
            Order::Desc => sea_query::Order::Desc,
        };
        statement.order_by_expr(key.sq(), direction);
        // Lấy dư MỘT dòng để biết còn trang sau mà không cần câu đếm.
        statement.limit(u64::from(request.limit()) + 1);
        let mut rows = read_rows(self.handle, &statement, self.within, R::read).await?;
        let limit = request.limit() as usize;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let next_cursor =
            if has_more { rows.last().map(|row| key_of(row).to_string()) } else { None };
        Ok(Page {
            data: rows,
            pagination: Pagination::new(request.limit(), next_cursor).with_total(total),
        })
    }

    /// One page ordered by the NOT NULL column `sort` first, then by the ID column `id` — for lists with a
    /// natural order (position, name, code; `NaturalOrderPageQuery`) or a chosen sort column
    /// (`SortChoicePageQuery`). `sort_of` reads the column's value from a row, for the next page's cursor. Same
    /// keyset rules as [`page`](Self::page); a cursor of another list or sort column is a 422 on `cursor`. Needs
    /// an index on the filter columns + `(sort, id)`. Text is ordered by the database's collation: SQLite and
    /// Postgres may order names differently, but within one database paging neither repeats nor skips a row.
    ///
    /// A nullable column does not compile (`Option<T>` is not a [`SortKey`]):
    ///
    /// ```compile_fail,E0277
    /// # use autoxide_kernel::db::{Queries as _, Record, TenantKey, TenantRead};
    /// # use autoxide_kernel::pagination::PageRequest;
    /// #[derive(Record)]
    /// #[record(table = "kernel_doc_note", scope = tenant)]
    /// struct Note { #[record(primary_key)] id: uuid::Uuid, tenant_id: TenantKey, title: Option<String> }
    /// async fn list(read: &TenantRead<'_>, page: &PageRequest) {
    ///     let _ = read.find::<Note>().page_by(Note::TITLE, |n| n.title.clone(), Note::ID, page).await;
    /// }
    /// ```
    ///
    /// Control (one token differs, it compiles):
    ///
    /// ```no_run
    /// # use autoxide_kernel::db::{Queries as _, Record, TenantKey, TenantRead};
    /// # use autoxide_kernel::pagination::PageRequest;
    /// #[derive(Record)]
    /// #[record(table = "kernel_doc_note", scope = tenant)]
    /// struct Note { #[record(primary_key)] id: uuid::Uuid, tenant_id: TenantKey, title: String }
    /// async fn list(read: &TenantRead<'_>, page: &PageRequest) {
    ///     let _ = read.find::<Note>().page_by(Note::TITLE, |n| n.title.clone(), Note::ID, page).await;
    /// }
    /// ```
    pub async fn page_by<T: SortKey>(
        self,
        sort: Col<R, T>,
        sort_of: impl Fn(&R) -> T,
        id: Col<R, Uuid>,
        request: &PageRequest,
    ) -> Result<Page<R>, Error>
    where
        R: HasId,
    {
        if let Some(error) = self.error {
            return Err(error);
        }
        let total = self.total(request).await?;
        let mut statement = self.statement;
        let fits = |value: &SortValue| T::from_sort_value(value).is_some();
        if let Some((value, after_id)) = request.key_after(sort.name(), fits)? {
            let value = T::from_sort_value(&value).ok_or_else(|| {
                Error::internal("page cursor value checked a moment ago no longer fits")
            })?;
            // `col >= v AND (col > v OR id > id0)` như `fetch_page_by`: cận trước cho database nhảy thẳng tới
            // điểm trong index (cột lọc, col, id) thay vì quét từ đầu phần lọc.
            let (bound, after, tie) = match request.order() {
                Order::Asc => (sort.gte(value.clone()), sort.gt(value), id.gt(after_id)),
                Order::Desc => (sort.lte(value.clone()), sort.lt(value), id.lt(after_id)),
            };
            statement.cond_where(bound.inner);
            statement.cond_where(sea_query::Condition::any().add(after.inner).add(tie.inner));
        }
        let direction = match request.order() {
            Order::Asc => sea_query::Order::Asc,
            Order::Desc => sea_query::Order::Desc,
        };
        statement.order_by_expr(sort.sq(), direction.clone());
        statement.order_by_expr(id.sq(), direction);
        statement.limit(u64::from(request.limit()) + 1);
        let mut rows = read_rows(self.handle, &statement, self.within, R::read).await?;
        let limit = request.limit() as usize;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let next_cursor = match (has_more, rows.last()) {
            (true, Some(row)) => Some(crate::pagination::key_cursor(
                sort.name(),
                sort_of(row).into_sort_value(),
                row.id(),
            )),
            _ => None,
        };
        Ok(Page {
            data: rows,
            pagination: Pagination::new(request.limit(), next_cursor).with_total(total),
        })
    }

    /// Like [`page`](Self::page), but splits rows into ranked tiers: every row matching `tiers[0]` comes first,
    /// then the rows matching `tiers[1]` (and no earlier tier)…; within a tier rows are ordered by ID. A row
    /// matching no tier is left out. For search where "exact accent match ranks before accent-insensitive match"
    /// (D-202): the rank depends on what the user typed, so it cannot be a column. Same cursor as the old
    /// `fetch_page_tiers` (the tier and the last ID); a page runs at most `tiers.len()` queries.
    pub async fn page_tiers(
        mut self,
        tiers: &[Cond<R>],
        id: Col<R, Uuid>,
        request: &PageRequest,
    ) -> Result<Page<R>, Error>
    where
        R: HasId,
    {
        for tier in tiers {
            self.check(&tier.subs);
        }
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        let in_range = |value: &SortValue| match value {
            SortValue::Integer(tier) => usize::try_from(*tier).is_ok_and(|tier| tier < tiers.len()),
            _ => false,
        };
        let (start, after) = match request.key_after(TIER, in_range)? {
            None => (0, None),
            Some((SortValue::Integer(tier), after)) => {
                (usize::try_from(tier).unwrap_or_default(), Some(after))
            }
            Some(_) => {
                return Err(Error::internal("tier cursor checked a moment ago no longer fits"));
            }
        };
        let total = if request.wants_total() {
            let mut counted = self.statement.clone();
            counted.cond_where(
                tiers
                    .iter()
                    .fold(sea_query::Condition::any(), |any, tier| any.add(tier.inner.clone())),
            );
            counted.clear_selects();
            counted.expr(sea_query::Expr::cust("COUNT(*)"));
            let rows = read_rows(self.handle, &counted, self.within, <(i64,)>::from_row).await?;
            Some(rows.first().map_or(0, |(n,)| u64::try_from(*n).unwrap_or_default()))
        } else {
            None
        };
        let direction = match request.order() {
            Order::Asc => sea_query::Order::Asc,
            Order::Desc => sea_query::Order::Desc,
        };
        let wanted = request.limit() as usize + 1;
        let mut rows: Vec<(usize, R)> = Vec::new();
        for (index, tier) in tiers.iter().enumerate().skip(start) {
            let mut condition = sea_query::Condition::all().add(tier.inner.clone());
            for earlier in &tiers[..index] {
                condition = condition.add(earlier.inner.clone().not());
            }
            let mut statement = self.statement.clone();
            statement.cond_where(condition);
            if let (true, Some(after)) = (index == start, after) {
                let past = match request.order() {
                    Order::Asc => id.gt(after),
                    Order::Desc => id.lt(after),
                };
                statement.cond_where(past.inner);
            }
            statement.order_by_expr(id.sq(), direction.clone());
            statement.limit((wanted - rows.len()) as u64);
            let found = read_rows(self.handle, &statement, self.within, R::read).await?;
            rows.extend(found.into_iter().map(|row| (index, row)));
            if rows.len() >= wanted {
                break;
            }
        }
        let has_more = rows.len() >= wanted;
        rows.truncate(request.limit() as usize);
        let next_cursor = match (has_more, rows.last()) {
            (true, Some((tier, row))) => Some(crate::pagination::key_cursor(
                TIER,
                SortValue::Integer(i64::try_from(*tier).unwrap_or_default()),
                row.id(),
            )),
            _ => None,
        };
        Ok(Page {
            data: rows.into_iter().map(|(_, row)| row).collect(),
            pagination: Pagination::new(request.limit(), next_cursor).with_total(total),
        })
    }

    // `count=true`: tổng số dòng khớp bộ lọc, bằng một câu đếm riêng.
    async fn total(&self, request: &PageRequest) -> Result<Option<u64>, Error> {
        if !request.wants_total() {
            return Ok(None);
        }
        let mut counted = self.statement.clone();
        counted.clear_selects();
        counted.expr(sea_query::Expr::cust("COUNT(*)"));
        let rows = read_rows(self.handle, &counted, self.within, <(i64,)>::from_row).await?;
        Ok(Some(rows.first().map_or(0, |(n,)| u64::try_from(*n).unwrap_or_default())))
    }
}

impl<'h, A: Access, R: Record> Find<'h, Tx<A>, R> {
    /// Exclusive row lock until the write transaction ends. Postgres: `FOR UPDATE`. SQLite: nothing more is
    /// needed — a write transaction (`BEGIN IMMEDIATE`) already holds the whole database. Only on a write
    /// transaction, and the locked query reads rows only (no `count`, `exists`, `select`, `page`), so both
    /// databases behave the same. Lock rows in the same order everywhere to avoid deadlocks.
    ///
    /// ```compile_fail,E0599
    /// # use autoxide_kernel::db::{Queries as _, Record, TenantKey, TenantRead};
    /// #[derive(Record)]
    /// #[record(table = "kernel_doc_note", scope = tenant)]
    /// struct Note { id: uuid::Uuid, tenant_id: TenantKey }
    /// fn lock(read: &TenantRead<'_>) { let _ = read.find::<Note>().for_update(); }
    /// ```
    ///
    /// ```compile_fail,E0599
    /// # use autoxide_kernel::db::{Queries as _, Record, TenantKey, TenantTx};
    /// #[derive(Record)]
    /// #[record(table = "kernel_doc_note", scope = tenant)]
    /// struct Note { id: uuid::Uuid, tenant_id: TenantKey }
    /// fn lock(tx: &TenantTx) { let _ = tx.find::<Note>().for_update().count(); }
    /// ```
    ///
    /// Control (one token differs from each, it compiles):
    ///
    /// ```no_run
    /// # use autoxide_kernel::db::{Queries as _, Record, TenantKey, TenantTx};
    /// #[derive(Record)]
    /// #[record(table = "kernel_doc_note", scope = tenant)]
    /// struct Note { id: uuid::Uuid, tenant_id: TenantKey }
    /// fn lock(tx: &TenantTx) { let _ = tx.find::<Note>().for_update().one(); }
    /// ```
    pub fn for_update(mut self) -> Locked<'h, A, R> {
        self.statement.lock(LockType::Update);
        Locked { find: self }
    }

    /// Shared row lock until the write transaction ends (Postgres `FOR SHARE`; SQLite as
    /// [`for_update`](Self::for_update)): several transactions may hold it, a writer of the row waits.
    pub fn for_share(mut self) -> Locked<'h, A, R> {
        self.statement.lock(LockType::Share);
        Locked { find: self }
    }
}

/// A `SELECT … FOR UPDATE / FOR SHARE` on a write transaction: reads rows only.
pub struct Locked<'h, A, R> {
    find: Find<'h, Tx<A>, R>,
}

impl<A: Access, R: Record> Locked<'_, A, R> {
    /// Adds a condition (AND).
    #[must_use]
    pub fn filter(mut self, condition: Cond<R>) -> Self {
        self.find = self.find.filter(condition);
        self
    }

    /// `ORDER BY column ASC` (lock in a fixed order).
    #[must_use]
    pub fn order_asc<T: DbValue>(mut self, column: Col<R, T>) -> Self {
        self.find = self.find.order_asc(column);
        self
    }

    /// At most `limit` rows.
    #[must_use]
    pub fn limit(mut self, limit: u64) -> Self {
        self.find = self.find.limit(limit);
        self
    }

    /// Every matching row, locked.
    pub async fn all(self) -> Result<Vec<R>, Error> {
        self.find.all().await
    }

    /// The first matching row, locked.
    pub async fn one(self) -> Result<Option<R>, Error> {
        self.find.one().await
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
        if let Some(error) = self.find.error {
            return Err(error);
        }
        read_rows(self.find.handle, &self.find.statement, self.find.within, Out::from_row).await
    }
}

// Đọc mọi dòng của câu, có trần thời gian khi `within` có giá trị (`super::bounded`).
async fn read_rows<H: Handle, T>(
    handle: &H,
    statement: &SelectStatement,
    within: Option<std::time::Duration>,
    read: impl Fn(&Row<'_>) -> Result<T, Error>,
) -> Result<Vec<T>, Error> {
    let connection = handle.connection();
    let statement = connection.build(statement);
    let rows = match within {
        Some(limit) => connection.all_within(statement, limit).await?,
        None => connection.all(statement).await?,
    };
    rows.iter().map(|row| read(&Row(row))).collect()
}
