//! Chép dữ liệu của các module từ database này sang database khác — SQLite sang
//! Postgres hay ngược lại (D-070). Dùng khi một bản cài lớn lên và chuyển sang
//! Postgres, hay để mang dữ liệu thật về máy dev.
//!
//! - Hai database phải có CÙNG phiên bản schema (cùng migration đã áp dụng). Đích phải
//!   là database MỚI mà bên gọi vừa áp dụng migration: dữ liệu migration gieo sẵn ở đích
//!   (vd role `admin`) bị xoá rồi thay bằng dữ liệu của nguồn. Lệnh `autoxide copy` kiểm
//!   đích chưa từng có migration nào trước khi gọi.
//! - Bảng được chép theo thứ tự tạo bảng ở đích (tức thứ tự migration), nên bảng được
//!   tham chiếu luôn có trước bảng tham chiếu nó (foreign key trong một module).
//! - Mỗi giá trị được đọc theo kiểu cột của nguồn và ghi theo kiểu cột của đích (UUID,
//!   thời gian, chữ, số nguyên, boolean, byte, số thực), nên cách mỗi database lưu kiểu
//!   đó không quan trọng.
//! - Mọi thứ ghi trong MỘT transaction ở đích: hỏng giữa chừng thì đích vẫn trống.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use futures_util::TryStreamExt as _;
use sea_orm::sea_query::{Alias, Expr, Query, SimpleExpr, Value};
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement, StreamTrait};
use uuid::Uuid;

use crate::database::Database;

/// Số dòng mỗi câu `INSERT` (SQLite giới hạn số tham số mỗi câu).
const BATCH_ROWS: usize = 100;

/// Lỗi khi chép, với câu nói rõ phải làm gì.
#[derive(Debug)]
pub struct TransferError(String);

impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TransferError {}

impl From<sea_orm::DbErr> for TransferError {
    fn from(error: sea_orm::DbErr) -> Self {
        TransferError(format!("database: {error}"))
    }
}

/// Kết quả chép một bảng.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CopiedTable {
    pub table: String,
    pub rows: u64,
}

impl CopiedTable {
    /// Table `table`, with `rows` rows copied.
    pub fn new(table: impl Into<String>, rows: u64) -> Self {
        CopiedTable { table: table.into(), rows }
    }
}

/// Kiểu của một cột, chung cho hai database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Uuid,
    Timestamp,
    Text,
    SmallInt,
    Int,
    BigInt,
    Bool,
    Bytes,
    Float,
}

#[derive(Debug, Clone)]
struct Column {
    name: String,
    kind: Kind,
}

/// Chép mọi bảng của `modules` (trừ bảng lịch sử migration) từ `source` sang `target`.
/// XOÁ dữ liệu đang có trong các bảng đó ở đích (dữ liệu migration gieo sẵn) trước khi
/// chép, trong cùng transaction: chỉ gọi với một database mới.
pub async fn copy_modules(
    source: &Database,
    target: &Database,
    modules: &[&str],
) -> Result<Vec<CopiedTable>, TransferError> {
    // Đọc nguồn trong một snapshot: mọi bảng như ở cùng một thời điểm, dù máy chủ vẫn
    // đang ghi.
    let snapshot = source.begin_snapshot().await?;
    let tables = module_tables(target.write(), modules).await?;
    let source_tables = module_tables(&snapshot, modules).await?;
    if let Some(missing) = tables.iter().find(|table| !source_tables.contains(table)) {
        return Err(TransferError(format!(
            "table `{missing}` exists in the target but not in the source: apply the same \
             migrations to both (`autoxide migrate up`)"
        )));
    }
    // Đọc cột của mọi bảng TRƯỚC khi mở transaction: SQLite chỉ có một kết nối ghi,
    // đang giữ transaction thì không hỏi thêm được gì qua kết nối đó.
    let mut plans = Vec::with_capacity(tables.len());
    for table in &tables {
        let from = columns(&snapshot, table).await?;
        let to = columns(target.write(), table).await?;
        plans.push((table, from, to));
    }
    let transaction = target.begin_write().await?;
    // Bảng tham chiếu xoá trước bảng được tham chiếu: ngược thứ tự tạo.
    for table in tables.iter().rev() {
        let mut delete = Query::delete();
        delete.from_table(Alias::new(table.as_str()));
        transaction.execute(&delete).await?;
    }
    let mut copied = Vec::with_capacity(tables.len());
    for (table, from, to) in plans {
        let rows = copy_table(&snapshot, &transaction, table, &from, &to).await?;
        tracing::info!(table, rows, "table copied");
        copied.push(CopiedTable { table: table.clone(), rows });
    }
    transaction.commit().await?;
    snapshot.commit().await?;
    for table in &copied {
        let rows = count(target.read(), &table.table).await?;
        if rows != table.rows {
            return Err(TransferError(format!(
                "`{}` has {rows} rows in the target after copying {} rows",
                table.table, table.rows
            )));
        }
    }
    Ok(copied)
}

