//! Copies the data of the modules from one database to another: SQLite to
//! Postgres or the reverse (D-070). Used when an installation grows and moves to
//! Postgres, or to bring real data to a dev machine.
//!
//! - Both databases must have the SAME schema version (the same migrations applied). The target must be a
//!   NEW database to which the caller has just applied the migrations: data the migrations seeded at the
//!   target (e.g. the `admin` role) is deleted and replaced by the source's data. The command
//!   `autoxide copy` checks that the target has never had any migration before calling.
//! - Tables are copied in the order they were created at the target (that is, migration order), so a
//!   referenced table always comes before the table that references it (foreign key within a module).
//! - Each value is read by the source's column type and written by the target's column type (UUID,
//!   time, text, integer, boolean, bytes, float), so how each database stores that type
//!   does not matter.
//! - Everything is written in ONE transaction at the target: if it fails midway the target stays empty.
//!
//! Moving a tenant between two placements (spec 33 "Provisioning and moving tenants") uses
//! [`TenantTables`]: the same column-typed reading / writing, but only the `tenant_id = X` rows of
//! tenant-level tables (leaving out control-plane tables), and the caller opens the transaction at the
//! target itself.

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

/// Number of rows per `INSERT` statement (SQLite limits the number of parameters per statement).
const BATCH_ROWS: usize = 100;

/// Error while copying, with a sentence that says what to do.
#[derive(Debug)]
pub struct TransferError(String);

impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TransferError {}

impl TransferError {
    /// An error with the sentence `message`.
    pub(crate) fn new(message: impl Into<String>) -> Self {
        TransferError(message.into())
    }
}

impl From<sea_orm::DbErr> for TransferError {
    fn from(error: sea_orm::DbErr) -> Self {
        TransferError(format!("database: {error}"))
    }
}

/// Result of copying one table.
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

/// The type of a column, common to both databases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Uuid,
    Timestamp,
    /// Date (Postgres `date`, SQLite `date_text`: the text `YYYY-MM-DD`).
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
pub(crate) struct Column {
    pub(crate) name: String,
    kind: Kind,
}

/// Copies every table of `modules` (except the migration history tables) from `source` to `target`.
/// DELETES the data currently in those tables at the target (data seeded by migrations) before
/// copying, in the same transaction: call it only with a new database.
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

