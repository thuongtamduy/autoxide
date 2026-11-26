//! SPIKE — never merged. Prototype of the kernel `db` layer (D-20261126-kernel-data-layer) to check, on
//! SQLite and Postgres, that the hardest query shapes used by modules today (ledger, inventory, reporting,
//! catalog search) can be written without SeaORM / sea-query types in the module's code.
//!
//! Everything public here is what a module would see. sea-query / SeaORM stay inside.
//! `Record` impls are hand-written with `record!` (a stand-in for `#[derive(Record)]`).

#![allow(clippy::missing_errors_doc, clippy::must_use_candidate, clippy::return_self_not_must_use)]

use std::marker::PhantomData;

use autoxide_kernel::database::{Database, statement};
use autoxide_kernel::error::Error;
use autoxide_kernel::search::escape_like;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::{Date, Timestamp};
use sea_orm::sea_query::{
    self as sq, Alias, ExprTrait as _, Func, IntoCondition as _, LikeExpr, LockType, OnConflict,
    Query, SelectStatement,
};
use sea_orm::{ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, QueryResult};
use uuid::Uuid;

// ───────────────────────────── scope ─────────────────────────────

mod sealed {
    pub trait Sealed {}
}

/// Whether a record belongs to one tenant (`Tenant`) or to the control plane (`Global`).
pub trait Scope: sealed::Sealed + Send + Sync + 'static {
    const TENANT: bool;
}
pub enum Tenant {}
pub enum Global {}
impl sealed::Sealed for Tenant {}
impl sealed::Sealed for Global {}
impl Scope for Tenant {
    const TENANT: bool = true;
}
impl Scope for Global {
    const TENANT: bool = false;
}

// ───────────────────────────── values ─────────────────────────────

/// A value a column can hold.
pub trait DbValue: Sized + Send + Sync + 'static {
    fn into_value(self) -> sq::Value;
    fn read(row: &QueryResult, index: usize) -> Result<Self, Error>;
}

/// A value a module may write with `update().set(…)`. The tenant column has its own type that is NOT
/// `Settable`, so moving a row to another tenant does not compile.
pub trait Settable: DbValue {}

macro_rules! db_values {
    ($($t:ty),*) => {$(
        impl DbValue for $t {
            fn into_value(self) -> sq::Value { self.into() }
            fn read(row: &QueryResult, index: usize) -> Result<Self, Error> {
                Ok(row.try_get_by_index::<$t>(index)?)
            }
        }
        impl Settable for $t {}
        impl DbValue for Option<$t> {
            fn into_value(self) -> sq::Value { self.into() }
            fn read(row: &QueryResult, index: usize) -> Result<Self, Error> {
                Ok(row.try_get_by_index::<Option<$t>>(index)?)
            }
        }
        impl Settable for Option<$t> {}
    )*};
}
db_values!(Uuid, i64, i32, bool, String, Timestamp, Date);

/// The `tenant_id` column. Readable, comparable, never settable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TenantKey(pub Uuid);

impl DbValue for TenantKey {
    fn into_value(self) -> sq::Value {
        self.0.into()
    }
    fn read(row: &QueryResult, index: usize) -> Result<Self, Error> {
        Ok(TenantKey(row.try_get_by_index::<Uuid>(index)?))
    }
}

/// A row read as a tuple (`select((a, b, sum(c)))` → `(A, B, i64)`).
pub trait FromRow: Sized {
    fn from_row(row: &QueryResult) -> Result<Self, Error>;
}

macro_rules! tuples {
    ($( ($($name:ident $index:tt),+) )*) => {$(
        impl<$($name: DbValue),+> FromRow for ($($name,)+) {
            fn from_row(row: &QueryResult) -> Result<Self, Error> {
                Ok(($($name::read(row, $index)?,)+))
            }
        }
        impl<Rec: Record, $($name: Selectable<Rec>),+> Selection<Rec> for ($($name,)+) {
            type Out = ($($name::Out,)+);
            fn exprs(self) -> Vec<sq::Expr> { vec![$(self.$index.expr()),+] }
        }
    )*};
}
tuples! {
    (A 0)
    (A 0, B 1)
    (A 0, B 1, C 2)
    (A 0, B 1, C 2, D 3)
    (A 0, B 1, C 2, D 3, E 4)
    (A 0, B 1, C 2, D 3, E 4, F 5)
}

