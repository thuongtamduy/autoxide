// Đo dữ liệu một lần lùi sẽ mất (chạy thử của `Migrator::rollback_plan`), không giữ lock lâu trên database đang chạy
// (kiểm độc lập #586):
//
// - SQLite: chép database bằng `VACUUM INTO` vào một thư mục riêng (0700) cạnh file database, chạy các `down` trên
//   bản sao (0600), xoá cả thư mục (guard `Drop`). Database thật không bị khoá ghi. Lượt chạy thật (máy chủ đã dừng)
//   đo thẳng trên database trong transaction rồi huỷ, không chép.
// - Postgres: chạy các `down` trong một transaction REPEATABLE READ rồi huỷ, với `SET LOCAL lock_timeout` /
//   `statement_timeout`:
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

/// How the dry run of a rollback measures. On Postgres it runs the schema `down`s inside a transaction that is
/// rolled back, on the live database, bounded by the two timeouts. On SQLite it works on a private copy of the
/// database, unless the servers are stopped ([`servers_stopped`](Self::servers_stopped)): then directly inside a
/// rolled-back transaction, without a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DryRunLimits {
    /// Longest wait for a lock on a table another session uses (`lock_timeout`). Never zero: on Postgres zero
    /// means waiting forever.
    pub lock_timeout: Duration,
    /// Longest run of one statement (`statement_timeout`).
    pub statement_timeout: Duration,
    /// No server uses the database (the real run of a rollback, after `--yes`).
    pub servers_stopped: bool,
}

impl DryRunLimits {
    /// Limits with these two timeouts, servers still running.
    pub fn new(lock_timeout: Duration, statement_timeout: Duration) -> Self {
        DryRunLimits { lock_timeout, statement_timeout, servers_stopped: false }
    }

    /// The same limits once every server is stopped.
    #[must_use]
    pub fn with_servers_stopped(mut self) -> Self {
        self.servers_stopped = true;
        self
    }
}

impl Default for DryRunLimits {
    fn default() -> Self {
        DryRunLimits::new(Duration::from_secs(2), Duration::from_secs(60))
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
        DbBackend::Sqlite if limits.servers_stopped => {
            // Máy chủ đã dừng: đo thẳng trong transaction rồi huỷ, không chép lại database.
            let transaction = database.write().begin().await?;
            let outcome = run_downs(&transaction, steps, &modules).await;
            transaction.rollback().await?;
            outcome?
        }
        DbBackend::Sqlite => on_sqlite_copy(database, steps, &modules).await?,
        _ => {
            // REPEATABLE READ: đếm trước và sau trên cùng một ảnh dữ liệu, dữ liệu đang được ghi không làm sai phép đo.
            let transaction = database
                .write()
                .begin_with_config(Some(sea_orm::IsolationLevel::RepeatableRead), None)
                .await?;
            let timeouts = format!(
                "SET LOCAL lock_timeout = '{}ms'; SET LOCAL statement_timeout = '{}ms'",
                limits.lock_timeout.as_millis().max(1),
                limits.statement_timeout.as_millis().max(1)
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
    let copy = PrivateCopy::create(database).await?;
    let quoted = copy.file.display().to_string().replace('\'', "''");
    database.write().execute_unprepared(&format!("VACUUM INTO '{quoted}'")).await?;
    copy.restrict()?;
    let url = format!("sqlite://{}?mode=rw", copy.file.display());
    let connection = sea_orm::Database::connect(url).await?;
    let outcome = async {
        let transaction = connection.begin().await?;
        let measured = run_downs(&transaction, steps, modules).await;
        transaction.rollback().await?;
        measured
    }
    .await;
    let closed = connection.close().await;
    // `copy` bị drop ở đây (và cả khi `?` trả lỗi sớm, panic hay future bị huỷ): thư mục bản sao bị xoá.
    drop(copy);
    let measured = outcome?;
    closed?;
    Ok(measured)
}

// Thư mục riêng (0700) cạnh file database — cùng ổ, cùng ranh giới tin cậy với database — chứa bản sao (0600).
// Bản sao có toàn bộ dữ liệu (hash mật khẩu, dữ liệu cá nhân): `Drop` xoá cả thư mục, kể cả file phụ `-journal`,
// `-wal`, `-shm`, dù đo xong hay hỏng giữa chừng.
struct PrivateCopy {
    directory: std::path::PathBuf,
    file: std::path::PathBuf,
}

impl PrivateCopy {
    async fn create(database: &Database) -> Result<Self, DbErr> {
        let row = database
            .read()
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT file FROM pragma_database_list WHERE name = 'main'",
            ))
            .await?;
        let main: String = match row {
            Some(row) => row.try_get_by_index(0)?,
            None => String::new(),
        };
        // Database trong bộ nhớ (không có file): thư mục tạm của hệ thống.
        let parent = std::path::Path::new(&main)
            .parent()
            .filter(|parent| !main.is_empty() && !parent.as_os_str().is_empty())
            .map_or_else(std::env::temp_dir, std::path::Path::to_path_buf);
        let directory = parent.join(format!(".autoxide-rollback-{}", uuid::Uuid::now_v7()));
        crate::backup::private_dir(&directory).map_err(|error| {
            DbErr::Custom(format!(
                "cannot create `{}` for the dry run copy: {error}",
                directory.display()
            ))
        })?;
        let file = directory.join("copy.sqlite");
        Ok(PrivateCopy { directory, file })
    }

    // Chỉ chủ đọc được bản sao.
    fn restrict(&self) -> Result<(), DbErr> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&self.file, std::fs::Permissions::from_mode(0o600)).map_err(
                |error| DbErr::Custom(format!("cannot protect the dry run copy: {error}")),
            )?;
        }
        Ok(())
    }
}

