//! Copy **with row filtering and column overrides** (step 38, split / merge of tenants,
//! `tenancy::relocate`): the same way of reading by the source's column type and writing by the target's
//! column type as [`TenantTables::copy`], but
//!
//! - only the rows that satisfy an SQL condition (e.g. "node belongs to branch N", "the parent row goes
//!   along");
//! - each row goes through an override function before being written (change `tenant_id`, change the parent
//!   node of the branch root, re-encrypt secrets for the new tenant...);
//! - optionally skip rows whose key already exists at the target (`ON CONFLICT DO NOTHING`, behaving the
//!   same on SQLite and Postgres): a merge meets the very rows already copied by a split.
//!
//! Also reads the table structure in the database (primary key, unique indexes, foreign keys) so the plan
//! finds conflicts before copying, not in the middle of it.

use crate::time::{Date, Precision, Timestamp};
use sea_orm::sea_query::{Condition, OnConflict};

use super::{
    BATCH_ROWS, Column, Kind, TENANT_COLUMN, TenantTables, TransferError, convert, fresh, read,
};
use crate::database::Database;
use futures_util::TryStreamExt as _;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query, SimpleExpr, Value};
use sea_orm::{ConnectionTrait, DatabaseBackend, StreamTrait};

/// A row read from the source: column names and values, in the source's column order.
#[derive(Debug, Clone)]
pub(crate) struct Row(Vec<(String, Value)>);

impl Row {
    pub(crate) fn get(&self, column: &str) -> Option<&Value> {
        self.0.iter().find(|(name, _)| name == column).map(|(_, value)| value)
    }

    /// Sets the value of column `column` (a column that does not exist: ignored, the table has no such column).
    pub(crate) fn set(&mut self, column: &str, value: Value) {
        if let Some(slot) = self.0.iter_mut().find(|(name, _)| name == column) {
            slot.1 = value;
        }
    }

    pub(crate) fn uuid(&self, column: &str) -> Option<uuid::Uuid> {
        match self.get(column) {
            Some(Value::Uuid(Some(id))) => Some(*id),
            Some(Value::String(Some(text))) => text.parse().ok(),
            _ => None,
        }
    }

    pub(crate) fn text(&self, column: &str) -> Option<&str> {
        match self.get(column) {
            Some(Value::String(Some(text))) => Some(text.as_str()),
            _ => None,
        }
    }
}

/// A function that overrides a row before it is written to the target.
pub(crate) type Rewrite<'a> = &'a (dyn Fn(&mut Row) -> Result<(), TransferError> + Sync);

/// A stable text form of a value, to compare keys between two databases (`None` = `NULL`).
pub(crate) fn canonical(value: &Value) -> Option<String> {
    match value {
        Value::Uuid(value) => value.map(|id| id.to_string()),
        Value::Enum(_) => Timestamp::from_sql_value(value)
            .map(|at| at.to_rfc3339_opts(Precision::Micros, true))
            .or_else(|| Date::from_sql_value(value).map(|day| day.to_string())),
        Value::String(value) => value.clone(),
        Value::SmallInt(value) => value.map(|number| number.to_string()),
        Value::Int(value) => value.map(|number| number.to_string()),
        Value::BigInt(value) => value.map(|number| number.to_string()),
        Value::Bool(value) => value.map(|flag| flag.to_string()),
        Value::Double(value) => value.map(|number| number.to_string()),
        Value::Bytes(value) => value.as_ref().map(|bytes| format!("{bytes:?}")),
        _ => None,
    }
}

impl TenantTables {
    fn plan_of(&self, table: &str) -> Result<&super::TablePlan, TransferError> {
        self.tables.iter().find(|plan| plan.name == table).ok_or_else(|| {
            TransferError(format!("`{table}` is not a tenant-level table of the installed modules"))
        })
    }

    /// Whether table `table` is in the list.
    pub(crate) fn contains(&self, table: &str) -> bool {
        self.tables.iter().any(|plan| plan.name == table)
    }

    /// Whether table `table` has column `column` at the source.
    pub(crate) fn has_column(&self, table: &str, column: &str) -> bool {
        self.plan_of(table).is_ok_and(|plan| plan.from.iter().any(|found| found.name == column))
    }

    /// Whether column `column` of `table` is a UUID.
    pub(crate) fn is_uuid_column(&self, table: &str, column: &str) -> bool {
        self.plan_of(table).is_ok_and(|plan| {
            plan.from.iter().any(|found| found.name == column && found.kind == Kind::Uuid)
        })
    }

    /// Counts the rows of `table` that satisfy `condition` at `database` (read through a connection that sees
    /// the latest data).
    pub(crate) async fn count_matching(
        &self,
        database: &Database,
        table: &str,
        condition: Condition,
    ) -> Result<u64, TransferError> {
        count_condition(fresh(database), table, condition).await
    }