/// The tables of `modules` in creation order, leaving out the `<module>_migrations` tables and, on SQLite,
/// virtual tables and their shadow tables (derived data a module rebuilds with triggers).
pub(crate) async fn module_tables<C: ConnectionTrait>(
    connection: &C,
    modules: &[&str],
) -> Result<Vec<String>, TransferError> {
    let backend = connection.get_database_backend();
    let sql = match backend {
        // `rowid` của `sqlite_master` tăng theo thứ tự tạo. Bảng ảo (FTS5…) và bảng phụ của chúng không chép: là
        // dữ liệu dẫn xuất, trigger của module dựng lại khi chép bảng gốc (D-20261126-content-word-index).
        DatabaseBackend::Sqlite => {
            "SELECT m.name AS name FROM sqlite_master m WHERE m.type = 'table' \
             AND m.name NOT LIKE 'sqlite_%' AND NOT EXISTS (SELECT 1 FROM pragma_table_list t \
             WHERE t.schema = 'main' AND t.name = m.name AND t.type IN ('virtual', 'shadow')) \
             ORDER BY m.rowid"
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

/// The columns of `table` and their types.
pub(crate) async fn columns<C: ConnectionTrait>(
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

/// The common type from the type name declared in SQLite (set by sea-query, e.g. `uuid_text`) or in
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

/// Reads a value by the column type at the source.
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

/// Converts a value to the column type at the target (e.g. a SQLite integer to a 4-byte `integer`
/// of Postgres). A value that does not fit is an error, not truncated.
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

/// The tenant column of a tenant-level table.
const TENANT_COLUMN: &str = "tenant_id";

/// A tenant-level table: the columns at the source and at the target.
#[derive(Debug, Clone)]
struct TablePlan {
    name: String,
    from: Vec<Column>,
    to: Vec<Column>,
}

/// The kind of a tenant-level table, recognized by its columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TableKind {
    /// Module data.
    Data,
    /// The job queue `<module>_jobs` (has `run_at`, `claimed_until`, `completed_at`).
    Jobs,
    /// The outbox `<module>_events` (has `next_attempt_at`, `claimed_until`, `delivered_at`).
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

/// The tenant-level tables of the modules in two databases of the same schema, in creation order at the target,
/// to count, copy and delete the rows of ONE tenant (moving a tenant between two placements, spec 33).
///
/// - Leaves out `<module>_migrations` tables and control-plane tables (only the control database holds data).
/// - Every other table must have a `tenant_id` column (the tenant-level table rule, D-253); if missing it
///   is an error instead of copying the whole table.
/// - The tenant's rows are read through a connection that always sees the latest data ([`fresh`]): SQLite
///   reads through the read pool (not holding the shared database's single write connection), Postgres
///   through the write pool (the read pool may be a lagging read replica).
#[derive(Debug, Clone)]
pub struct TenantTables {
    tables: Vec<TablePlan>,
}

/// A connection that sees every committed change of `database` (see [`TenantTables`]).
fn fresh(database: &Database) -> &sea_orm::DatabaseConnection {
    match database.backend() {
        crate::config::DatabaseBackend::Sqlite => database.read(),
        crate::config::DatabaseBackend::Postgres => database.write(),
    }
}

impl TenantTables {
    /// The tenant-level tables of `modules` (except `control_plane`) that exist at `target`, with their
    /// columns at `source` and `target`. A table missing at the source or a missing column: the two
    /// databases do not have the same schema yet.
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

    /// Table names, in creation order.
    pub fn names(&self) -> Vec<&str> {
        self.tables.iter().map(|table| table.name.as_str()).collect()
    }

    /// The kind of table `name` (a table not in the list: [`TableKind::Data`]).
    pub fn kind(&self, name: &str) -> TableKind {
        self.tables.iter().find(|table| table.name == name).map_or(TableKind::Data, TablePlan::kind)
    }

    /// The job queues and outboxes among the tables, with their kinds.
    pub fn queues(&self) -> Vec<(&str, TableKind)> {
        self.tables
            .iter()
            .map(|table| (table.name.as_str(), table.kind()))
            .filter(|(_, kind)| *kind != TableKind::Data)
            .collect()
    }

    /// The row count of tenant `tenant` in each table of `database`, in creation order.
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

    /// Deletes the rows of tenant `tenant` in every table whose name satisfies `only`, through `connection`
    /// (usually the caller's transaction): referencing tables before referenced tables (reverse creation
    /// order). Returns the number of rows deleted.
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

    /// Copies the rows of tenant `tenant` from `source` into `target` (the caller's transaction on the
    /// target database): first deletes the tenant's rows already at the target (an earlier copy that was
    /// interrupted, a copy kept from an earlier move), then copies each table in creation order. Returns
    /// the number of rows copied per table.
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

/// Columns left out of a tenant's data export (step 35, D-328): password / token hashes, secrets,
/// sealed data. They do not help the tenant when taking their data away, and only add risk if the export leaks.
pub fn is_secret_column(name: &str) -> bool {
    name.ends_with("_hash")
        || name.ends_with("_token")
        || name.contains("secret")
        || name.contains("sealed")
        || name == "password"
}

impl TenantTables {
    /// Exports every row of tenant `tenant` at `source` as JSON Lines (`{"table": …, "row": {…}}` per
    /// line), sending ~64 KiB chunks through `sink`. Leaves out job queues / outboxes and secret columns
    /// ([`is_secret_column`]). Bytes are written as base64, times as RFC 3339 UTC, dates as `YYYY-MM-DD`.
    /// Returns the row count per table.
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
                        // Thời điểm: chuỗi RFC 3339 `+00:00` như trước. Ngày: `YYYY-MM-DD`, như API trả
                        // (trước đây cột ngày ra `null` — mất dữ liệu khi khách mang đi).
                        ref value @ Value::Enum(_) => Timestamp::from_sql_value(value)
                            .map(|at| serde_json::Value::from(at.to_rfc3339()))
                            .or_else(|| {
                                Date::from_sql_value(value)
                                    .map(|day| serde_json::Value::from(day.to_string()))
                            }),
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

/// Whether `a` and `b` are the same database (two placements pointing at the same place): SQLite compares
/// files, Postgres compares the database name, schema and the server start time
/// (`pg_postmaster_start_time`, which differs between two servers even with the same database name). Moving
/// a tenant between two "databases" that are really one would delete the data in the source cleanup step,
/// so the caller must refuse.
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
