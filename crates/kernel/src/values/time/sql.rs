//! Plugs [`Timestamp`] and [`Date`] into SeaORM / sqlx on BOTH SQLite and Postgres, keeping the old storage form.
//!
//! **Writing** (values in `INSERT`, `UPDATE`, `WHERE`…): SeaORM turns a field into a `sea_query::Value`. The
//! type from any time library has its own `Value` variant, but that variant's encoding is fixed — and for
//! `time` / `jiff` it differs from the `chrono` form already stored in SQLite. The only variant that behaves
//! differently on each database is `Value::Enum` (meant for Postgres enum types): SQLite receives exactly the
//! string we write, Postgres receives `$1::"timestamptz"` — Postgres itself reads the RFC 3339 string as
//! `timestamptz`. So values travel as
//! `Value::Enum { type_name: "timestamptz" | "date", value: <string in the old form> }`.
//!
//! The column is NOT declared as an enum (`ValueType::enum_type_name` = `None`, `column_type` is a time
//! type), so SeaORM does not cast the column when reading.
//!
//! **Reading**: SeaORM calls `TryGetable`; we take the underlying sqlx row (`try_as_pg_row` /
//! `try_as_sqlite_row`) and decode it with a hand-written `sqlx::Decode`: Postgres sends `timestamptz` in
//! binary (microseconds since 2000-01-01 UTC), SQLite returns a string (or a number) as `sqlx` + `chrono`
//! accepted.

use std::borrow::Cow;
use std::sync::Arc;

use sea_orm::sea_query::{
    ArrayType, ColumnType, Enum, EnumTypeName, Nullable, OptionEnum, ValueType, ValueTypeErr,
};
use sea_orm::sqlx::error::BoxDynError;
use sea_orm::sqlx::postgres::types::Oid;
use sea_orm::sqlx::postgres::{PgTypeInfo, PgValueFormat, PgValueRef};
use sea_orm::sqlx::sqlite::{SqliteTypeInfo, SqliteValueRef};
use sea_orm::sqlx::{Decode, Postgres, Sqlite, Type, TypeInfo as _, ValueRef as _};
use sea_orm::{ColIdx, DbErr, QueryResult, RuntimeErr, TryGetError, TryGetable, Value};

use super::format;
use super::{Date, Timestamp};

const TIMESTAMPTZ: &str = "timestamptz";
// OID của kiểu dựng sẵn trong Postgres: cố định qua mọi phiên bản (`pg_type.dat`). `sqlx` 0.9 không công bố
// hằng `PgTypeInfo::TIMESTAMPTZ` nên khai bằng OID.
const TIMESTAMPTZ_OID: Oid = Oid(1184);
const DATE_OID: Oid = Oid(1082);
const DATE: &str = "date";

/// 2000-01-01T00:00:00Z — the epoch of Postgres's binary time type — in Unix seconds.
const POSTGRES_EPOCH_UNIX_SECONDS: i64 = 946_684_800;
/// 2000-01-01 in days since 1970-01-01.
const POSTGRES_EPOCH_UNIX_DAYS: i64 = 10_957;
const MICROS_PER_SECOND: i64 = 1_000_000;
const SECONDS_PER_DAY: i64 = 86_400;

fn enum_value(type_name: &'static str, text: Option<String>) -> Value {
    let type_name = EnumTypeName::from(type_name);
    match text {
        Some(text) => {
            Value::Enum(OptionEnum::Some(Box::new(Enum { type_name, value: Cow::Owned(text) })))
        }
        None => Value::Enum(OptionEnum::None(type_name)),
    }
}

/// The string in a `Value` carrying the type name `type_name` (or `Value::String`, for hand-built values).
fn text_of(value: &Value, type_name: &str) -> Option<String> {
    match value {
        Value::Enum(OptionEnum::Some(inner)) if &*inner.type_name == type_name => {
            Some(inner.value.clone().into_owned())
        }
        Value::String(Some(text)) => Some(text.clone()),
        _ => None,
    }
}

/// The string in a `Value::Enum` carrying exactly the type name `type_name` (`Value::String` is not
/// accepted): so generic code (copying data between the two databases) can tell time columns from text
/// columns.
fn enum_text<'v>(value: &'v Value, type_name: &str) -> Option<&'v str> {
    match value {
        Value::Enum(OptionEnum::Some(inner)) if &*inner.type_name == type_name => {
            Some(&inner.value)
        }
        _ => None,
    }
}

impl Timestamp {
    /// The point in time in a `Value` created by [`Timestamp`] (`None`: NULL or not a point in time).
    pub fn from_sql_value(value: &Value) -> Option<Timestamp> {
        enum_text(value, TIMESTAMPTZ)
            .and_then(format::parse_lenient_timestamp)
            .map(Timestamp::exact)
    }
}

