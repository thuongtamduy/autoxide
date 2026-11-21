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
//!
//! Dời một khách giữa hai placement (đặc tả 33 "Cấp phát và dời khách") dùng [`TenantTables`]:
//! cùng cách đọc / ghi theo kiểu cột, nhưng chỉ dòng `tenant_id = X` của bảng cấp khách hàng (bỏ
//! bảng mặt điều khiển), và bên gọi tự mở transaction ở đích.

use crate::time::{Date, Timestamp};
use futures_util::TryStreamExt as _;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query, SimpleExpr, Value};
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement, StreamTrait};
use std::collections::HashMap;
use uuid::Uuid;

use crate::database::Database;
use crate::tenancy::TenantId;

mod filtered;

pub(crate) use filtered::{Row, canonical, foreign_keys, primary_key, tenant_is, unique_indexes};

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

impl TransferError {
    /// Lỗi với câu `message`.
    pub(crate) fn new(message: impl Into<String>) -> Self {
        TransferError(message.into())
    }
}

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
    /// Ngày (Postgres `date`, SQLite `date_text` — chữ `YYYY-MM-DD`).
    Date,
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
        let rows = copy_table(&snapshot, &transaction, table, &from, &to, None).await?;
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
        DatabaseBackend::Sqlite => crate::database::statement(
            backend,
            "SELECT name, type AS kind FROM pragma_table_info(?) ORDER BY cid",
            [table.into()],
        ),
        DatabaseBackend::Postgres => crate::database::statement(
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
        "date" | "date_text" => Kind::Date,
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
    tenant: Option<TenantId>,
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
    if let Some(tenant) = tenant {
        select.and_where(Expr::col(Alias::new(TENANT_COLUMN)).eq(tenant.as_uuid()));
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
        Kind::Timestamp => Value::from(row.try_get::<Option<Timestamp>>("", name)?),
        Kind::Date => Value::from(row.try_get::<Option<Date>>("", name)?),
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
    count_where(connection, table, None).await
}

async fn count_where<C: ConnectionTrait>(
    connection: &C,
    table: &str,
    tenant: Option<TenantId>,
) -> Result<u64, TransferError> {
    let mut select = Query::select();
    select.expr_as(Expr::cust("COUNT(*)"), Alias::new("rows")).from(Alias::new(table));
    if let Some(tenant) = tenant {
        select.and_where(Expr::col(Alias::new(TENANT_COLUMN)).eq(tenant.as_uuid()));
    }
    let backend = connection.get_database_backend();
    let row = connection.query_one_raw(backend.build(&select)).await?;
    let rows: i64 = match row {
        Some(row) => row.try_get("", "rows")?,
        None => 0,
    };
    u64::try_from(rows).map_err(|error| TransferError(error.to_string()))
}

/// Cột khách của bảng cấp khách hàng.
const TENANT_COLUMN: &str = "tenant_id";

/// Một bảng cấp khách hàng: cột ở nguồn và ở đích.
#[derive(Debug, Clone)]
struct TablePlan {
    name: String,
    from: Vec<Column>,
    to: Vec<Column>,
}

/// Loại một bảng cấp khách hàng, nhận theo cột.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TableKind {
    /// Dữ liệu của module.
    Data,
    /// Hàng đợi job `<module>_jobs` (có `run_at`, `claimed_until`, `completed_at`).
    Jobs,
    /// Outbox `<module>_events` (có `next_attempt_at`, `claimed_until`, `delivered_at`).
    Events,
}

impl TablePlan {
    fn kind(&self) -> TableKind {
        let has = |name: &str| self.from.iter().any(|column| column.name == name);
        if has("claimed_until") && has("run_at") && has("completed_at") {
            TableKind::Jobs
        } else if has("claimed_until") && has("next_attempt_at") && has("delivered_at") {
            TableKind::Events
        } else {
            TableKind::Data
        }
    }
}