// ───────────────────────────── record and columns ─────────────────────────────

/// A table row. `#[derive(Record)]` writes this; here `record!` stands in for it.
pub trait Record: Sized + Send + Sync + 'static {
    const TABLE: &'static str;
    type Scope: Scope;
    const COLUMNS: &'static [&'static str];
    /// The value of `tenant_id` (tenant records), checked on insert.
    fn tenant_of(&self) -> Option<Uuid>;
    fn read(row: &QueryResult) -> Result<Self, Error>;
    fn values(self) -> Vec<sq::Value>;
}

fn module_of(table: &str) -> &str {
    table.split('_').next().unwrap_or(table)
}

fn table<R: Record>() -> Alias {
    Alias::new(R::TABLE)
}

fn qualified<R: Record>(column: &'static str) -> sq::Expr {
    sq::Expr::col((table::<R>(), Alias::new(column)))
}

fn tenant_filter<R: Record>(tenant: Uuid) -> sq::Expr {
    qualified::<R>("tenant_id").eq(tenant)
}

/// A typed column: `Note::TITLE: Col<Note, String>`; `Note::TITLE.eq(42)` does not compile.
pub struct Col<R, T> {
    name: &'static str,
    _marker: PhantomData<fn() -> (R, T)>,
}

impl<R, T> Clone for Col<R, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<R, T> Copy for Col<R, T> {}

/// A column with its value type erased: conflict targets, `do_update` lists.
#[derive(Clone, Copy)]
pub struct ColName<R> {
    name: &'static str,
    _marker: PhantomData<fn() -> R>,
}

impl<R: Record, T: DbValue> Col<R, T> {
    pub const fn new(name: &'static str) -> Self {
        Col { name, _marker: PhantomData }
    }

    pub const fn any(self) -> ColName<R> {
        ColName { name: self.name, _marker: PhantomData }
    }

    fn sq(self) -> sq::Expr {
        qualified::<R>(self.name)
    }

    pub fn expr(self) -> Expr<R, T> {
        Expr::wrap(self.sq())
    }

    pub fn eq(self, value: T) -> Cond<R> {
        Cond::wrap(self.sq().eq(value.into_value()))
    }

    pub fn ne(self, value: T) -> Cond<R> {
        Cond::wrap(self.sq().ne(value.into_value()))
    }

    pub fn gt(self, value: T) -> Cond<R> {
        Cond::wrap(self.sq().gt(value.into_value()))
    }

    pub fn gte(self, value: T) -> Cond<R> {
        Cond::wrap(self.sq().gte(value.into_value()))
    }

    pub fn lt(self, value: T) -> Cond<R> {
        Cond::wrap(self.sq().lt(value.into_value()))
    }

    pub fn lte(self, value: T) -> Cond<R> {
        Cond::wrap(self.sq().lte(value.into_value()))
    }

    pub fn is_in(self, values: impl IntoIterator<Item = T>) -> Cond<R> {
        let values: Vec<sq::Value> = values.into_iter().map(DbValue::into_value).collect();
        Cond::wrap(self.sq().is_in(values))
    }

    /// `self IN (subquery)`; the subquery comes from the same handle, so it is filtered by tenant already.
    pub fn in_sub<S: Record>(self, sub: Sub<S, T>) -> Cond<R> {
        Cond::wrap(self.sq().in_subquery(sub.statement))
    }

    /// Join condition `self = other` (both tables of the same module).
    pub fn eq_col<S: Record>(self, other: Col<S, T>) -> On<R, S> {
        On { expr: self.sq().equals((table::<S>(), Alias::new(other.name))), _marker: PhantomData }
    }
}

impl<R: Record> Col<R, String> {
    /// `LIKE '%text%'` with `%`, `_`, `\` escaped — the same on both databases.
    pub fn contains(self, text: &str) -> Cond<R> {
        let pattern = format!("%{}%", escape_like(text));
        Cond::wrap(self.sq().like(LikeExpr::new(pattern).escape('\\')))
    }
}

/// A condition on rows of `R`.
pub struct Cond<R> {
    inner: sq::Condition,
    _marker: PhantomData<fn() -> R>,
}

impl<R> Cond<R> {
    fn wrap(expr: sq::Expr) -> Self {
        Cond { inner: expr.into_condition(), _marker: PhantomData }
    }