impl Date {
    /// The date in a `Value` created by [`Date`] (`None`: NULL or not a date).
    pub fn from_sql_value(value: &Value) -> Option<Date> {
        enum_text(value, DATE).and_then(format::parse_date).map(Date::from)
    }
}

// --- Timestamp -----------------------------------------------------------------------------------------

impl From<Timestamp> for Value {
    fn from(value: Timestamp) -> Self {
        enum_value(TIMESTAMPTZ, Some(value.to_sql_text()))
    }
}

impl Nullable for Timestamp {
    fn null() -> Value {
        enum_value(TIMESTAMPTZ, None)
    }
}

impl ValueType for Timestamp {
    fn try_from(value: Value) -> Result<Self, ValueTypeErr> {
        text_of(&value, TIMESTAMPTZ)
            .and_then(|text| format::parse_lenient_timestamp(&text))
            .map(Timestamp::exact)
            .ok_or(ValueTypeErr)
    }

    fn type_name() -> String {
        "Timestamp".to_owned()
    }

    fn array_type() -> ArrayType {
        ArrayType::Enum(Box::new(EnumTypeName::from(TIMESTAMPTZ)))
    }

    fn column_type() -> ColumnType {
        ColumnType::TimestampWithTimeZone
    }
}

impl TryGetable for Timestamp {
    fn try_get_by<I: ColIdx>(result: &QueryResult, index: I) -> Result<Self, TryGetError> {
        if let Some(row) = result.try_as_pg_row() {
            return present(
                sea_orm::sqlx::Row::try_get(row, index.as_sqlx_postgres_index()),
                index,
            );
        }
        if let Some(row) = result.try_as_sqlite_row() {
            return present(sea_orm::sqlx::Row::try_get(row, index.as_sqlx_sqlite_index()), index);
        }
        Err(unsupported_row())
    }
}

impl Type<Postgres> for Timestamp {
    fn type_info() -> PgTypeInfo {
        PgTypeInfo::with_oid(TIMESTAMPTZ_OID)
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        ty.oid() == Some(TIMESTAMPTZ_OID)
    }
}

impl<'r> Decode<'r, Postgres> for Timestamp {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        match value.format() {
            PgValueFormat::Binary => {
                let micros = <i64 as Decode<Postgres>>::decode(value)?;
                let unix_micros = micros
                    .checked_add(POSTGRES_EPOCH_UNIX_SECONDS * MICROS_PER_SECOND)
                    .and_then(Timestamp::from_unix_micros);
                unix_micros.ok_or_else(|| format!("timestamptz out of range: {micros}").into())
            }
            // Dạng chữ chỉ có ở giao thức truy vấn đơn giản (`2026-11-21 10:00:00.123456+00`).
            PgValueFormat::Text => {
                let text = value.as_str()?;
                format::parse_timestamp(text)
                    .map(Timestamp::exact)
                    .ok_or_else(|| format!("invalid timestamptz: {text}").into())
            }
        }
    }
}

impl Type<Sqlite> for Timestamp {
    fn type_info() -> SqliteTypeInfo {
        <String as Type<Sqlite>>::type_info()
    }

    fn compatible(ty: &SqliteTypeInfo) -> bool {
        // Như `chrono::DateTime` của `sqlx`: chuỗi, số nguyên (giây Unix), số thực (ngày Julian).
        matches!(ty.name(), "TEXT" | "DATETIME" | "INTEGER" | "REAL" | "NUMERIC")
    }
}

impl<'r> Decode<'r, Sqlite> for Timestamp {
    fn decode(value: SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
        let kind = value.type_info().name().to_owned();
        match kind.as_str() {
            "INTEGER" => {
                let seconds = <i64 as Decode<Sqlite>>::decode(value)?;
                Timestamp::from_unix_seconds(seconds)
                    .ok_or_else(|| format!("invalid datetime: {seconds}").into())
            }
            "REAL" => {
                let julian_day = <f64 as Decode<Sqlite>>::decode(value)?;
                from_julian_day(julian_day)
                    .ok_or_else(|| format!("invalid datetime: {julian_day}").into())
            }
            _ => {
                let text = <&str as Decode<Sqlite>>::decode(value)?;
                format::parse_lenient_timestamp(text)
                    .map(Timestamp::exact)
                    .ok_or_else(|| format!("invalid datetime: {text}").into())
            }
        }
    }
}

/// Julian day (the float SQLite returns from `julianday()`) → point in time, as `sqlx` + `chrono` read it.
fn from_julian_day(julian_day: f64) -> Option<Timestamp> {
    const UNIX_EPOCH_JULIAN_DAY: f64 = 2_440_587.5;
    let seconds = (julian_day - UNIX_EPOCH_JULIAN_DAY) * 86_400.0;
    if !seconds.is_finite() || seconds.abs() > 1e12 {
        return None;
    }
    // Đổi số thực sang nguyên có mất phần lẻ là cố ý (micro giây); đã chặn NaN, vô cực và giá trị quá lớn ở trên.
    #[allow(clippy::cast_possible_truncation)]
    let micros = (seconds * 1e6).trunc() as i64;
    Timestamp::from_unix_micros(micros)
}