    /// Deletes the rows of `table` that satisfy `condition`, through `connection` (the caller's transaction).
    pub(crate) async fn delete_matching<C: ConnectionTrait>(
        &self,
        connection: &C,
        table: &str,
        condition: Condition,
    ) -> Result<u64, TransferError> {
        let mut delete = Query::delete();
        delete.from_table(Alias::new(table)).cond_where(condition);
        Ok(connection.execute(&delete).await?.rows_affected())
    }

    /// The values (in text form, [`canonical`]) of the columns `columns` in every row of `table` that satisfies
    /// `condition`, read through `connection`.
    pub(crate) async fn values<C: ConnectionTrait>(
        &self,
        connection: &C,
        table: &str,
        columns: &[String],
        condition: Condition,
    ) -> Result<Vec<Vec<Option<String>>>, TransferError> {
        let plan = self.plan_of(table)?;
        let mut wanted = Vec::with_capacity(columns.len());
        for name in columns {
            let column =
                plan.from.iter().find(|column| &column.name == name).ok_or_else(|| {
                    TransferError(format!("column `{table}.{name}` does not exist"))
                })?;
            wanted.push(column.clone());
        }
        let mut select = Query::select();
        select.from(Alias::new(table)).cond_where(condition);
        for column in &wanted {
            select.column(Alias::new(&column.name));
        }
        if wanted.is_empty() {
            // Khoá chỉ gồm `tenant_id`: mỗi dòng một khoá rỗng.
            select.expr(Expr::cust("1"));
        }
        let backend = connection.get_database_backend();
        let rows = connection.query_all_raw(backend.build(&select)).await?;
        let mut found = Vec::with_capacity(rows.len());
        for row in rows {
            let mut values = Vec::with_capacity(wanted.len());
            for column in &wanted {
                values.push(canonical(&read(&row, column)?));
            }
            found.push(values);
        }
        Ok(found)
    }

    /// Reads through `connection` every row of `table` that satisfies `condition`, with all the source's
    /// columns.
    pub(crate) async fn rows<C: ConnectionTrait>(
        &self,
        connection: &C,
        table: &str,
        condition: Condition,
    ) -> Result<Vec<Row>, TransferError> {
        let plan = self.plan_of(table)?;
        let mut select = Query::select();
        select.from(Alias::new(table)).cond_where(condition);
        for column in &plan.from {
            select.column(Alias::new(&column.name));
        }
        let backend = connection.get_database_backend();
        let rows = connection.query_all_raw(backend.build(&select)).await?;
        let mut found = Vec::with_capacity(rows.len());
        for row in rows {
            let mut values = Vec::with_capacity(plan.from.len());
            for column in &plan.from {
                values.push((column.name.clone(), read(&row, column)?));
            }
            found.push(Row(values));
        }
        Ok(found)
    }

    /// Writes `rows` (read with [`rows`](Self::rows) from the same table) into `table` through `target`;
    /// `skip_existing`: a row whose key already exists at the target is skipped. Returns the number of rows
    /// written.
    pub(crate) async fn insert_rows<C: ConnectionTrait>(
        &self,
        target: &C,
        table: &str,
        rows: Vec<Row>,
        skip_existing: bool,
    ) -> Result<u64, TransferError> {
        let plan = self.plan_of(table)?;
        let mut written = 0;
        let mut batch = Vec::with_capacity(BATCH_ROWS);
        for row in rows {
            batch.push(target_values(plan, row, table)?);
            if batch.len() == BATCH_ROWS {
                written += insert_batch(
                    target,
                    table,
                    &plan.from,
                    std::mem::take(&mut batch),
                    skip_existing,
                )
                .await?;
            }
        }
        if !batch.is_empty() {
            written += insert_batch(target, table, &plan.from, batch, skip_existing).await?;
        }
        Ok(written)
    }