    /// Every condition added must hold (empty: true).
    pub fn all() -> Self {
        Cond { inner: sq::Condition::all(), _marker: PhantomData }
    }

    /// At least one condition added must hold (empty: false).
    pub fn any() -> Self {
        Cond { inner: sq::Condition::any(), _marker: PhantomData }
    }

    pub fn add(self, other: Cond<R>) -> Self {
        Cond { inner: self.inner.add(other.inner), _marker: PhantomData }
    }

    pub fn not(self) -> Self {
        Cond { inner: self.inner.not(), _marker: PhantomData }
    }
}

/// `JOIN … ON` condition between two records.
pub struct On<R, S> {
    expr: sq::Expr,
    _marker: PhantomData<fn() -> (R, S)>,
}

/// A typed expression over rows of `R`.
pub struct Expr<R, T> {
    inner: sq::Expr,
    _marker: PhantomData<fn() -> (R, T)>,
}

impl<R, T> Expr<R, T> {
    fn wrap(inner: sq::Expr) -> Self {
        Expr { inner, _marker: PhantomData }
    }
}

impl<R: Record, T: DbValue> Expr<R, T> {
    pub fn value(value: T) -> Self {
        Expr::wrap(sq::Expr::val(value.into_value()))
    }

    /// `CASE WHEN condition THEN then ELSE otherwise END`.
    pub fn case_when(condition: Cond<R>, then: Expr<R, T>, otherwise: Expr<R, T>) -> Self {
        Expr::wrap(sq::Expr::case(condition.inner, then.inner).finally(otherwise.inner).into())
    }
}

impl<R: Record> std::ops::Sub for Expr<R, i64> {
    type Output = Expr<R, i64>;
    fn sub(self, other: Self) -> Self {
        Expr::wrap(self.inner.sub(other.inner))
    }
}

impl<R: Record> std::ops::Add for Expr<R, i64> {
    type Output = Expr<R, i64>;
    fn add(self, other: Self) -> Self {
        Expr::wrap(self.inner.add(other.inner))
    }
}

impl<R: Record> std::ops::Mul for Expr<R, i64> {
    type Output = Expr<R, i64>;
    fn mul(self, other: Self) -> Self {
        Expr::wrap(self.inner.mul(other.inner))
    }
}

impl<R: Record> std::ops::Neg for Expr<R, i64> {
    type Output = Expr<R, i64>;
    fn neg(self) -> Self {
        Expr::wrap(sq::Expr::val(0_i64).sub(self.inner))
    }
}

impl<R: Record, T: DbValue> From<Col<R, T>> for Expr<R, T> {
    fn from(column: Col<R, T>) -> Self {
        column.expr()
    }
}

/// An aggregate. Every integer aggregate is `CAST(COALESCE(…, 0) AS BIGINT)`: Postgres returns `numeric` for
/// `SUM(bigint)` and SQLite may return NULL; the module always reads `i64`.
pub struct Agg<R, T> {
    inner: sq::Expr,
    _marker: PhantomData<fn() -> (R, T)>,
}

fn bigint(expr: sq::Expr) -> sq::Expr {
    Func::cast_as(Func::coalesce([expr, sq::Expr::val(0_i64)]), Alias::new("BIGINT")).into()
}

pub fn sum<R: Record>(expr: impl Into<Expr<R, i64>>) -> Agg<R, i64> {
    Agg { inner: bigint(Func::sum(expr.into().inner).into()), _marker: PhantomData }
}

pub fn count<R: Record>() -> Agg<R, i64> {
    Agg { inner: bigint(Func::count(sq::Expr::cust("*")).into()), _marker: PhantomData }
}

pub fn max<R: Record, T: DbValue>(column: Col<R, T>) -> Agg<R, Option<T>>
where
    Option<T>: DbValue,
{
    Agg { inner: Func::max(column.sq()).into(), _marker: PhantomData }
}

/// One item of a `select((…))` list.
pub trait Selectable<R>: Sized {
    type Out: DbValue;
    fn expr(self) -> sq::Expr;
}

impl<R: Record, T: DbValue> Selectable<R> for Col<R, T> {
    type Out = T;
    fn expr(self) -> sq::Expr {
        self.sq()
    }
}