// --- Date ----------------------------------------------------------------------------------------------

impl From<Date> for Value {
    fn from(value: Date) -> Self {
        enum_value(DATE, Some(value.to_string()))
    }
}

impl Nullable for Date {
    fn null() -> Value {
        enum_value(DATE, None)
    }
}

impl ValueType for Date {
    fn try_from(value: Value) -> Result<Self, ValueTypeErr> {
        text_of(&value, DATE)
            .and_then(|text| format::parse_date(&text))
            .map(Date::from)
            .ok_or(ValueTypeErr)
    }

    fn type_name() -> String {
        "Date".to_owned()
    }

    fn array_type() -> ArrayType {
        ArrayType::Enum(Box::new(EnumTypeName::from(DATE)))
    }

    fn column_type() -> ColumnType {
        ColumnType::Date
    }
}

impl TryGetable for Date {
    fn try_get_by<I: ColIdx>(result: &QueryResult, index: I) -> Result<Self, TryGetError> {
        if let Some(row) = result.try_as_pg_row() {
            return present(
                sea_orm::sqlx::Row::try_get(row, index.as_sqlx_postgres_index()),
                index,
            );
        }
        if let Some(row) = result.try_as_sqlite_row() {
            return present(sea_orm::sqlx::Row::try_get(row, index.as_sqlx_sqlite_index()), index);
        }
        Err(unsupported_row())
    }
}

impl Type<Postgres> for Date {
    fn type_info() -> PgTypeInfo {
        PgTypeInfo::with_oid(DATE_OID)
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        ty.oid() == Some(DATE_OID)
    }
}

impl<'r> Decode<'r, Postgres> for Date {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        match value.format() {
            PgValueFormat::Binary => {
                let days = i64::from(<i32 as Decode<Postgres>>::decode(value)?);
                let unix_seconds = (POSTGRES_EPOCH_UNIX_DAYS + days) * SECONDS_PER_DAY;
                jiff::Timestamp::from_second(unix_seconds)
                    .map(|timestamp| {
                        Date::from(timestamp.to_zoned_datetime(jiff::tz::TimeZone::UTC).date())
                    })
                    .map_err(|_| format!("date out of range: {days}").into())
            }
            PgValueFormat::Text => {
                let text = value.as_str()?;
                format::parse_date(text)
                    .map(Date::from)
                    .ok_or_else(|| format!("invalid date: {text}").into())
            }
        }
    }
}

impl Type<Sqlite> for Date {
    fn type_info() -> SqliteTypeInfo {
        <String as Type<Sqlite>>::type_info()
    }

    fn compatible(ty: &SqliteTypeInfo) -> bool {
        matches!(ty.name(), "TEXT" | "DATE")
    }
}

impl<'r> Decode<'r, Sqlite> for Date {
    fn decode(value: SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
        let text = <&str as Decode<Sqlite>>::decode(value)?;
        format::parse_date(text)
            .map(Date::from)
            .ok_or_else(|| format!("invalid date: {text}").into())
    }
}

// --- Dùng chung ----------------------------------------------------------------------------------------

/// Result of sqlx's `try_get::<Option<T>>` → the result SeaORM expects: an sqlx error becomes `DbErr`, NULL
/// becomes `TryGetError::Null` (SeaORM then turns it into `None` for an `Option<T>` field).
fn present<T, I: ColIdx>(
    result: Result<Option<T>, sea_orm::sqlx::Error>,
    index: I,
) -> Result<T, TryGetError> {
    match result {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Err(TryGetError::Null(format!("{index:?}"))),
        Err(error) => Err(TryGetError::DbErr(DbErr::Query(RuntimeErr::SqlxError(Arc::new(error))))),
    }
}

fn unsupported_row() -> TryGetError {
    TryGetError::DbErr(DbErr::Type(
        "time values are only read from SQLite or Postgres rows".to_owned(),
    ))
}

// SeaORM đòi kiểu của cột khoá chính cài `TryFromU64` (để đọc ID tự tăng sau INSERT). Ngày / thời điểm
// không bao giờ là ID tự tăng: luôn báo lỗi, như cài đặt sẵn của SeaORM cho kiểu `chrono`.
impl sea_orm::TryFromU64 for Timestamp {
    fn try_from_u64(_: u64) -> Result<Self, DbErr> {
        Err(DbErr::ConvertFromU64("Timestamp"))
    }
}

impl sea_orm::TryFromU64 for Date {
    fn try_from_u64(_: u64) -> Result<Self, DbErr> {
        Err(DbErr::ConvertFromU64("Date"))
    }
}
