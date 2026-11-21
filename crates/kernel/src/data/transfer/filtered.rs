//! Chép **có lọc dòng và ghi đè cột** (bước 38, tách / gộp khách — `tenancy::relocate`): cùng cách
//! đọc theo kiểu cột của nguồn, ghi theo kiểu cột của đích như [`TenantTables::copy`], nhưng
//!
//! - chỉ dòng thoả một điều kiện SQL (vd "nút thuộc nhánh N", "dòng cha đi theo");
//! - mỗi dòng qua một hàm ghi đè trước khi ghi (đổi `tenant_id`, đổi nút cha của gốc nhánh, mã hoá
//!   lại secret cho khách mới…);
//! - tuỳ chọn bỏ qua dòng mà khoá đã có ở đích (`ON CONFLICT DO NOTHING`, chạy như nhau trên SQLite
//!   và Postgres) — gộp khách gặp lại đúng dòng đã chép khi tách.
//!
//! Kèm đọc cấu trúc bảng ở database (khoá chính, unique index, foreign key) để kế hoạch tìm xung
//! đột trước khi chép, không phải giữa lúc chép.

use crate::time::{Date, Precision, Timestamp};
use sea_orm::sea_query::{Condition, OnConflict};

use super::{
    BATCH_ROWS, Column, Kind, TENANT_COLUMN, TenantTables, TransferError, convert, fresh, read,
};
use crate::database::Database;
use futures_util::TryStreamExt as _;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query, SimpleExpr, Value};
use sea_orm::{ConnectionTrait, DatabaseBackend, StreamTrait};

/// Một dòng đọc từ nguồn: tên cột và giá trị, theo thứ tự cột của nguồn.
#[derive(Debug, Clone)]
pub(crate) struct Row(Vec<(String, Value)>);

impl Row {
    pub(crate) fn get(&self, column: &str) -> Option<&Value> {
        self.0.iter().find(|(name, _)| name == column).map(|(_, value)| value)
    }

    /// Đặt giá trị cột `column` (cột không có: bỏ qua — bảng không có cột đó).
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

/// Hàm ghi đè một dòng trước khi ghi vào đích.
pub(crate) type Rewrite<'a> = &'a (dyn Fn(&mut Row) -> Result<(), TransferError> + Sync);

/// Giá trị dạng chữ ổn định, để so khoá giữa hai database (`None` = `NULL`).
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

    /// Bảng `table` có trong danh sách không.
    pub(crate) fn contains(&self, table: &str) -> bool {
        self.tables.iter().any(|plan| plan.name == table)
    }

    /// Bảng `table` có cột `column` ở nguồn không.
    pub(crate) fn has_column(&self, table: &str, column: &str) -> bool {
        self.plan_of(table).is_ok_and(|plan| plan.from.iter().any(|found| found.name == column))
    }

    /// Cột `column` của `table` là UUID.
    pub(crate) fn is_uuid_column(&self, table: &str, column: &str) -> bool {
        self.plan_of(table).is_ok_and(|plan| {
            plan.from.iter().any(|found| found.name == column && found.kind == Kind::Uuid)
        })
    }

    /// Đếm dòng của `table` thoả `condition` ở `database` (đọc qua kết nối thấy dữ liệu mới nhất).
    pub(crate) async fn count_matching(
        &self,
        database: &Database,
        table: &str,
        condition: Condition,
    ) -> Result<u64, TransferError> {
        count_condition(fresh(database), table, condition).await
    }

    /// Xoá dòng của `table` thoả `condition`, qua `connection` (transaction của bên gọi).
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

    /// Giá trị (dạng chữ, [`canonical`]) của các cột `columns` ở mọi dòng của `table` thoả
    /// `condition`, đọc qua `connection`.
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

    /// Đọc qua `connection` mọi dòng của `table` thoả `condition`, đầy đủ cột của nguồn.
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

    /// Ghi `rows` (đọc bằng [`rows`](Self::rows) từ cùng bảng) vào `table` qua `target`;
    /// `skip_existing`: dòng mà khoá đã có ở đích bị bỏ qua. Trả số dòng đã ghi.
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

    /// Chép dòng của `table` thoả `condition` từ `source` vào `target` (transaction ở database
    /// đích của bên gọi), mỗi dòng qua `rewrite` trước khi ghi. `skip_existing`: dòng mà khoá
    /// (khoá chính hay unique) đã có ở đích bị bỏ qua, không lỗi. Trả số dòng đã ghi.
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

/// Giá trị của `row` theo kiểu cột ở đích.
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

/// Điều kiện `tenant_id = tenant` của bảng cấp khách hàng.
pub(crate) fn tenant_is(tenant: crate::tenancy::TenantId) -> Condition {
    Condition::all().add(Expr::col(Alias::new(TENANT_COLUMN)).eq(tenant.as_uuid()))
}

/// Một foreign key một cột: `table.column` trỏ tới `parent.parent_column`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForeignKey {
    pub(crate) column: String,
    pub(crate) parent: String,
    pub(crate) parent_column: String,
}

/// Cột khoá chính của `table`, theo thứ tự trong khoá.
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

/// Unique index (không phải khoá chính) của `table` chỉ gồm cột thường, không điều kiện: danh sách
/// cột của từng index. Index một phần hay theo biểu thức bị bỏ qua (kế hoạch không so được; lệnh
/// chép vẫn báo lỗi nếu đụng).
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

/// Foreign key một cột của `table`.
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