impl<R: Record, T: DbValue> Selectable<R> for Agg<R, T> {
    type Out = T;
    fn expr(self) -> sq::Expr {
        self.inner
    }
}

/// A whole `select((…))` list; its `Out` is the tuple each row is read as.
pub trait Selection<R> {
    type Out;
    fn exprs(self) -> Vec<sq::Expr>;
}

// ───────────────────────────── handles ─────────────────────────────

/// What every handle gives the query builders. Sealed: modules cannot make a handle without a tenant.
pub trait Handle: sealed::Sealed + Sync {
    #[doc(hidden)]
    fn connection(&self) -> Connection<'_>;
    #[doc(hidden)]
    fn tenant(&self) -> Option<Uuid>;
    /// Whether this handle accepts records of scope `Tenant`.
    #[doc(hidden)]
    fn accepts<R: Record>(&self) -> Result<(), Error> {
        if R::Scope::TENANT && self.tenant().is_none() {
            return Err(Error::internal(format!(
                "`{}` is a tenant table: query it through `tenant.db()`",
                R::TABLE
            )));
        }
        Ok(())
    }
}

#[doc(hidden)]
pub enum Connection<'a> {
    Read(&'a DatabaseConnection),
    Write(&'a DatabaseTransaction),
}

impl Connection<'_> {
    fn backend(&self) -> DbBackend {
        match self {
            Connection::Read(c) => c.get_database_backend(),
            Connection::Write(c) => c.get_database_backend(),
        }
    }

    async fn all(&self, statement: sea_orm::Statement) -> Result<Vec<QueryResult>, Error> {
        Ok(match self {
            Connection::Read(c) => c.query_all_raw(statement).await?,
            Connection::Write(c) => c.query_all_raw(statement).await?,
        })
    }

    async fn execute(&self, statement: sea_orm::Statement) -> Result<u64, Error> {
        Ok(match self {
            Connection::Read(c) => c.execute_raw(statement).await?,
            Connection::Write(c) => c.execute_raw(statement).await?,
        }
        .rows_affected())
    }

    fn build(&self, statement: &impl sea_orm::StatementBuilder) -> sea_orm::Statement {
        statement.build(&self.backend())
    }
}

/// `tenant.db()`: every record of scope `Tenant` is filtered by this tenant.
pub struct TenantDb<'a> {
    database: &'a Database,
    tenant: Uuid,
}

/// Stand-in for `TenantContext::db()`.
pub fn tenant_db(tenant: &TenantContext) -> TenantDb<'_> {
    TenantDb { database: tenant.database(), tenant: tenant.id().as_uuid() }
}

impl<'a> TenantDb<'a> {
    pub fn read(&self) -> TenantRead<'a> {
        TenantRead { connection: self.database.read(), tenant: self.tenant }
    }

    pub async fn begin_write(&self) -> Result<TenantTx, Error> {
        Ok(TenantTx { transaction: self.database.begin_write().await?, tenant: self.tenant })
    }
}

/// Reads of one tenant, on the read connection.
pub struct TenantRead<'a> {
    connection: &'a DatabaseConnection,
    tenant: Uuid,
}

impl sealed::Sealed for TenantRead<'_> {}
impl Handle for TenantRead<'_> {
    fn connection(&self) -> Connection<'_> {
        Connection::Read(self.connection)
    }
    fn tenant(&self) -> Option<Uuid> {
        Some(self.tenant)
    }
}

/// A write transaction of one tenant.
pub struct TenantTx {
    transaction: DatabaseTransaction,
    tenant: Uuid,
}

impl sealed::Sealed for TenantTx {}
impl Handle for TenantTx {
    fn connection(&self) -> Connection<'_> {
        Connection::Write(&self.transaction)
    }
    fn tenant(&self) -> Option<Uuid> {
        Some(self.tenant)
    }
}

impl TenantTx {
    pub async fn commit(self) -> Result<(), Error> {
        Ok(self.transaction.commit().await?)
    }

    /// Lock by name until the transaction ends (Postgres advisory lock; SQLite: the write transaction already
    /// holds the database). The name is prefixed with the tenant, so two tenants never wait on each other.
    pub async fn lock_key(&self, key: &str) -> Result<(), Error> {
        let key = format!("{}:{key}", self.tenant);
        Ok(autoxide_kernel::rows::lock_key(&self.transaction, &key).await?)
    }