/// Bảng của `modules` theo thứ tự tạo, bỏ bảng `<module>_migrations`.
async fn module_tables<C: ConnectionTrait>(
    connection: &C,
    modules: &[&str],
) -> Result<Vec<String>, TransferError> {
    let backend = connection.get_database_backend();
    let sql = match backend {
        // `rowid` của `sqlite_master` tăng theo thứ tự tạo.
        DatabaseBackend::Sqlite => {
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
             ORDER BY rowid"
        }
        // `oid` của `pg_class` tăng theo thứ tự tạo.
        DatabaseBackend::Postgres => {
            "SELECT c.relname::text AS name FROM pg_class c JOIN pg_namespace n \
             ON n.oid = c.relnamespace WHERE c.relkind = 'r' AND n.nspname = current_schema() \
             ORDER BY c.oid"
        }
        other => return Err(TransferError(format!("unsupported database {other:?}"))),
    };
    let rows = connection.query_all_raw(Statement::from_string(backend, sql)).await?;
    let mut tables = Vec::new();
    for row in rows {
        let name: String = row.try_get("", "name")?;
        let owned = modules.iter().any(|module| {
            name.strip_prefix(&format!("{module}_")).is_some_and(|rest| rest != "migrations")
        });
        if owned {
            tables.push(name);
        }
    }
    Ok(tables)
}

/// Cột của `table` và kiểu của chúng.
async fn columns<C: ConnectionTrait>(
    connection: &C,
    table: &str,
) -> Result<Vec<Column>, TransferError> {
    let backend = connection.get_database_backend();
    let statement = match backend {
        DatabaseBackend::Sqlite => Statement::from_sql_and_values(
            backend,
            "SELECT name, type AS kind FROM pragma_table_info(?) ORDER BY cid",
            [table.into()],
        ),
        DatabaseBackend::Postgres => Statement::from_sql_and_values(
            backend,
            "SELECT column_name::text AS name, data_type::text AS kind \
             FROM information_schema.columns WHERE table_schema = current_schema() \
             AND table_name = $1 ORDER BY ordinal_position",
            [table.into()],
        ),
        other => return Err(TransferError(format!("unsupported database {other:?}"))),
    };
    let rows = connection.query_all_raw(statement).await?;
    let mut columns = Vec::with_capacity(rows.len());
    for row in rows {
        let name: String = row.try_get("", "name")?;
        let declared: String = row.try_get("", "kind")?;
        let kind = kind_of(&declared).map(|kind| match (backend, kind) {
            // SQLite lưu mọi số nguyên 64 bit, dù cột khai `integer`.
            (DatabaseBackend::Sqlite, Kind::SmallInt | Kind::Int) => Kind::BigInt,
            _ => kind,
        });
        let kind = kind.ok_or_else(|| {
            TransferError(format!(
                "column `{table}.{name}` has type `{declared}`, which copying does not support"
            ))
        })?;
        columns.push(Column { name, kind });
    }
    Ok(columns)
}

