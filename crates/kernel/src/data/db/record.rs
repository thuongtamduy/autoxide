//! Records, typed columns, conditions and expressions.

use std::marker::PhantomData;

use sea_orm::sea_query::{self, Alias, ExprTrait as _, Func, IntoCondition as _, LikeExpr};
use uuid::Uuid;

use super::value::{DbValue, Row};
use crate::error::Error;

mod sealed {
    pub trait Sealed {}
}

/// Whether a record belongs to one tenant ([`Tenant`]) or to the control plane ([`Global`]). Sealed: there
/// are exactly these two.
pub trait Scope: sealed::Sealed + Send + Sync + 'static {
    /// `true` for [`Tenant`].
    const TENANT: bool;
}

/// Scope of a tenant table: it has `tenant_id`, and every query through a tenant handle is filtered by it.
#[derive(Debug)]
pub enum Tenant {}

/// Scope of a control-plane table (no `tenant_id`).
#[derive(Debug)]
pub enum Global {}

impl sealed::Sealed for Tenant {}
impl sealed::Sealed for Global {}

impl Scope for Tenant {
    const TENANT: bool = true;
}

impl Scope for Global {
    const TENANT: bool = false;
}

/// A row of one module table. Written by `#[derive(Record)]`, never by hand: the derive checks at compile
/// time that the table belongs to the module (`<module>_…`), that the scope is declared, and that a tenant
/// record has `tenant_id: TenantKey`.
pub trait Record: crate::__private::RecordSeal + Sized + Send + Sync + 'static {
    /// Table name.
    const TABLE: &'static str;
    /// [`Tenant`] or [`Global`].
    type Scope: Scope;
    /// Column names, in field order.
    const COLUMNS: &'static [&'static str];
    /// Whether each column is nullable (an `Option` field), in field order.
    const NULLABLE: &'static [bool];
    /// The primary key column, when declared with `#[record(primary_key)]`.
    const PRIMARY_KEY: Option<&'static str>;

    /// The value of `tenant_id` of a tenant record; `None` for a global record.
    #[doc(hidden)]
    fn tenant_of(&self) -> Option<Uuid>;
    /// Reads one row whose columns come in [`COLUMNS`](Self::COLUMNS) order.
    #[doc(hidden)]
    fn read(row: &Row<'_>) -> Result<Self, Error>;
    /// The values of the fields, in [`COLUMNS`](Self::COLUMNS) order.
    #[doc(hidden)]
    fn values(self) -> Vec<super::Value>;
}

pub(crate) fn table<R: Record>() -> Alias {
    Alias::new(R::TABLE)
}

pub(crate) fn qualified<R: Record>(column: &str) -> sea_query::Expr {
    sea_query::Expr::col((table::<R>(), Alias::new(column)))
}

pub(crate) fn tenant_filter<R: Record>(tenant: Uuid) -> sea_query::Expr {
    qualified::<R>("tenant_id").eq(tenant)
}

/// A typed column of record `R` holding `T`: `Note::TITLE: Col<Note, String>`, so `Note::TITLE.eq(42)` does
/// not compile. The derive writes one constant per field.
pub struct Col<R, T> {
    name: &'static str,
    // `fn() -> (R, T)`: đánh dấu kiểu mà không giữ giá trị nào; dạng hàm để `Col` luôn `Send + Sync + Copy`
    // bất kể `R`, `T` là gì.
    _marker: PhantomData<fn() -> (R, T)>,
}

impl<R, T> Clone for Col<R, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R, T> Copy for Col<R, T> {}

impl<R, T> std::fmt::Debug for Col<R, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Col").field(&self.name).finish()
    }
}

/// A column with its value type erased, for lists of columns of different types (conflict targets).
pub struct ColName<R> {
    pub(crate) name: &'static str,
    _marker: PhantomData<fn() -> R>,
}

impl<R> Clone for ColName<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R> Copy for ColName<R> {}

impl<R: Record, T: DbValue> Col<R, T> {
    /// Column `name` (used by the derive).
    #[doc(hidden)]
    pub const fn new(name: &'static str) -> Self {
        Col { name, _marker: PhantomData }
    }

    /// The column name.
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// This column in a conflict target (`on_conflict_do_nothing([R::A.key(), R::B.key()])`).
    pub const fn key(self) -> ColName<R> {
        ColName { name: self.name, _marker: PhantomData }
    }

    pub(crate) fn sq(self) -> sea_query::Expr {
        qualified::<R>(self.name)
    }

    /// The column as an expression (arithmetic, `case_when`, aggregates).
    pub fn expr(self) -> Expr<R, T> {
        Expr::wrap(self.sq())
    }

    /// `column = value`.
    pub fn eq(self, value: T) -> Cond<R> {
        Cond::wrap(self.sq().eq(value.into_value().0))
    }