    pub fn insert<R: Record>(&self, row: R) -> Insert<'_, R> {
        let error = match (R::Scope::TENANT, row.tenant_of()) {
            (true, Some(owner)) if owner != self.tenant => Some(Error::internal(format!(
                "insert into `{}`: the row belongs to another tenant",
                R::TABLE
            ))),
            _ => None,
        };
        let mut statement = Query::insert();
        statement.into_table(table::<R>()).columns(R::COLUMNS.iter().map(|c| Alias::new(*c)));
        // `values` chỉ lỗi khi số giá trị khác số cột — `Record` sinh cả hai từ cùng một danh sách.
        let values: Vec<sq::Expr> = row.values().into_iter().map(sq::Expr::val).collect();
        let error = error
            .or_else(|| statement.values(values).err().map(|e| Error::internal(e.to_string())));
        Insert { tx: self, statement, error, guarded_update: false, _marker: PhantomData }
    }

    pub fn update<R: Record>(&self) -> Update<'_, R> {
        let mut statement = Query::update();
        statement.table(table::<R>());
        if R::Scope::TENANT {
            statement.and_where(tenant_filter::<R>(self.tenant));
        }
        Update { tx: self, statement, error: None, has_set: false, _marker: PhantomData }
    }

    pub fn delete<R: Record>(&self) -> Delete<'_, R> {
        let mut statement = Query::delete();
        statement.from_table(table::<R>());
        if R::Scope::TENANT {
            statement.and_where(tenant_filter::<R>(self.tenant));
        }
        Delete { tx: self, statement, _marker: PhantomData }
    }
}

/// Control-plane reads: only `Global` records compile here.
pub struct GlobalRead<'a> {
    connection: &'a DatabaseConnection,
}

pub fn global_db(database: &Database) -> GlobalRead<'_> {
    GlobalRead { connection: database.read() }
}

impl sealed::Sealed for GlobalRead<'_> {}
impl Handle for GlobalRead<'_> {
    fn connection(&self) -> Connection<'_> {
        Connection::Read(self.connection)
    }
    fn tenant(&self) -> Option<Uuid> {
        None
    }
}

// ───────────────────────────── queries ─────────────────────────────

/// Query entry points, on every handle.
pub trait Queries: Handle + Sized {
    /// `SELECT` rows of `R`. On a tenant handle a tenant record is filtered by tenant; on `GlobalRead` a
    /// tenant record does not compile (see `find_global`).
    fn find<R: Record>(&self) -> Find<'_, Self, R> {
        let mut statement = Query::select();
        statement.from(table::<R>());
        for column in R::COLUMNS {
            statement.expr(qualified::<R>(column));
        }
        let error = self.accepts::<R>().err();
        if let (true, Some(tenant)) = (R::Scope::TENANT, self.tenant()) {
            statement.and_where(tenant_filter::<R>(tenant));
        }
        Find { handle: self, statement, error, joined: Vec::new(), _marker: PhantomData }
    }

    /// Subquery selecting `column` of `S`, filtered by tenant like `find`.
    fn sub<S: Record, T: DbValue>(&self, column: Col<S, T>) -> Sub<S, T> {
        let mut statement = Query::select();
        statement.from(table::<S>()).expr(column.sq());
        if let (true, Some(tenant)) = (S::Scope::TENANT, self.tenant()) {
            statement.and_where(tenant_filter::<S>(tenant));
        }
        Sub { statement, _marker: PhantomData }
    }

    /// Hand-written SQL. On a tenant handle the text MUST use `:tenant`; the kernel binds it.
    fn sql(&self, text: &str) -> Sql<'_, Self> {
        Sql { handle: self, text: text.to_owned(), binds: Vec::new() }
    }
}

impl<H: Handle> Queries for H {}

/// A subquery built from a handle (already scoped).
pub struct Sub<S, T> {
    statement: SelectStatement,
    _marker: PhantomData<fn() -> (S, T)>,
}

impl<S: Record, T> Sub<S, T> {
    pub fn filter(mut self, condition: Cond<S>) -> Self {
        self.statement.cond_where(condition.inner);
        self
    }
}

pub struct Find<'h, H, R> {
    handle: &'h H,
    statement: SelectStatement,
    error: Option<Error>,
    joined: Vec<&'static str>,
    _marker: PhantomData<fn() -> R>,
}