/// Kiểu chung từ tên kiểu khai ở SQLite (do sea-query đặt, vd `uuid_text`) hay ở
/// Postgres (`information_schema.columns.data_type`).
fn kind_of(declared: &str) -> Option<Kind> {
    let declared = declared.to_ascii_lowercase();
    let base = declared.split('(').next().unwrap_or_default().trim();
    Some(match base {
        "uuid" | "uuid_text" => Kind::Uuid,
        "timestamp with time zone" | "timestamp_with_timezone_text" => Kind::Timestamp,
        "text" | "varchar" | "character varying" | "character" | "char" | "json_text"
        | "jsonb_text" | "enum_text" => Kind::Text,
        "smallint" | "tinyint" => Kind::SmallInt,
        "integer" | "int" => Kind::Int,
        "bigint" => Kind::BigInt,
        "boolean" | "bool" => Kind::Bool,
        "bytea" | "blob" | "varbinary_blob" => Kind::Bytes,
        "double precision" | "real" | "double" | "float" => Kind::Float,
        _ => return None,
    })
}

async fn copy_table<S: ConnectionTrait + StreamTrait, C: ConnectionTrait>(
    source: &S,
    target: &C,
    table: &str,
    from: &[Column],
    to: &[Column],
) -> Result<u64, TransferError> {
    let target_kinds: HashMap<&str, Kind> =
        to.iter().map(|column| (column.name.as_str(), column.kind)).collect();
    if let Some(missing) =
        from.iter().find(|column| !target_kinds.contains_key(column.name.as_str()))
    {
        return Err(TransferError(format!(
            "column `{table}.{}` is missing in the target: apply the same migrations to both",
            missing.name
        )));
    }
    let mut select = Query::select();
    select.from(Alias::new(table));
    for column in from {
        select.column(Alias::new(&column.name));
    }
    let backend = ConnectionTrait::get_database_backend(source);
    let rows = source.stream_raw(backend.build(&select)).await?;
    // `pin!`: luồng của database có thể tự trỏ vào mình; ghim nó tại chỗ để đọc dần.
    let mut rows = std::pin::pin!(rows);
    let mut batch: Vec<Vec<SimpleExpr>> = Vec::with_capacity(BATCH_ROWS);
    let mut copied = 0;
    while let Some(row) = rows.try_next().await? {
        let mut values = Vec::with_capacity(from.len());
        for column in from {
            let value = read(&row, column)?;
            let kind = target_kinds.get(column.name.as_str()).copied().unwrap_or(column.kind);
            values.push(SimpleExpr::Value(convert(value, kind, table, &column.name)?));
        }
        batch.push(values);
        if batch.len() == BATCH_ROWS {
            copied += insert(target, table, from, std::mem::take(&mut batch)).await?;
        }
    }
    if !batch.is_empty() {
        copied += insert(target, table, from, batch).await?;
    }
    Ok(copied)
}

/// Đọc một giá trị theo kiểu cột ở nguồn.
fn read(row: &QueryResult, column: &Column) -> Result<Value, TransferError> {
    let name = column.name.as_str();
    Ok(match column.kind {
        Kind::Uuid => Value::Uuid(row.try_get::<Option<Uuid>>("", name)?),
        Kind::Timestamp => {
            Value::ChronoDateTimeUtc(row.try_get::<Option<DateTime<Utc>>>("", name)?)
        }
        Kind::Text => Value::String(row.try_get::<Option<String>>("", name)?),
        Kind::SmallInt => Value::SmallInt(row.try_get::<Option<i16>>("", name)?),
        Kind::Int => Value::Int(row.try_get::<Option<i32>>("", name)?),
        Kind::BigInt => Value::BigInt(row.try_get::<Option<i64>>("", name)?),
        Kind::Bool => Value::Bool(row.try_get::<Option<bool>>("", name)?),
        Kind::Bytes => Value::Bytes(row.try_get::<Option<Vec<u8>>>("", name)?),
        Kind::Float => Value::Double(row.try_get::<Option<f64>>("", name)?),
    })
}