/// Bảng cấp khách hàng của các module ở hai database cùng schema, theo thứ tự tạo bảng ở đích —
/// để đếm, chép và xoá dòng của MỘT khách (dời khách giữa hai placement, đặc tả 33).
///
/// - Bỏ bảng `<module>_migrations` và bảng mặt điều khiển (chỉ database điều khiển có dữ liệu).
/// - Mọi bảng còn lại phải có cột `tenant_id` (luật bảng cấp khách hàng, D-253); thiếu thì báo lỗi
///   thay vì chép cả bảng.
/// - Dòng của khách được đọc qua kết nối luôn thấy dữ liệu mới nhất ([`fresh`]): SQLite đọc qua
///   pool đọc (không giữ kết nối ghi duy nhất của database chung), Postgres qua pool ghi (pool đọc
///   có thể là read replica đang trễ).
#[derive(Debug, Clone)]
pub struct TenantTables {
    tables: Vec<TablePlan>,
}

/// Kết nối thấy mọi thay đổi đã commit của `database` (xem [`TenantTables`]).
fn fresh(database: &Database) -> &sea_orm::DatabaseConnection {
    match database.backend() {
        crate::config::DatabaseBackend::Sqlite => database.read(),
        crate::config::DatabaseBackend::Postgres => database.write(),
    }
}

impl TenantTables {
    /// Bảng cấp khách hàng của `modules` (trừ `control_plane`) có ở `target`, kèm cột của chúng ở
    /// `source` và `target`. Bảng thiếu ở nguồn hay thiếu cột: hai database chưa cùng schema.
    pub async fn read(
        source: &Database,
        target: &Database,
        modules: &[&str],
        control_plane: &[&str],
    ) -> Result<Self, TransferError> {
        let keep = |table: &String| !control_plane.contains(&table.as_str());
        let names: Vec<String> =
            module_tables(target.write(), modules).await?.into_iter().filter(keep).collect();
        let source_names = module_tables(source.write(), modules).await?;
        let mut tables = Vec::with_capacity(names.len());
        for name in names {
            if !source_names.contains(&name) {
                return Err(TransferError(format!(
                    "table `{name}` exists in the target but not in the source: apply the same \
                     migrations to both (`autoxide migrate up --all`)"
                )));
            }
            let from = columns(source.write(), &name).await?;
            let to = columns(target.write(), &name).await?;
            if !from.iter().any(|column| column.name == TENANT_COLUMN) {
                return Err(TransferError(format!(
                    "table `{name}` has no `{TENANT_COLUMN}` column: tenant-level tables carry it; \
                     a table shared by the whole platform is declared in \
                     `Module::control_plane_tables`"
                )));
            }
            tables.push(TablePlan { name, from, to });
        }
        Ok(TenantTables { tables })
    }

    /// Tên bảng, theo thứ tự tạo.
    pub fn names(&self) -> Vec<&str> {
        self.tables.iter().map(|table| table.name.as_str()).collect()
    }

    /// Loại của bảng `name` (bảng không có trong danh sách: [`TableKind::Data`]).
    pub fn kind(&self, name: &str) -> TableKind {
        self.tables.iter().find(|table| table.name == name).map_or(TableKind::Data, TablePlan::kind)
    }

    /// Hàng đợi job và outbox trong các bảng, kèm loại.
    pub fn queues(&self) -> Vec<(&str, TableKind)> {
        self.tables
            .iter()
            .map(|table| (table.name.as_str(), table.kind()))
            .filter(|(_, kind)| *kind != TableKind::Data)
            .collect()
    }

    /// Số dòng của khách `tenant` trong từng bảng của `database`, theo thứ tự tạo.
    pub async fn count(
        &self,
        database: &Database,
        tenant: TenantId,
    ) -> Result<Vec<CopiedTable>, TransferError> {
        let connection = fresh(database);
        let mut counts = Vec::with_capacity(self.tables.len());
        for table in &self.tables {
            let rows = count_where(connection, &table.name, Some(tenant)).await?;
            counts.push(CopiedTable { table: table.name.clone(), rows });
        }
        Ok(counts)
    }

