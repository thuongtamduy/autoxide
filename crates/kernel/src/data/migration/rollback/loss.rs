// Đo dữ liệu một lần lùi sẽ mất (chạy thử của `Migrator::rollback_plan`), không giữ lock lâu trên database đang chạy
// (kiểm độc lập #586):
//
// - SQLite: chép database ra file tạm bằng `VACUUM INTO` (một lần đọc), chạy các `down` trên bản sao, xoá bản sao.
//   Database thật không bị khoá ghi.
// - Postgres: chạy các `down` trong một transaction rồi huỷ, với `SET LOCAL lock_timeout` / `statement_timeout`:
//   bảng đang bận thì dừng sau `lock_timeout` và báo rõ, không đứng chờ chặn các request khác. Trong lúc chạy,
//   lệnh DDL giữ ACCESS EXCLUSIVE trên bảng nó đổi tới khi huỷ transaction.
//
// Chụp shape (bảng → cột → mặc định) một lần trước và một lần sau. Mất: bảng biến mất (đếm dòng), cột biến mất (đếm
// giá trị khác NULL và khác mặc định của cột — cột chỉ toàn giá trị mặc định thì lùi không mất gì), và bảng còn đó
// mà ít dòng hơn (`DELETE` hay bỏ rồi tạo lại trong `down`). Bảng bị bỏ thì không tính thêm cột của nó.

use std::collections::BTreeMap;
use std::time::Duration;

use sea_orm::{ConnectionTrait, DbBackend, DbErr, Statement, TransactionTrait};

use super::super::manager::{self, SchemaManager};
use super::super::{Entry, ModuleMigrations};
use super::LostData;
use crate::database::Database;

/// How long the dry run of a rollback may wait on Postgres (it runs the schema `down`s inside a transaction that
/// is rolled back, on the live database).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DryRunLimits {
    /// Longest wait for a lock on a table another session uses (`lock_timeout`).
    pub lock_timeout: Duration,
    /// Longest run of one statement (`statement_timeout`).
    pub statement_timeout: Duration,
}

impl Default for DryRunLimits {
    fn default() -> Self {
        DryRunLimits {
            lock_timeout: Duration::from_secs(2),
            statement_timeout: Duration::from_secs(60),
        }
    }
}

// Bảng → cột → biểu thức mặc định (như catalog ghi, dùng lại nguyên văn trong SQL của chính database đó).
type Shape = BTreeMap<String, BTreeMap<String, Option<String>>>;

pub(super) async fn measure(
    database: &Database,
    steps: &[(&ModuleMigrations, Entry)],
    limits: DryRunLimits,
) -> Result<Vec<LostData>, DbErr> {
    let modules: Vec<&str> = steps.iter().map(|(set, _)| set.module).collect();
    let backend = database.write().get_database_backend();
    let (before, after, shrunk) = match backend {
        DbBackend::Sqlite => on_sqlite_copy(database, steps, &modules).await?,
        _ => {
            let transaction = database.write().begin().await?;
            let timeouts = format!(
                "SET LOCAL lock_timeout = '{}ms'; SET LOCAL statement_timeout = '{}ms'",
                limits.lock_timeout.as_millis(),
                limits.statement_timeout.as_millis()
            );
            transaction.execute_unprepared(&timeouts).await?;
            let outcome = run_downs(&transaction, steps, &modules).await;
            transaction.rollback().await?;
            outcome.map_err(explain_timeout)?
        }
    };
    // Đếm trên dữ liệu thật: bản sao / transaction đã bỏ, database còn nguyên.
    let mut lost = Vec::new();
    for (table, columns) in &before {
        match after.get(table) {
            None => {
                let rows = count(database.read(), backend, table, None).await?;
                lost.push(LostData { table: table.clone(), column: None, rows, dropped: true });
            }
            Some(remaining) => {
                for (column, default) in columns {
                    if remaining.contains_key(column) {
                        continue;
                    }
                    let rows =
                        count(database.read(), backend, table, Some((column, default.as_deref())))
                            .await?;
                    if rows > 0 {
                        lost.push(LostData {
                            table: table.clone(),
                            column: Some(column.clone()),
                            rows,
                            dropped: false,
                        });
                    }
                }
            }
        }
    }
    for (table, rows) in shrunk {
        if after.contains_key(&table) {
            lost.push(LostData { table, column: None, rows, dropped: false });
        }
    }
    Ok(lost)
}

type Measured = (Shape, Shape, Vec<(String, u64)>);