impl<'h, H: Handle, R: Record> Find<'h, H, R> {
    pub fn filter(mut self, condition: Cond<R>) -> Self {
        self.statement.cond_where(condition.inner);
        self
    }

    /// `INNER JOIN S ON …`: only within one module; `S` is filtered by tenant too.
    pub fn join<S: Record>(mut self, on: On<R, S>) -> Self {
        if module_of(S::TABLE) != module_of(R::TABLE) {
            self.error.get_or_insert_with(|| {
                Error::internal(format!(
                    "join `{}` → `{}`: modules do not join each other's tables (architecture rule 1)",
                    R::TABLE,
                    S::TABLE
                ))
            });
        }
        let mut condition = sq::Condition::all().add(on.expr);
        if let (true, Some(tenant)) = (S::Scope::TENANT, self.handle.tenant()) {
            condition = condition.add(tenant_filter::<S>(tenant));
        }
        self.statement.inner_join(table::<S>(), condition);
        self.joined.push(S::TABLE);
        self
    }

    /// A condition on a joined record.
    pub fn filter_joined<S: Record>(mut self, condition: Cond<S>) -> Self {
        if !self.joined.contains(&S::TABLE) {
            self.error
                .get_or_insert_with(|| Error::internal(format!("`{}` is not joined", S::TABLE)));
        }
        self.statement.cond_where(condition.inner);
        self
    }

    pub fn order_asc<T: DbValue>(mut self, column: Col<R, T>) -> Self {
        self.statement.order_by_expr(column.sq(), sq::Order::Asc);
        self
    }

    pub fn order_desc<T: DbValue>(mut self, column: Col<R, T>) -> Self {
        self.statement.order_by_expr(column.sq(), sq::Order::Desc);
        self
    }

    pub fn limit(mut self, limit: u64) -> Self {
        self.statement.limit(limit);
        self
    }

    /// Exclusive row lock until the write transaction ends (Postgres `FOR UPDATE`; SQLite: the write
    /// transaction already holds the database).
    pub fn for_update(mut self) -> Self {
        self.statement.lock(LockType::Update);
        self
    }

    /// Shared row lock (Postgres `FOR SHARE`; SQLite as above).
    pub fn for_share(mut self) -> Self {
        self.statement.lock(LockType::Share);
        self
    }

    pub async fn all(self) -> Result<Vec<R>, Error> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let connection = self.handle.connection();
        let statement = connection.build(&self.statement);
        connection.all(statement).await?.iter().map(R::read).collect()
    }

    pub async fn one(mut self) -> Result<Option<R>, Error> {
        self.statement.limit(1);
        Ok(self.all().await?.into_iter().next())
    }

    /// Select expressions instead of whole rows; each row reads as `S::Out`.
    pub fn select<S: Selection<R>>(mut self, selection: S) -> Grouped<'h, H, R, S::Out> {
        self.statement.clear_selects();
        for expr in selection.exprs() {
            self.statement.expr(expr);
        }
        Grouped { find: self, _marker: PhantomData }
    }

    pub async fn count(self) -> Result<i64, Error> {
        let rows = self.select((count::<R>(),)).all().await?;
        Ok(rows.first().map_or(0, |(n,)| *n))
    }
}

pub struct Grouped<'h, H, R, Out> {
    find: Find<'h, H, R>,
    _marker: PhantomData<fn() -> Out>,
}

impl<H: Handle, R: Record, Out: FromRow> Grouped<'_, H, R, Out> {
    pub fn group_by<T: DbValue>(mut self, column: Col<R, T>) -> Self {
        self.find.statement.add_group_by([column.sq()]);
        self
    }

    pub fn order_asc<T: DbValue>(mut self, column: Col<R, T>) -> Self {
        self.find.statement.order_by_expr(column.sq(), sq::Order::Asc);
        self
    }

    pub async fn all(self) -> Result<Vec<Out>, Error> {
        if let Some(error) = self.find.error {
            return Err(error);
        }
        let connection = self.find.handle.connection();
        let statement = connection.build(&self.find.statement);
        connection.all(statement).await?.iter().map(Out::from_row).collect()
    }
}

/// Bounds for `adjust`: the value AFTER the change stays within `[floor, ceiling]`.
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub floor: i64,
    pub ceiling: i64,
}