    /// Xoá dòng của khách `tenant` trong mọi bảng có tên thoả `only`, qua `connection` (thường
    /// là transaction của bên gọi): bảng tham chiếu trước bảng được tham chiếu (ngược thứ tự tạo).
    /// Trả số dòng đã xoá.
    pub async fn delete<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: TenantId,
        only: impl Fn(&str) -> bool,
    ) -> Result<u64, TransferError> {
        let mut deleted = 0;
        for table in self.tables.iter().rev().filter(|table| only(&table.name)) {
            let mut delete = Query::delete();
            delete
                .from_table(Alias::new(table.name.as_str()))
                .and_where(Expr::col(Alias::new(TENANT_COLUMN)).eq(tenant.as_uuid()));
            deleted += connection.execute(&delete).await?.rows_affected();
        }
        Ok(deleted)
    }

    /// Chép dòng của khách `tenant` từ `source` vào `target` (transaction ở database đích của bên
    /// gọi): xoá dòng của khách đang có ở đích trước (lần chép trước bị ngắt, bản giữ lại của lần
    /// dời trước), rồi chép từng bảng theo thứ tự tạo. Trả số dòng đã chép mỗi bảng.
    pub async fn copy<C: ConnectionTrait>(
        &self,
        source: &Database,
        target: &C,
        tenant: TenantId,
    ) -> Result<Vec<CopiedTable>, TransferError> {
        self.delete(target, tenant, |_| true).await?;
        let connection = fresh(source);
        let mut copied = Vec::with_capacity(self.tables.len());
        for table in &self.tables {
            let rows =
                copy_table(connection, target, &table.name, &table.from, &table.to, Some(tenant))
                    .await?;
            tracing::info!(table = %table.name, rows, tenant_id = %tenant, "tenant rows copied");
            copied.push(CopiedTable { table: table.name.clone(), rows });
        }
        Ok(copied)
    }
}

/// Cột không đưa vào bản xuất dữ liệu của khách (bước 35, D-328): bản băm mật khẩu / token, secret,
/// dữ liệu đã niêm phong — không giúp gì khách khi mang dữ liệu đi, chỉ thêm rủi ro nếu bản xuất lộ.
pub fn is_secret_column(name: &str) -> bool {
    name.ends_with("_hash")
        || name.ends_with("_token")
        || name.contains("secret")
        || name.contains("sealed")
        || name == "password"
}