async fn on_sqlite_copy(
    database: &Database,
    steps: &[(&ModuleMigrations, Entry)],
    modules: &[&str],
) -> Result<Measured, DbErr> {
    let path =
        std::env::temp_dir().join(format!("autoxide-rollback-{}.sqlite", uuid::Uuid::now_v7()));
    let quoted = path.display().to_string().replace('\'', "''");
    let copy = format!("VACUUM INTO '{quoted}'");
    database.write().execute_unprepared(&copy).await?;
    let outcome = async {
        let url = format!("sqlite://{}?mode=rw", path.display());
        let connection = sea_orm::Database::connect(url).await?;
        let transaction = connection.begin().await?;
        let measured = run_downs(&transaction, steps, modules).await;
        transaction.rollback().await?;
        connection.close().await?;
        measured
    }
    .await;
    // Bản sao có dữ liệu của khách: xoá dù đo được hay không.
    let _ = std::fs::remove_file(&path);
    outcome
}

// Chạy các `down` schema trong `connection` (transaction sẽ huỷ, trên database thật hay bản sao): shape trước, shape sau, và số dòng mất
// của bảng thuộc các module bị lùi mà còn tồn tại sau đó.
async fn run_downs(
    connection: &sea_orm::DatabaseTransaction,
    steps: &[(&ModuleMigrations, Entry)],
    modules: &[&str],
) -> Result<Measured, DbErr> {
    let backend = connection.get_database_backend();
    let before = shape(connection).await?;
    let owned: Vec<String> = before
        .keys()
        // Bảng lịch sử `<module>_migrations` luôn mất dòng của migration bị gỡ: đó không phải dữ liệu.
        .filter(|table| {
            modules.iter().any(|module| {
                table.starts_with(&format!("{module}_"))
                    && **table != format!("{module}_migrations")
            })
        })
        .cloned()
        .collect();
    let mut counts = Vec::new();
    for table in &owned {
        counts.push((table.clone(), count(connection, backend, table, None).await?));
    }
    for (set, entry) in steps {
        let Entry::Schema(migration) = entry else {
            continue;
        };
        let nested = connection.begin().await?;
        migration.down(&SchemaManager::new(&nested)).await?;
        manager::forget(&nested, &set.history_table(), migration.name()).await?;
        nested.commit().await?;
    }
    let after = shape(connection).await?;
    let mut shrunk = Vec::new();
    for (table, rows) in counts {
        if after.contains_key(&table) {
            let left = count(connection, backend, &table, None).await?;
            if left < rows {
                shrunk.push((table, rows - left));
            }
        }
    }
    Ok((before, after, shrunk))
}

fn explain_timeout(error: DbErr) -> DbErr {
    let text = error.to_string();
    if text.contains("lock timeout") || text.contains("statement timeout") {
        DbErr::Custom(format!(
            "{text}: the dry run waited too long for a table the servers are using; run it when the load is low, \
             after stopping the servers, or with a longer `--lock-timeout-seconds`"
        ))
    } else {
        error
    }
}

/// Tables, their columns and the default of each column.
async fn shape<C: ConnectionTrait>(connection: &C) -> Result<Shape, DbErr> {
    let backend = connection.get_database_backend();
    let sql = match backend {
        DbBackend::Sqlite => {
            "SELECT m.name, p.name, p.dflt_value FROM sqlite_master m, pragma_table_info(m.name) p \
             WHERE m.type = 'table' AND m.name NOT LIKE 'sqlite_%'"
        }
        _ => {
            "SELECT table_name::text, column_name::text, column_default::text FROM information_schema.columns \
             WHERE table_schema = current_schema()"
        }
    };
    let rows = connection.query_all_raw(Statement::from_string(backend, sql)).await?;
    let mut shape = Shape::new();
    for row in rows {
        let table: String = row.try_get_by_index(0)?;
        let column: String = row.try_get_by_index(1)?;
        let default: Option<String> = row.try_get_by_index(2)?;
        shape.entry(table).or_default().insert(column, default);
    }
    Ok(shape)
}

/// Rows of `table`; with a column, its values that are neither NULL nor the column's default. Names come from the
/// database catalog and are checked to be plain identifiers; the default is the catalog's own expression for this
/// database.
async fn count<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    table: &str,
    column: Option<(&str, Option<&str>)>,
) -> Result<u64, DbErr> {
    let plain = |name: &str| name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !plain(table) || !column.is_none_or(|(name, _)| plain(name)) {
        return Err(DbErr::Custom(format!("unexpected table or column name `{table}`")));
    }
    let sql = match column {
        None => format!("SELECT COUNT(*) FROM \"{table}\""),
        Some((column, None)) => format!("SELECT COUNT(\"{column}\") FROM \"{table}\""),
        Some((column, Some(default))) => {
            format!(
                "SELECT COUNT(*) FROM \"{table}\" WHERE \"{column}\" IS NOT NULL AND \"{column}\" <> ({default})"
            )
        }
    };
    let row = connection.query_one_raw(Statement::from_string(backend, sql)).await?;
    let rows: i64 = match row {
        Some(row) => row.try_get_by_index(0)?,
        None => 0,
    };
    Ok(u64::try_from(rows).unwrap_or(0))
}