impl Bounds {
    pub const NON_NEGATIVE: Bounds = Bounds { floor: 0, ceiling: i64::MAX };
}

pub struct Update<'t, R> {
    tx: &'t TenantTx,
    statement: sq::UpdateStatement,
    error: Option<Error>,
    has_set: bool,
    _marker: PhantomData<fn() -> R>,
}

impl<R: Record> Update<'_, R> {
    /// `SET column = value`. Only `Settable` values: the tenant column is not.
    pub fn set<T: Settable>(mut self, column: Col<R, T>, value: T) -> Self {
        self.statement.value(Alias::new(column.name), value.into_value());
        self.has_set = true;
        self
    }

    pub fn set_expr<T: Settable>(mut self, column: Col<R, T>, expr: Expr<R, T>) -> Self {
        self.statement.value(Alias::new(column.name), expr.inner);
        self.has_set = true;
        self
    }

    /// `SET c = c + delta` only where the result stays within `bounds` (no overflow on either database).
    pub fn adjust(mut self, column: Col<R, i64>, delta: i64, bounds: Bounds) -> Self {
        let low = i128::from(bounds.floor) - i128::from(delta);
        let high = i128::from(bounds.ceiling) - i128::from(delta);
        if low > high || low > i128::from(i64::MAX) || high < i128::from(i64::MIN) {
            // Không dòng nào thoả: điều kiện luôn sai.
            self.statement.and_where(sq::Expr::val(1).eq(0));
        }
        if let Ok(low) = i64::try_from(low) {
            self.statement.and_where(column.sq().gte(low));
        }
        if let Ok(high) = i64::try_from(high) {
            self.statement.and_where(column.sq().lte(high));
        }
        self.statement
            .value(Alias::new(column.name), sq::Expr::col(Alias::new(column.name)).add(delta));
        self.has_set = true;
        self
    }

    pub fn filter(mut self, condition: Cond<R>) -> Self {
        self.statement.cond_where(condition.inner);
        self
    }

    pub async fn exec(self) -> Result<u64, Error> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if !self.has_set {
            return Err(Error::internal("update without `set`"));
        }
        let connection = self.tx.connection();
        connection.execute(connection.build(&self.statement)).await
    }
}

pub struct Delete<'t, R> {
    tx: &'t TenantTx,
    statement: sq::DeleteStatement,
    _marker: PhantomData<fn() -> R>,
}

impl<R: Record> Delete<'_, R> {
    pub fn filter(mut self, condition: Cond<R>) -> Self {
        self.statement.cond_where(condition.inner);
        self
    }

    pub async fn exec(self) -> Result<u64, Error> {
        let connection = self.tx.connection();
        connection.execute(connection.build(&self.statement)).await
    }
}

pub struct Insert<'t, R> {
    tx: &'t TenantTx,
    statement: sq::InsertStatement,
    error: Option<Error>,
    guarded_update: bool,
    _marker: PhantomData<fn() -> R>,
}

impl<R: Record> Insert<'_, R> {
    /// `ON CONFLICT (target) DO NOTHING`.
    pub fn on_conflict_do_nothing<const N: usize>(mut self, target: [ColName<R>; N]) -> Self {
        let mut conflict = OnConflict::columns(target.iter().map(|c| Alias::new(c.name)));
        conflict.do_nothing();
        self.statement.on_conflict(conflict);
        self
    }

    /// `ON CONFLICT (target) DO UPDATE SET update = excluded.update`. On a tenant record the kernel adds
    /// `WHERE <table>.tenant_id = <this tenant>`: a unique key without `tenant_id` never lets one tenant
    /// overwrite another tenant's row; such a collision is an error instead.
    pub fn on_conflict_update<const N: usize, const M: usize>(
        mut self,
        target: [ColName<R>; N],
        update: [ColName<R>; M],
    ) -> Self {
        let mut conflict = OnConflict::columns(target.iter().map(|c| Alias::new(c.name)));
        conflict.update_columns(update.iter().map(|c| Alias::new(c.name)));
        if R::Scope::TENANT {
            conflict.action_and_where(tenant_filter::<R>(self.tx.tenant));
            self.guarded_update = true;
        }
        self.statement.on_conflict(conflict);
        self
    }

    pub async fn exec(self) -> Result<u64, Error> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let connection = self.tx.connection();
        let changed = connection.execute(connection.build(&self.statement)).await?;
        if changed == 0 && self.guarded_update {
            return Err(Error::conflict(
                "db.unique_conflict",
                "A record with the same unique key already exists.",
            ));
        }
        Ok(changed)
    }
}