impl TenantTables {
    /// Xuất mọi dòng của khách `tenant` ở `source` thành JSON Lines (`{"table": …, "row": {…}}` mỗi
    /// dòng), gửi từng khúc ~64 KiB qua `sink`. Bỏ hàng đợi job / outbox và cột bí mật
    /// ([`is_secret_column`]). Byte ghi base64, thời gian RFC 3339 UTC. Trả số dòng mỗi bảng.
    pub async fn export(
        &self,
        source: &Database,
        tenant: TenantId,
        sink: &tokio::sync::mpsc::Sender<Result<bytes::Bytes, crate::error::Error>>,
    ) -> Result<Vec<CopiedTable>, TransferError> {
        use base64::Engine as _;
        const CHUNK: usize = 64 * 1024;
        let connection = fresh(source);
        let backend = ConnectionTrait::get_database_backend(connection);
        let mut buffer: Vec<u8> = Vec::with_capacity(CHUNK * 2);
        let closed =
            || TransferError("the export was stopped while it was being written".to_owned());
        let mut counted = Vec::new();
        for table in self.tables.iter().filter(|table| table.kind() == TableKind::Data) {
            let columns: Vec<&Column> =
                table.from.iter().filter(|column| !is_secret_column(&column.name)).collect();
            let mut select = Query::select();
            select.from(Alias::new(table.name.as_str()));
            for column in &columns {
                select.column(Alias::new(column.name.as_str()));
            }
            select.and_where(Expr::col(Alias::new(TENANT_COLUMN)).eq(tenant.as_uuid()));
            let rows = connection.stream_raw(backend.build(&select)).await?;
            let mut rows = std::pin::pin!(rows);
            let mut count = 0;
            while let Some(row) = rows.try_next().await? {
                let mut object = serde_json::Map::with_capacity(columns.len());
                for column in &columns {
                    let value = match read(&row, column)? {
                        Value::Uuid(value) => {
                            value.map(|id| serde_json::Value::from(id.to_string()))
                        }
                        // Thời điểm: chuỗi RFC 3339 `+00:00` như trước. (Cột ngày vẫn ra `null` như trước.)
                        ref value @ Value::Enum(_) => Timestamp::from_sql_value(value)
                            .map(|at| serde_json::Value::from(at.to_rfc3339())),
                        Value::String(value) => value.map(serde_json::Value::from),
                        Value::SmallInt(value) => value.map(serde_json::Value::from),
                        Value::Int(value) => value.map(serde_json::Value::from),
                        Value::BigInt(value) => value.map(serde_json::Value::from),
                        Value::Bool(value) => value.map(serde_json::Value::from),
                        Value::Double(value) => value.map(serde_json::Value::from),
                        Value::Bytes(value) => value.map(|bytes| {
                            serde_json::Value::from(
                                base64::engine::general_purpose::STANDARD.encode(bytes),
                            )
                        }),
                        _ => None,
                    };
                    object.insert(column.name.clone(), value.unwrap_or(serde_json::Value::Null));
                }
                let line = serde_json::json!({ "table": table.name, "row": object });
                serde_json::to_writer(&mut buffer, &line)
                    .map_err(|error| TransferError(error.to_string()))?;
                buffer.push(b'\n');
                count += 1;
                if buffer.len() >= CHUNK {
                    let chunk = bytes::Bytes::from(std::mem::take(&mut buffer));
                    sink.send(Ok(chunk)).await.map_err(|_| closed())?;
                }
            }
            counted.push(CopiedTable { table: table.name.clone(), rows: count });
        }
        if !buffer.is_empty() {
            sink.send(Ok(bytes::Bytes::from(buffer))).await.map_err(|_| closed())?;
        }
        Ok(counted)
    }
}

/// `a` và `b` là cùng một database không (hai placement trỏ tới cùng một chỗ): SQLite so file,
/// Postgres so tên database, schema và lúc máy chủ khởi động (`pg_postmaster_start_time`, khác
/// nhau giữa hai máy chủ dù cùng tên database). Dời khách giữa hai "database" thật ra là một sẽ
/// xoá mất dữ liệu ở bước dọn nguồn — bên gọi phải từ chối.
pub async fn same_database(a: &Database, b: &Database) -> Result<bool, TransferError> {
    if a.backend() != b.backend() {
        return Ok(false);
    }
    Ok(identity(a).await? == identity(b).await?)
}

async fn identity(database: &Database) -> Result<String, TransferError> {
    let connection = database.write();
    let backend = connection.get_database_backend();
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "SELECT file AS id FROM pragma_database_list WHERE name = 'main'"
        }
        DatabaseBackend::Postgres => {
            "SELECT current_database()::text || '/' || current_schema()::text || '/' || \
             pg_postmaster_start_time()::text AS id"
        }
        other => return Err(TransferError(format!("unsupported database {other:?}"))),
    };
    let row = connection.query_one_raw(Statement::from_string(backend, sql)).await?;
    let id: String = match row {
        Some(row) => row.try_get::<Option<String>>("", "id")?.unwrap_or_default(),
        None => String::new(),
    };
    if backend == DatabaseBackend::Sqlite {
        // Database trong bộ nhớ (không file): mỗi kết nối một database riêng.
        if id.is_empty() {
            return Ok(uuid::Uuid::now_v7().to_string());
        }
        return Ok(
            std::fs::canonicalize(&id).map_or(id, |path| path.to_string_lossy().into_owned())
        );
    }
    Ok(id)
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
        assert_eq!(kind_of("date_text"), Some(Kind::Date));
        assert_eq!(kind_of("date"), Some(Kind::Date));
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
