//! Ảnh chụp schema của database: bảng, cột (tên, kiểu, nullable, mặc định), index (tên, cột,
//! unique) — để so schema trước và sau một vòng migration down → up (đặc tả 31 B2).
//!
//! Hai nhánh truy vấn: SQLite (`sqlite_master`, `pragma_table_info`, `pragma_index_list`,
//! `pragma_index_info`) và Postgres (`information_schema.columns`, `pg_index`). Sắp xếp ổn định để
//! hai ảnh chụp so được bằng `==` và `Debug` dễ đọc khi khác.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, QueryResult, Statement};

use crate::TestDatabaseError;

/// Ảnh chụp schema. Lấy bằng [`schema_snapshot`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct SchemaSnapshot {
    pub tables: Vec<TableSnapshot>,
}

/// Một bảng.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TableSnapshot {
    pub name: String,
    pub columns: Vec<ColumnSnapshot>,
    pub indexes: Vec<IndexSnapshot>,
}

/// Một cột.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ColumnSnapshot {
    pub name: String,
    /// Kiểu như database ghi (chữ thường).
    pub kind: String,
    pub nullable: bool,
    pub default: Option<String>,
}

/// Một index (kể cả index ngầm của khoá chính / unique).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct IndexSnapshot {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

impl SchemaSnapshot {
    /// Tên các bảng.
    pub fn table_names(&self) -> Vec<&str> {
        self.tables.iter().map(|table| table.name.as_str()).collect()
    }
}

/// Chụp schema của `connection` (SQLite hay Postgres).
pub async fn schema_snapshot(
    connection: &DatabaseConnection,
) -> Result<SchemaSnapshot, TestDatabaseError> {
    let backend = connection.get_database_backend();
    let tables = match backend {
        DbBackend::Sqlite => {
            names(
                connection,
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
                 ORDER BY name",
            )
            .await?
        }
        DbBackend::Postgres => {
            names(
                connection,
                "SELECT table_name::text FROM information_schema.tables \
                 WHERE table_schema = current_schema() AND table_type = 'BASE TABLE' ORDER BY 1",
            )
            .await?
        }
        other => {
            return Err(TestDatabaseError(format!("schema snapshot does not support {other:?}")));
        }
    };
    let mut snapshot = SchemaSnapshot::default();
    for name in tables {
        let columns = columns(connection, backend, &name).await?;
        let mut indexes = indexes(connection, backend, &name).await?;
        indexes.sort_by(|a, b| a.name.cmp(&b.name));
        snapshot.tables.push(TableSnapshot { name, columns, indexes });
    }
    Ok(snapshot)
}

async fn rows(connection: &DatabaseConnection, sql: String) -> Result<Vec<QueryResult>, DbErr> {
    connection.query_all_raw(Statement::from_string(connection.get_database_backend(), sql)).await
}

async fn names(connection: &DatabaseConnection, sql: &str) -> Result<Vec<String>, DbErr> {
    rows(connection, sql.to_owned())
        .await?
        .iter()
        .map(|row| row.try_get_by_index::<String>(0))
        .collect()
}

/// Tên bảng chỉ gồm chữ, số, `_` (bảng của module luôn vậy): đặt thẳng vào câu SQL an toàn.
fn checked(table: &str) -> Result<&str, TestDatabaseError> {
    if table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(table)
    } else {
        Err(TestDatabaseError(format!("unexpected table name `{table}`")))
    }
}

async fn columns(
    connection: &DatabaseConnection,
    backend: DbBackend,
    table: &str,
) -> Result<Vec<ColumnSnapshot>, TestDatabaseError> {
    let table = checked(table)?;
    let sql = match backend {
        DbBackend::Sqlite => format!(
            "SELECT name, lower(type), CASE WHEN \"notnull\" = 0 AND pk = 0 THEN 1 ELSE 0 END, \
             dflt_value FROM pragma_table_info('{table}') ORDER BY cid"
        ),
        _ => format!(
            "SELECT column_name::text, lower(data_type::text), \
             CASE WHEN is_nullable = 'YES' THEN 1 ELSE 0 END, column_default::text \
             FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = '{table}' \
             ORDER BY ordinal_position"
        ),
    };
    let found = rows(connection, sql).await?;
    found
        .iter()
        .map(|row| {
            Ok(ColumnSnapshot {
                name: row.try_get_by_index::<String>(0)?,
                kind: row.try_get_by_index::<String>(1)?,
                nullable: row.try_get_by_index::<i32>(2)? == 1,
                default: row.try_get_by_index::<Option<String>>(3)?,
            })
        })
        .collect::<Result<_, DbErr>>()
        .map_err(Into::into)
}