/// Hand-written SQL with named parameters (`:name`). `:tenant` is bound by the kernel.
pub struct Sql<'h, H> {
    handle: &'h H,
    text: String,
    binds: Vec<(String, sq::Value)>,
}

impl<H: Handle> Sql<'_, H> {
    pub fn bind(mut self, name: &str, value: impl DbValue) -> Self {
        self.binds.push((name.to_owned(), value.into_value()));
        self
    }

    fn render(&self, backend: DbBackend) -> Result<sea_orm::Statement, Error> {
        let mut sql = String::with_capacity(self.text.len());
        let mut values = Vec::new();
        let mut saw_tenant = false;
        let mut chars = self.text.char_indices().peekable();
        let bytes = self.text.as_bytes();
        while let Some((index, character)) = chars.next() {
            let previous_colon = index > 0 && bytes[index - 1] == b':';
            let next_is_name =
                chars.peek().is_some_and(|(_, c)| c.is_ascii_alphabetic() || *c == '_');
            if character != ':'
                || previous_colon
                || !next_is_name
                || chars.peek().is_some_and(|(_, c)| *c == ':')
            {
                sql.push(character);
                continue;
            }
            let mut name = String::new();
            while let Some((_, c)) = chars.peek().copied() {
                if c.is_ascii_alphanumeric() || c == '_' {
                    name.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            let value = if name == "tenant" {
                saw_tenant = true;
                match self.handle.tenant() {
                    Some(tenant) => sq::Value::from(tenant),
                    None => return Err(Error::internal("`:tenant` on a handle without tenant")),
                }
            } else {
                match self.binds.iter().find(|(bound, _)| *bound == name) {
                    Some((_, value)) => value.clone(),
                    None => return Err(Error::internal(format!("`:{name}` is not bound"))),
                }
            };
            values.push(value);
            match backend {
                DbBackend::Postgres => sql.push_str(&format!("${}", values.len())),
                _ => sql.push('?'),
            }
        }
        if self.handle.tenant().is_some() && !saw_tenant {
            return Err(Error::internal(
                "hand-written SQL on a tenant handle must filter with `:tenant`",
            ));
        }
        Ok(statement(backend, sql, values))
    }

    pub async fn all<Out: FromRow>(self) -> Result<Vec<Out>, Error> {
        let connection = self.handle.connection();
        let statement = self.render(connection.backend())?;
        connection.all(statement).await?.iter().map(Out::from_row).collect()
    }
}

/// Stand-in for `#[derive(Record)]`.
#[macro_export]
macro_rules! record {
    (
        $(#[$meta:meta])*
        struct $name:ident in $table:literal, scope $scope:ident {
            $( $field:ident : $ty:ty => $konst:ident ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq)]
        pub struct $name { $( pub $field: $ty ),+ }

        impl $name {
            $( pub const $konst: $crate::Col<$name, $ty> = $crate::Col::new(stringify!($field)); )+
        }

        impl $crate::Record for $name {
            const TABLE: &'static str = $table;
            type Scope = $crate::$scope;
            const COLUMNS: &'static [&'static str] = &[$( stringify!($field) ),+];
            fn tenant_of(&self) -> Option<uuid::Uuid> {
                $crate::record!(@tenant self $( $field )+)
            }
            fn read(row: &sea_orm::QueryResult) -> Result<Self, autoxide_kernel::error::Error> {
                let mut index = 0usize;
                $( let $field = <$ty as $crate::DbValue>::read(row, index)?; index += 1; )+
                let _ = index;
                Ok($name { $( $field ),+ })
            }
            fn values(self) -> Vec<sea_orm::sea_query::Value> {
                vec![$( $crate::DbValue::into_value(self.$field) ),+]
            }
        }
    };
    (@tenant $self:ident tenant_id $($rest:ident)*) => { Some($self.tenant_id.0) };
    (@tenant $self:ident $first:ident $($rest:ident)*) => { $crate::record!(@tenant $self $($rest)*) };
    (@tenant $self:ident) => { None };
}