    /// Copies the rows of `table` that satisfy `condition` from `source` into `target` (the caller's
    /// transaction on the target database), each row going through `rewrite` before being written.
    /// `skip_existing`: a row whose key (primary or unique) already exists at the target is skipped,
    /// without an error. Returns the number of rows written.
    pub(crate) async fn copy_where<C: ConnectionTrait>(
        &self,
        source: &Database,
        target: &C,
        table: &str,
        condition: Condition,
        rewrite: Rewrite<'_>,
        skip_existing: bool,
    ) -> Result<u64, TransferError> {
        let plan = self.plan_of(table)?;
        let connection = fresh(source);
        let mut select = Query::select();
        select.from(Alias::new(table)).cond_where(condition);
        for column in &plan.from {
            select.column(Alias::new(&column.name));
        }
        let backend = ConnectionTrait::get_database_backend(connection);
        let rows = connection.stream_raw(backend.build(&select)).await?;
        // `pin!`: luồng của database có thể tự trỏ vào mình; ghim nó tại chỗ để đọc dần.
        let mut rows = std::pin::pin!(rows);
        let mut batch = Vec::with_capacity(BATCH_ROWS);
        let mut written = 0;
        while let Some(found) = rows.try_next().await? {
            let mut values = Vec::with_capacity(plan.from.len());
            for column in &plan.from {
                values.push((column.name.clone(), read(&found, column)?));
            }
            let mut row = Row(values);
            rewrite(&mut row)?;
            batch.push(target_values(plan, row, table)?);
            if batch.len() == BATCH_ROWS {
                written += insert_batch(
                    target,
                    table,
                    &plan.from,
                    std::mem::take(&mut batch),
                    skip_existing,
                )
                .await?;
            }
        }
        if !batch.is_empty() {
            written += insert_batch(target, table, &plan.from, batch, skip_existing).await?;
        }
        Ok(written)
    }
}

/// The value of `row` by the column type at the target.
fn target_values(
    plan: &super::TablePlan,
    row: Row,
    table: &str,
) -> Result<Vec<SimpleExpr>, TransferError> {
    let mut values = Vec::with_capacity(plan.from.len());
    for (name, value) in row.0 {
        let kind = plan
            .to
            .iter()
            .find(|column| column.name == name)
            .map(|column| column.kind)
            .ok_or_else(|| {
                TransferError(format!(
                    "column `{table}.{name}` is missing in the target: apply the same migrations \
                     to both"
                ))
            })?;
        values.push(SimpleExpr::Value(convert(value, kind, table, &name)?));
    }
    Ok(values)
}

async fn insert_batch<C: ConnectionTrait>(
    target: &C,
    table: &str,
    columns: &[Column],
    rows: Vec<Vec<SimpleExpr>>,
    skip_existing: bool,
) -> Result<u64, TransferError> {
    let mut insert = Query::insert();
    insert.into_table(Alias::new(table));
    insert.columns(columns.iter().map(|column| Alias::new(&column.name)));
    for row in rows {
        insert.values(row).map_err(|error| TransferError(error.to_string()))?;
    }
    if skip_existing {
        insert.on_conflict(OnConflict::new().do_nothing().to_owned());
    }
    Ok(target.execute(&insert).await?.rows_affected())
}

async fn count_condition<C: ConnectionTrait>(
    connection: &C,
    table: &str,
    condition: Condition,
) -> Result<u64, TransferError> {
    let mut select = Query::select();
    select
        .expr_as(Expr::cust("COUNT(*)"), Alias::new("rows"))
        .from(Alias::new(table))
        .cond_where(condition);
    let backend = connection.get_database_backend();
    let row = connection.query_one_raw(backend.build(&select)).await?;
    let rows: i64 = match row {
        Some(row) => row.try_get("", "rows")?,
        None => 0,
    };
    u64::try_from(rows).map_err(|error| TransferError(error.to_string()))
}

/// The condition `tenant_id = tenant` of a tenant-level table.
pub(crate) fn tenant_is(tenant: crate::tenancy::TenantId) -> Condition {
    Condition::all().add(Expr::col(Alias::new(TENANT_COLUMN)).eq(tenant.as_uuid()))
}

/// A single-column foreign key: `table.column` points at `parent.parent_column`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForeignKey {
    pub(crate) column: String,
    pub(crate) parent: String,
    pub(crate) parent_column: String,
}

/// The primary key columns of `table`, in key order.
pub(crate) async fn primary_key<C: ConnectionTrait>(
    connection: &C,
    table: &str,
) -> Result<Vec<String>, TransferError> {
    let backend = connection.get_database_backend();
    let statement = match backend {
        DatabaseBackend::Sqlite => crate::database::statement(
            backend,
            "SELECT name FROM pragma_table_info(?) WHERE pk > 0 ORDER BY pk",
            [table.into()],
        ),
        DatabaseBackend::Postgres => crate::database::statement(
            backend,
            "SELECT a.attname::text AS name FROM pg_index i JOIN pg_attribute a \
             ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey) \
             WHERE i.indrelid = to_regclass($1) AND i.indisprimary \
             ORDER BY array_position(i.indkey::int2[], a.attnum)",
            [table.into()],
        ),
        other => return Err(TransferError(format!("unsupported database {other:?}"))),
    };
    let rows = connection.query_all_raw(statement).await?;
    rows.iter().map(|row| row.try_get::<String>("", "name").map_err(Into::into)).collect()
}