    /// `column <> value`.
    pub fn ne(self, value: T) -> Cond<R> {
        Cond::wrap(self.sq().ne(value.into_value().0))
    }

    /// `column > value`.
    pub fn gt(self, value: T) -> Cond<R> {
        Cond::wrap(self.sq().gt(value.into_value().0))
    }

    /// `column >= value`.
    pub fn gte(self, value: T) -> Cond<R> {
        Cond::wrap(self.sq().gte(value.into_value().0))
    }

    /// `column < value`.
    pub fn lt(self, value: T) -> Cond<R> {
        Cond::wrap(self.sq().lt(value.into_value().0))
    }

    /// `column <= value`.
    pub fn lte(self, value: T) -> Cond<R> {
        Cond::wrap(self.sq().lte(value.into_value().0))
    }

    /// `column IN (values)`; an empty list matches no row.
    pub fn is_in(self, values: impl IntoIterator<Item = T>) -> Cond<R> {
        let values: Vec<sea_query::Value> =
            values.into_iter().map(|value| value.into_value().0).collect();
        Cond::wrap(self.sq().is_in(values))
    }

    /// `column IS NULL`.
    pub fn is_null(self) -> Cond<R> {
        Cond::wrap(self.sq().is_null())
    }

    /// `column IS NOT NULL`.
    pub fn is_not_null(self) -> Cond<R> {
        Cond::wrap(self.sq().is_not_null())
    }

    /// `column IN (subquery)`. The subquery comes from the same handle (`handle.sub(…)`), so it is filtered by
    /// tenant like every other query.
    pub fn in_sub<S: Record>(self, sub: super::query::Sub<S, T>) -> Cond<R> {
        let mut condition = Cond::wrap(self.sq().in_subquery(sub.statement));
        condition.subs = sub.subs;
        condition
    }

    /// Join condition `self = other`.
    pub fn eq_col<S: Record>(self, other: Col<S, T>) -> On<R, S> {
        On { expr: self.sq().equals((table::<S>(), Alias::new(other.name))), _marker: PhantomData }
    }
}

impl<R: Record, T: super::value::Settable> Col<R, T> {
    /// This column in the list of columns an `on_conflict_update` rewrites. Only settable columns: never
    /// `tenant_id`.
    pub const fn updated(self) -> Updated<R> {
        Updated { name: self.name, _marker: PhantomData }
    }
}

/// A settable column in the update list of `on_conflict_update` (`R::X.updated()`).
pub struct Updated<R> {
    pub(crate) name: &'static str,
    _marker: PhantomData<fn() -> R>,
}

impl<R: Record> Col<R, String> {
    /// Contains `text`, ignoring case (`LOWER(column) LIKE '%text%'`), with `%`, `_` and `\` in `text` matched
    /// literally. Both databases compare lowercased values (SQLite's `LIKE` alone ignores ASCII case, Postgres'
    /// does not, so the kernel lowers both sides). SQLite's `LOWER` folds only ASCII letters: for text with
    /// accents, search a column stored folded (`autoxide_kernel::search::fold`), which both databases treat alike.
    pub fn contains(self, text: &str) -> Cond<R> {
        let pattern = format!("%{}%", crate::search::escape_like(&text.to_lowercase()));
        Cond::wrap(lowered(self.sq()).like(LikeExpr::new(pattern).escape('\\')))
    }

    /// Starts with `prefix`, ignoring case like [`contains`](Self::contains), matched literally. `LOWER(column)`
    /// does not use a plain index on the column: on a large table, search a column stored folded and indexed.
    pub fn starts_with(self, prefix: &str) -> Cond<R> {
        let pattern = format!("{}%", crate::search::escape_like(&prefix.to_lowercase()));
        Cond::wrap(lowered(self.sq()).like(LikeExpr::new(pattern).escape('\\')))
    }
}

/// Where a subquery inside a condition or an expression came from: the handle of tenant `Some(t)` (or of all
/// tenants, `None`), or a subquery the kernel refused (a global record on a table not registered as
/// control-plane). A query refuses a subquery of another handle, and every refused one.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubOrigin {
    Handle(Option<Uuid>),
    Refused(&'static str),
}

pub(crate) type Subs = Vec<SubOrigin>;

/// The error when a condition carries a subquery of another handle, or a refused one.
pub(crate) fn foreign_sub(subs: &[SubOrigin], tenant: Option<Uuid>) -> Option<Error> {
    subs.iter().find_map(|sub| match sub {
        SubOrigin::Handle(origin) if *origin == tenant => None,
        SubOrigin::Handle(_) => Some(Error::internal(
            "a subquery built from another handle (another tenant, or all tenants) is used in this query",
        )),
        SubOrigin::Refused(table) => Some(global_refused(table)),
    })
}