async fn indexes(
    connection: &DatabaseConnection,
    backend: DbBackend,
    table: &str,
) -> Result<Vec<IndexSnapshot>, TestDatabaseError> {
    let table = checked(table)?;
    let sql = match backend {
        DbBackend::Sqlite => format!(
            "SELECT l.name, l.\"unique\", i.name FROM pragma_index_list('{table}') l \
             JOIN pragma_index_info(l.name) i ORDER BY l.name, i.seqno"
        ),
        _ => format!(
            "SELECT ic.relname::text, CASE WHEN ix.indisunique THEN 1 ELSE 0 END, a.attname::text \
             FROM pg_index ix \
             JOIN pg_class ic ON ic.oid = ix.indexrelid \
             JOIN pg_class tc ON tc.oid = ix.indrelid \
             JOIN pg_namespace n ON n.oid = tc.relnamespace \
             JOIN LATERAL unnest(ix.indkey) WITH ORDINALITY AS k(attnum, position) ON true \
             JOIN pg_attribute a ON a.attrelid = tc.oid AND a.attnum = k.attnum \
             WHERE n.nspname = current_schema() AND tc.relname = '{table}' \
             ORDER BY ic.relname, k.position"
        ),
    };
    let found = rows(connection, sql).await?;
    let mut indexes: Vec<IndexSnapshot> = Vec::new();
    for row in &found {
        let name = row.try_get_by_index::<String>(0)?;
        let unique = row.try_get_by_index::<i32>(1)? == 1;
        let column = row.try_get_by_index::<String>(2)?;
        match indexes.last_mut() {
            Some(index) if index.name == name => index.columns.push(column),
            _ => indexes.push(IndexSnapshot { name, columns: vec![column], unique }),
        }
    }
    Ok(indexes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestDatabase;

    #[tokio::test]
    async fn schema_snapshot_doc_bang_cot_index_tren_ca_hai_database() {
        TestDatabase::run(async |test| {
            let connection = sea_orm::Database::connect(test.url()).await.unwrap();
            for sql in [
                "CREATE TABLE demo_items (id varchar(36) PRIMARY KEY, name varchar(50) NOT NULL, \
                 note text, rank integer DEFAULT 0)",
                "CREATE UNIQUE INDEX demo_items_name_unique ON demo_items (name, rank)",
                "CREATE INDEX demo_items_note_idx ON demo_items (note)",
            ] {
                connection
                    .execute_raw(Statement::from_string(connection.get_database_backend(), sql))
                    .await
                    .unwrap();
            }
            let snapshot = schema_snapshot(&connection).await.unwrap();
            assert_eq!(snapshot.table_names(), ["demo_items"]);
            let table = &snapshot.tables[0];
            let columns: Vec<(&str, bool)> = table
                .columns
                .iter()
                .map(|column| (column.name.as_str(), column.nullable))
                .collect();
            assert_eq!(columns, [("id", false), ("name", false), ("note", true), ("rank", true)]);
            assert!(table.columns[3].default.as_deref().is_some_and(|value| value.contains('0')));
            let unique =
                table.indexes.iter().find(|index| index.name == "demo_items_name_unique").unwrap();
            assert_eq!(
                (unique.columns.clone(), unique.unique),
                (vec!["name".to_owned(), "rank".to_owned()], true)
            );
            let plain =
                table.indexes.iter().find(|index| index.name == "demo_items_note_idx").unwrap();
            assert!(!plain.unique);
            // Chụp lại: y hệt.
            assert_eq!(schema_snapshot(&connection).await.unwrap(), snapshot);
            connection.close().await.unwrap();
        })
        .await
        .unwrap();
    }
}