/// The unique indexes (not the primary key) of `table` that consist only of plain columns, with no
/// condition: the list of columns of each index. Partial or expression indexes are skipped (the plan cannot
/// compare them; the copy command still reports an error if it hits one).
pub(crate) async fn unique_indexes<C: ConnectionTrait>(
    connection: &C,
    table: &str,
) -> Result<Vec<Vec<String>>, TransferError> {
    let backend = connection.get_database_backend();
    match backend {
        DatabaseBackend::Sqlite => {
            let list = connection
                .query_all_raw(crate::database::statement(
                    backend,
                    "SELECT name, origin, partial FROM pragma_index_list(?) WHERE \"unique\" = 1",
                    [table.into()],
                ))
                .await?;
            let mut indexes = Vec::new();
            for index in list {
                let name: String = index.try_get("", "name")?;
                let origin: String = index.try_get("", "origin")?;
                let partial: i64 = index.try_get("", "partial")?;
                if origin == "pk" || partial != 0 {
                    continue;
                }
                let columns = connection
                    .query_all_raw(crate::database::statement(
                        backend,
                        "SELECT name FROM pragma_index_info(?) ORDER BY seqno",
                        [name.into()],
                    ))
                    .await?;
                let mut names = Vec::new();
                for column in columns {
                    match column.try_get::<Option<String>>("", "name")? {
                        Some(name) => names.push(name),
                        // Cột là biểu thức.
                        None => names.clear(),
                    }
                }
                if !names.is_empty() {
                    indexes.push(names);
                }
            }
            Ok(indexes)
        }
        DatabaseBackend::Postgres => {
            let rows = connection
                .query_all_raw(crate::database::statement(
                    backend,
                    "SELECT c.relname::text AS index_name, a.attname::text AS name \
                     FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid \
                     JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey) \
                     WHERE i.indrelid = to_regclass($1) AND i.indisunique AND NOT i.indisprimary \
                     AND i.indpred IS NULL AND i.indexprs IS NULL \
                     ORDER BY c.relname, array_position(i.indkey::int2[], a.attnum)",
                    [table.into()],
                ))
                .await?;
            let mut indexes: Vec<(String, Vec<String>)> = Vec::new();
            for row in rows {
                let index: String = row.try_get("", "index_name")?;
                let name: String = row.try_get("", "name")?;
                match indexes.last_mut() {
                    Some((last, columns)) if *last == index => columns.push(name),
                    _ => indexes.push((index, vec![name])),
                }
            }
            Ok(indexes.into_iter().map(|(_, columns)| columns).collect())
        }
        other => Err(TransferError(format!("unsupported database {other:?}"))),
    }
}

/// A single-column foreign key of `table`.
pub(crate) async fn foreign_keys<C: ConnectionTrait>(
    connection: &C,
    table: &str,
) -> Result<Vec<ForeignKey>, TransferError> {
    let backend = connection.get_database_backend();
    let statement = match backend {
        DatabaseBackend::Sqlite => crate::database::statement(
            backend,
            "SELECT id, \"from\" AS col, \"table\" AS parent, \"to\" AS parent_col, \
             (SELECT COUNT(*) FROM pragma_foreign_key_list(?1) f2 WHERE f2.id = f.id) AS width \
             FROM pragma_foreign_key_list(?1) f",
            [table.into()],
        ),
        DatabaseBackend::Postgres => crate::database::statement(
            backend,
            "SELECT 0 AS id, a.attname::text AS col, cf.relname::text AS parent, \
             af.attname::text AS parent_col, array_length(con.conkey, 1)::bigint AS width \
             FROM pg_constraint con \
             JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = con.conkey[1] \
             JOIN pg_class cf ON cf.oid = con.confrelid \
             JOIN pg_attribute af ON af.attrelid = con.confrelid AND af.attnum = con.confkey[1] \
             WHERE con.contype = 'f' AND con.conrelid = to_regclass($1)",
            [table.into()],
        ),
        other => return Err(TransferError(format!("unsupported database {other:?}"))),
    };
    let rows = connection.query_all_raw(statement).await?;
    let mut keys = Vec::new();
    for row in rows {
        let width: i64 = row.try_get("", "width")?;
        if width != 1 {
            continue;
        }
        let parent: String = row.try_get("", "parent")?;
        // SQLite để trống cột đích khi foreign key trỏ khoá chính của bảng cha.
        let parent_column = match row.try_get::<Option<String>>("", "parent_col")? {
            Some(column) => column,
            None => primary_key(connection, &parent).await?.into_iter().next().unwrap_or_default(),
        };
        keys.push(ForeignKey { column: row.try_get("", "col")?, parent, parent_column });
    }
    Ok(keys)
}