/// The fail-closed check of a global record: its table must be one a module declared in
/// `control_plane_tables` (registered when the modules are built). A global record on any other table — a
/// tenant table read without the tenant filter — is refused on every handle, whatever the lint saw.
pub(crate) fn global_check<R: Record>() -> Option<&'static str> {
    (!R::Scope::TENANT && !super::check::is_control_plane(R::TABLE)).then_some(R::TABLE)
}

pub(crate) fn global_refused(table: &str) -> Error {
    Error::internal(format!(
        "record of `{table}` is `scope = global`, but `{table}` is not a control-plane table any module declares \
         (`control_plane_tables`): a tenant table's records are `scope = tenant`"
    ))
}

fn lowered(expr: sea_query::Expr) -> sea_query::Expr {
    Func::lower(expr).into()
}

/// A condition on rows of `R`.
pub struct Cond<R> {
    pub(crate) inner: sea_query::Condition,
    pub(crate) subs: Subs,
    _marker: PhantomData<fn() -> R>,
}

impl<R> std::fmt::Debug for Cond<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cond").finish_non_exhaustive()
    }
}

impl<R> Clone for Cond<R> {
    fn clone(&self) -> Self {
        Cond { inner: self.inner.clone(), subs: self.subs.clone(), _marker: PhantomData }
    }
}

impl<R> Cond<R> {
    pub(crate) fn wrap(expr: sea_query::Expr) -> Self {
        Cond { inner: expr.into_condition(), subs: Vec::new(), _marker: PhantomData }
    }

    pub(crate) fn from_condition(inner: sea_query::Condition) -> Self {
        Cond { inner, subs: Vec::new(), _marker: PhantomData }
    }

    /// Every condition added must hold; with none added, every row matches.
    pub fn all() -> Self {
        Cond::from_condition(sea_query::Condition::all())
    }

    /// At least one condition added must hold; with none added, no row matches.
    pub fn any() -> Self {
        Cond::from_condition(sea_query::Condition::any())
    }

    /// Adds `other` to the group (as AND under [`all`](Self::all), as OR under [`any`](Self::any)).
    #[must_use]
    pub fn with(mut self, other: Cond<R>) -> Self {
        self.subs.extend(other.subs);
        Cond { inner: self.inner.add(other.inner), subs: self.subs, _marker: PhantomData }
    }
}

/// `!condition`: the negation.
impl<R> std::ops::Not for Cond<R> {
    type Output = Cond<R>;
    fn not(self) -> Self {
        Cond { inner: self.inner.not(), subs: self.subs, _marker: PhantomData }
    }
}

/// `JOIN … ON` condition between records `R` and `S` (`R::X.eq_col(S::Y)`).
pub struct On<R, S> {
    pub(crate) expr: sea_query::Expr,
    _marker: PhantomData<fn() -> (R, S)>,
}

/// A typed expression over rows of `R`: a column, a value, arithmetic, `case_when`.
pub struct Expr<R, T> {
    pub(crate) inner: sea_query::Expr,
    pub(crate) subs: Subs,
    _marker: PhantomData<fn() -> (R, T)>,
}

impl<R, T> Expr<R, T> {
    fn wrap(inner: sea_query::Expr) -> Self {
        Expr { inner, subs: Vec::new(), _marker: PhantomData }
    }

    fn join(inner: sea_query::Expr, mut subs: Subs, more: Subs) -> Self {
        subs.extend(more);
        Expr { inner, subs, _marker: PhantomData }
    }
}

impl<R: Record, T: DbValue> Expr<R, T> {
    /// A constant.
    pub fn value(value: T) -> Self {
        Expr::wrap(sea_query::Expr::val(value.into_value().0))
    }

    /// `CASE WHEN condition THEN then ELSE otherwise END`.
    pub fn case_when(condition: Cond<R>, then: Expr<R, T>, otherwise: Expr<R, T>) -> Self {
        let mut subs = condition.subs;
        subs.extend(then.subs);
        Expr::join(
            sea_query::Expr::case(condition.inner, then.inner).finally(otherwise.inner).into(),
            subs,
            otherwise.subs,
        )
    }
}

impl<R: Record, T: DbValue> From<Col<R, T>> for Expr<R, T> {
    fn from(column: Col<R, T>) -> Self {
        column.expr()
    }
}

// Phép toán trên số nguyên của cột (`a - b`, `-a`): cài các trait `std::ops` như cài toán tử trong TypeScript
// không làm được — Rust cho định nghĩa `+`, `-` cho kiểu của mình.
impl<R: Record> std::ops::Add for Expr<R, i64> {
    type Output = Expr<R, i64>;
    fn add(self, other: Self) -> Self {
        Expr::join(self.inner.add(other.inner), self.subs, other.subs)
    }
}