/// Đổi giá trị sang kiểu cột ở đích (vd số nguyên của SQLite sang `integer` 4 byte
/// của Postgres). Giá trị không vừa thì báo lỗi, không cắt.
fn convert(value: Value, kind: Kind, table: &str, column: &str) -> Result<Value, TransferError> {
    let too_big =
        || TransferError(format!("a value of `{table}.{column}` does not fit the target"));
    let integer = match &value {
        Value::SmallInt(number) => Some(number.map(i64::from)),
        Value::Int(number) => Some(number.map(i64::from)),
        Value::BigInt(number) => Some(*number),
        _ => None,
    };
    Ok(match (kind, integer) {
        (Kind::SmallInt, Some(number)) => {
            Value::SmallInt(number.map(i16::try_from).transpose().map_err(|_| too_big())?)
        }
        (Kind::Int, Some(number)) => {
            Value::Int(number.map(i32::try_from).transpose().map_err(|_| too_big())?)
        }
        (Kind::BigInt, Some(number)) => Value::BigInt(number),
        _ => value,
    })
}

async fn insert<C: ConnectionTrait>(
    target: &C,
    table: &str,
    columns: &[Column],
    rows: Vec<Vec<SimpleExpr>>,
) -> Result<u64, TransferError> {
    let count = rows.len() as u64;
    let mut insert = Query::insert();
    insert.into_table(Alias::new(table));
    insert.columns(columns.iter().map(|column| Alias::new(&column.name)));
    for row in rows {
        insert.values(row).map_err(|error| TransferError(error.to_string()))?;
    }
    target.execute(&insert).await?;
    Ok(count)
}

async fn count<C: ConnectionTrait>(connection: &C, table: &str) -> Result<u64, TransferError> {
    let mut select = Query::select();
    select.expr_as(Expr::cust("COUNT(*)"), Alias::new("rows")).from(Alias::new(table));
    let backend = connection.get_database_backend();
    let row = connection.query_one_raw(backend.build(&select)).await?;
    let rows: i64 = match row {
        Some(row) => row.try_get("", "rows")?,
        None => 0,
    };
    u64::try_from(rows).map_err(|error| TransferError(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nhan_kieu_cot_cua_hai_database() {
        assert_eq!(kind_of("uuid_text"), Some(Kind::Uuid));
        assert_eq!(kind_of("uuid"), Some(Kind::Uuid));
        assert_eq!(kind_of("timestamp_with_timezone_text"), Some(Kind::Timestamp));
        assert_eq!(kind_of("timestamp with time zone"), Some(Kind::Timestamp));
        assert_eq!(kind_of("varchar(255)"), Some(Kind::Text));
        assert_eq!(kind_of("character varying"), Some(Kind::Text));
        assert_eq!(kind_of("INTEGER"), Some(Kind::Int));
        assert_eq!(kind_of("bigint"), Some(Kind::BigInt));
        assert_eq!(kind_of("bytea"), Some(Kind::Bytes));
        assert_eq!(kind_of("point"), None);
    }

    #[test]
    fn so_khong_vua_kieu_dich_thi_bao_loi() {
        let big = Value::BigInt(Some(i64::from(i32::MAX) + 1));
        assert!(convert(big, Kind::Int, "t", "c").is_err());
        let small = convert(Value::BigInt(Some(7)), Kind::Int, "t", "c").unwrap();
        assert_eq!(small, Value::Int(Some(7)));
        assert_eq!(convert(Value::Int(None), Kind::BigInt, "t", "c").unwrap(), Value::BigInt(None));
    }
}