impl Drop for PrivateCopy {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.directory)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(directory = %self.directory.display(), error = %error, "cannot delete the dry run copy");
        }
    }
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
    let oids_before = oids(connection).await?;
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
    // Postgres: chỉ đếm lại bảng mà transaction này đã xoá dòng (`n_tup_del`) hay đã bỏ rồi tạo lại (oid đổi) — không
    // `COUNT(*)` mọi bảng trong lúc giữ ACCESS EXCLUSIVE. SQLite (bản sao, hay máy chủ đã dừng): đếm lại hết.
    let touched = touched(connection, &oids_before).await?;
    let mut shrunk = Vec::new();
    for (table, rows) in counts {
        let recount = touched.as_ref().is_none_or(|touched| touched.contains(&table));
        if after.contains_key(&table) && recount {
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
    if text.contains("lock timeout") {
        DbErr::Custom(format!(
            "{text}: the dry run waited too long for a table the servers are using; run it when the load is low, \
             after stopping the servers, or with a longer `--lock-timeout-seconds`"
        ))
    } else if text.contains("statement timeout") {
        DbErr::Custom(format!(
            "{text}: one statement of the dry run ran too long; run it with a longer `--statement-timeout-seconds`"
        ))
    } else {
        error
    }
}

// Postgres: oid của từng bảng trong schema hiện tại (bảng bỏ rồi tạo lại có oid mới). SQLite: không cần, rỗng.
async fn oids(connection: &sea_orm::DatabaseTransaction) -> Result<BTreeMap<String, i64>, DbErr> {
    let backend = connection.get_database_backend();
    if backend == DbBackend::Sqlite {
        return Ok(BTreeMap::new());
    }
    let sql = "SELECT c.relname::text, c.oid::bigint FROM pg_class c \
               WHERE c.relkind = 'r' AND c.relnamespace = current_schema()::regnamespace";
    let rows = connection.query_all_raw(Statement::from_string(backend, sql)).await?;
    let mut found = BTreeMap::new();
    for row in rows {
        found.insert(row.try_get_by_index::<String>(0)?, row.try_get_by_index::<i64>(1)?);
    }
    Ok(found)
}

// Postgres: bảng mà transaction này đã xoá dòng (thống kê riêng của transaction) hay có oid khác trước. SQLite: `None`
// (đếm lại mọi bảng — bản sao hay database của máy chủ đã dừng, không ai khác ghi).
async fn touched(
    connection: &sea_orm::DatabaseTransaction,
    oids_before: &BTreeMap<String, i64>,
) -> Result<Option<Vec<String>>, DbErr> {
    let backend = connection.get_database_backend();
    if backend == DbBackend::Sqlite {
        return Ok(None);
    }
    let sql = "SELECT relname::text FROM pg_stat_xact_user_tables \
               WHERE schemaname = current_schema() AND n_tup_del > 0";
    let mut touched: Vec<String> = connection
        .query_all_raw(Statement::from_string(backend, sql))
        .await?
        .iter()
        .map(|row| row.try_get_by_index::<String>(0))
        .collect::<Result<_, _>>()?;
    for (table, oid) in oids(connection).await? {
        if oids_before.get(&table).is_some_and(|before| *before != oid) {
            touched.push(table);
        }
    }
    Ok(Some(touched))
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
        // Mặc định gọi hàm volatile (`nextval(...)`, thời điểm hiện tại…) cho mỗi lần đọc một giá trị mới: so `<>` vô
        // nghĩa, mọi giá trị khác NULL tính là dữ liệu.
        Some((column, Some(default))) if is_volatile(default) => {
            format!("SELECT COUNT(\"{column}\") FROM \"{table}\"")
        }
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

// Biểu thức mặc định gọi hàm cho giá trị mới mỗi lần (sequence, thời điểm, số ngẫu nhiên).
fn is_volatile(default: &str) -> bool {
    let lower = default.to_ascii_lowercase();
    ["nextval(", "now(", "current_timestamp", "random(", "gen_random_uuid(", "clock_timestamp("]
        .iter()
        .any(|call| lower.contains(call))
}