impl<R: Record> std::ops::Sub for Expr<R, i64> {
    type Output = Expr<R, i64>;
    fn sub(self, other: Self) -> Self {
        Expr::join(self.inner.sub(other.inner), self.subs, other.subs)
    }
}

impl<R: Record> std::ops::Mul for Expr<R, i64> {
    type Output = Expr<R, i64>;
    fn mul(self, other: Self) -> Self {
        Expr::join(self.inner.mul(other.inner), self.subs, other.subs)
    }
}

impl<R: Record> std::ops::Neg for Expr<R, i64> {
    type Output = Expr<R, i64>;
    fn neg(self) -> Self {
        Expr::join(sea_query::Expr::val(0_i64).sub(self.inner), self.subs, Vec::new())
    }
}

/// An aggregate over rows of `R`, for `select((…))` with `group_by`.
pub struct Agg<R, T> {
    pub(crate) inner: sea_query::Expr,
    pub(crate) subs: Subs,
    _marker: PhantomData<fn() -> (R, T)>,
}

impl<R, T> Agg<R, T> {
    fn new(inner: sea_query::Expr, subs: Subs) -> Self {
        Agg { inner, subs, _marker: PhantomData }
    }
}

// `CAST(COALESCE(x, 0) AS BIGINT)`: Postgres trả `numeric` cho `SUM(bigint)`, SQLite trả NULL khi không có dòng —
// ép về cùng một kiểu để module luôn đọc `i64` trên cả hai database.
fn bigint(expr: sea_query::Expr) -> sea_query::Expr {
    Func::cast_as(Func::coalesce([expr, sea_query::Expr::val(0_i64)]), Alias::new("BIGINT")).into()
}

/// `SUM(expr)` as `i64` (0 when no row).
pub fn sum<R: Record>(expr: impl Into<Expr<R, i64>>) -> Agg<R, i64> {
    let expr = expr.into();
    Agg::new(bigint(Func::sum(expr.inner).into()), expr.subs)
}

/// `COUNT(*)` as `i64`.
pub fn count<R: Record>() -> Agg<R, i64> {
    Agg::new(bigint(Func::count(sea_query::Expr::cust("*")).into()), Vec::new())
}

/// `COUNT(DISTINCT column)` as `i64`.
pub fn count_distinct<R: Record, T: DbValue>(column: Col<R, T>) -> Agg<R, i64> {
    Agg::new(bigint(Func::count_distinct(column.sq()).into()), Vec::new())
}

/// `MAX(column)`; `None` when no row.
pub fn max<R: Record, T: DbValue>(column: Col<R, T>) -> Agg<R, Option<T>>
where
    Option<T>: DbValue,
{
    Agg::new(Func::max(column.sq()).into(), Vec::new())
}

/// `MIN(column)`; `None` when no row.
pub fn min<R: Record, T: DbValue>(column: Col<R, T>) -> Agg<R, Option<T>>
where
    Option<T>: DbValue,
{
    Agg::new(Func::min(column.sq()).into(), Vec::new())
}

/// One item of a `select((…))` list: a column or an aggregate.
pub trait Selectable<R>: Sized {
    /// The type read back.
    type Out: DbValue;
    /// The expression and the tenants of the subqueries inside it.
    #[doc(hidden)]
    fn select_expr(self) -> (sea_query::Expr, Vec<SubOrigin>);
}

impl<R: Record, T: DbValue> Selectable<R> for Col<R, T> {
    type Out = T;
    fn select_expr(self) -> (sea_query::Expr, Vec<SubOrigin>) {
        (self.sq(), Vec::new())
    }
}

impl<R: Record, T: DbValue> Selectable<R> for Agg<R, T> {
    type Out = T;
    fn select_expr(self) -> (sea_query::Expr, Vec<SubOrigin>) {
        (self.inner, self.subs)
    }
}

/// A whole `select((…))` list (a tuple of [`Selectable`]); each row reads as `Out`.
pub trait Selection<R> {
    /// The tuple each row is read as.
    type Out;
    #[doc(hidden)]
    fn exprs(self) -> Vec<(sea_query::Expr, Vec<SubOrigin>)>;
}

macro_rules! selections {
    ($( ($($name:ident $index:tt),+) )*) => {$(
        impl<Rec: Record, $($name: Selectable<Rec>),+> Selection<Rec> for ($($name,)+) {
            type Out = ($($name::Out,)+);
            fn exprs(self) -> Vec<(sea_query::Expr, Vec<SubOrigin>)> {
                vec![$(self.$index.select_expr()),+]
            }
        }
    )*};
}

selections! {
    (A 0)
    (A 0, B 1)
    (A 0, B 1, C 2)
    (A 0, B 1, C 2, D 3)
    (A 0, B 1, C 2, D 3, E 4)
    (A 0, B 1, C 2, D 3, E 4, F 5)
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6)
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7)
}
