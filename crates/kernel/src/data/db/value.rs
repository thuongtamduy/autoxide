//! Values a column holds, rows read back, and the tenant column type.

use crate::error::Error;
use crate::time::{Date, Timestamp};
use sea_orm::QueryResult;
use sea_orm::sea_query;
use uuid::Uuid;

/// A value sent to the database. Opaque: modules build it through [`DbValue::into_value`], never by hand.
#[derive(Debug, Clone, PartialEq)]
pub struct Value(pub(crate) sea_query::Value);

/// One row read back from the database. Opaque: read through [`DbValue::read`].
pub struct Row<'a>(pub(crate) &'a QueryResult);

impl std::fmt::Debug for Row<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Row").finish_non_exhaustive()
    }
}

/// A type a column can hold: `bool`, `i16`, `i32`, `i64`, `String`, `Uuid`, `Timestamp`, `Date`,
/// `serde_json::Value`, `Vec<u8>`, `Option` of each, and [`TenantKey`].
///
/// A module stores its own type by implementing this trait on top of one of those (e.g. an enum stored as
/// text: convert to `String` in `into_value`, parse in `read`).
pub trait DbValue: Sized + Send + Sync + 'static {
    /// The value to send.
    fn into_value(self) -> Value;
    /// The value at position `index` of `row`.
    fn read(row: &Row<'_>, index: usize) -> Result<Self, Error>;
}

/// A value `update().set(…)` may write. Every [`DbValue`] of the list above is settable; [`TenantKey`] is
/// not, so moving a row to another tenant does not compile.
pub trait Settable: DbValue {}

// `macro_rules!`: viết cùng một khối `impl` cho nhiều kiểu, mỗi kiểu một lần (Rust không có "impl cho danh sách
// kiểu"). Mỗi kiểu dùng đúng cách đọc / ghi của SeaORM bên trong; module không thấy SeaORM.
macro_rules! db_values {
    ($($ty:ty),* $(,)?) => {$(
        impl DbValue for $ty {
            fn into_value(self) -> Value {
                Value(self.into())
            }
            fn read(row: &Row<'_>, index: usize) -> Result<Self, Error> {
                Ok(row.0.try_get_by_index::<$ty>(index)?)
            }
        }
        impl Settable for $ty {}

        impl DbValue for Option<$ty> {
            fn into_value(self) -> Value {
                Value(self.into())
            }
            fn read(row: &Row<'_>, index: usize) -> Result<Self, Error> {
                Ok(row.0.try_get_by_index::<Option<$ty>>(index)?)
            }
        }
        impl Settable for Option<$ty> {}
    )*};
}

db_values!(bool, i16, i32, i64, String, Uuid, Timestamp, Date, serde_json::Value, Vec<u8>);

/// A NOT NULL column type a list can be ordered by with `Find::page_by`: text, whole numbers, timestamps.
/// `Option<…>` is not one — the two databases place `NULL` differently, so paging would repeat or skip rows.
/// Sealed: only the kernel implements it.
pub trait SortKey: DbValue + Clone + sort_key::Sealed {
    #[doc(hidden)]
    fn into_sort_value(self) -> crate::pagination::SortValue;
    #[doc(hidden)]
    fn from_sort_value(value: &crate::pagination::SortValue) -> Option<Self>;
}

mod sort_key {
    pub trait Sealed {}
}

// `macro_rules!`: cùng một khối cho các kiểu số nguyên; con trỏ chở `i64`, đọc lại phải vừa kiểu của cột (giá
// trị bị sửa tay ngoài khoảng thì con trỏ hỏng, không tới database).
macro_rules! integer_sort_keys {
    ($($ty:ty),*) => {$(
        impl sort_key::Sealed for $ty {}
        impl SortKey for $ty {
            fn into_sort_value(self) -> crate::pagination::SortValue {
                crate::pagination::SortValue::Integer(i64::from(self))
            }
            fn from_sort_value(value: &crate::pagination::SortValue) -> Option<Self> {
                match value {
                    crate::pagination::SortValue::Integer(number) => <$ty>::try_from(*number).ok(),
                    _ => None,
                }
            }
        }
    )*};
}

integer_sort_keys!(i16, i32, i64);

impl sort_key::Sealed for String {}
impl SortKey for String {
    fn into_sort_value(self) -> crate::pagination::SortValue {
        crate::pagination::SortValue::Text(self)
    }
    fn from_sort_value(value: &crate::pagination::SortValue) -> Option<Self> {
        match value {
            crate::pagination::SortValue::Text(text) => Some(text.clone()),
            _ => None,
        }
    }
}

impl sort_key::Sealed for Timestamp {}
impl SortKey for Timestamp {
    fn into_sort_value(self) -> crate::pagination::SortValue {
        crate::pagination::SortValue::Time(self)
    }
    fn from_sort_value(value: &crate::pagination::SortValue) -> Option<Self> {
        match value {
            crate::pagination::SortValue::Time(time) => Some(*time),
            _ => None,
        }
    }
}

/// The `tenant_id` column of a tenant table. It can be read and compared, never written by
/// `update().set(…)`: the kernel sets it on insert from the tenant of the handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TenantKey(Uuid);

impl TenantKey {
    /// The key of tenant `id`.
    pub const fn new(id: crate::tenancy::TenantId) -> Self {
        TenantKey(id.as_uuid())
    }

    /// The tenant id as a UUID.
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl From<crate::tenancy::TenantId> for TenantKey {
    fn from(id: crate::tenancy::TenantId) -> Self {
        TenantKey::new(id)
    }
}

impl DbValue for TenantKey {
    fn into_value(self) -> Value {
        Value(self.0.into())
    }
    fn read(row: &Row<'_>, index: usize) -> Result<Self, Error> {
        Ok(TenantKey(row.0.try_get_by_index::<Uuid>(index)?))
    }
}

/// A row read as a tuple: `select((a, b, sum(c)))` gives `(A, B, i64)`.
pub trait FromRow: Sized {
    /// Reads the row.
    fn from_row(row: &Row<'_>) -> Result<Self, Error>;
}

macro_rules! tuples {
    ($( ($($name:ident $index:tt),+) )*) => {$(
        impl<$($name: DbValue),+> FromRow for ($($name,)+) {
            fn from_row(row: &Row<'_>) -> Result<Self, Error> {
                Ok(($($name::read(row, $index)?,)+))
            }
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
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6)
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7)
}
