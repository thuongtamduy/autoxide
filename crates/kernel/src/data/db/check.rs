//! Startup cross-check: every record a module declares matches the live schema its migrations built — the
//! table exists, every field is a column, and a tenant record's table has `tenant_id` while a global record's
//! does not. A mismatch stops startup with a message naming the record and the fix.

use sea_orm::{ConnectionTrait, DbBackend};

use std::collections::BTreeSet;
use std::sync::{OnceLock, PoisonError, RwLock};

use super::record::{Record, Scope};
use crate::database::Database;

/// What a module declares about one of its records (`Module::records`), for the startup cross-check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RecordInfo {
    /// Table name.
    pub table: &'static str,
    /// Scope `tenant`.
    pub tenant: bool,
    /// Column names.
    pub columns: &'static [&'static str],
    /// Whether each column is an `Option` field.
    pub nullable: &'static [bool],
}

impl RecordInfo {
    /// The declaration of record `R`.
    pub fn of<R: Record>() -> Self {
        RecordInfo {
            table: R::TABLE,
            tenant: R::Scope::TENANT,
            columns: R::COLUMNS,
            nullable: R::NULLABLE,
        }
    }
}

/// Problems of `records` of `module` against the schema of `database`; empty when they match.
pub(crate) async fn problems(
    database: &Database,
    module: &str,
    records: &[RecordInfo],
    control_plane_tables: &[&str],
) -> Result<Vec<String>, String> {
    let mut problems = Vec::new();
    for record in records {
        let table = record.table;
        let prefix = format!("{module}_");
        if !table.starts_with(&prefix) {
            problems.push(format!(
                "module `{module}` declares record of table `{table}`, which is not one of its tables (`{prefix}…`)"
            ));
            continue;
        }
        // Record `global` chỉ cho bảng control-plane mà module khai: bảng của khách không có đường đọc không lọc.
        if !record.tenant && !control_plane_tables.contains(&table) {
            problems.push(format!(
                "record of `{table}` (module `{module}`) is `scope = global`, but `{table}` is not in the module's \
                 `control_plane_tables`: a tenant table's records are `scope = tenant` with `tenant_id: TenantKey`"
            ));
        }
        let columns = columns(database, table).await?;
        if columns.is_empty() {
            problems.push(format!(
                "record of `{table}` (module `{module}`): the table does not exist; add the migration that creates it"
            ));
            continue;
        }
        for (index, column) in record.columns.iter().enumerate() {
            match columns.iter().find(|existing| existing.name == *column) {
                None => problems.push(format!(
                    "record of `{table}` (module `{module}`): field `{column}` has no column; add it in a migration \
                     or remove the field"
                )),
                // Cột cho NULL mà trường không phải `Option`: đọc một dòng NULL sẽ lỗi giữa chừng.
                Some(existing) if existing.nullable && !record.nullable.get(index).copied().unwrap_or(false) => {
                    problems.push(format!(
                        "record of `{table}` (module `{module}`): column `{column}` may be NULL, so field `{column}` \
                         must be an `Option`"
                    ));
                }
                Some(_) => {}
            }
        }
        let has_tenant = columns.iter().any(|existing| existing.name == "tenant_id");
        match (record.tenant, has_tenant) {
            (true, false) => problems.push(format!(
                "record of `{table}` (module `{module}`) is `scope = tenant` but the table has no `tenant_id`"
            )),
            (false, true) => problems.push(format!(
                "record of `{table}` (module `{module}`) is `scope = global` but the table has `tenant_id`: declare \
                 `scope = tenant`"
            )),
            _ => {}
        }
    }
    Ok(problems)
}

/// The problem of control-plane table `table` of `module` when it does not carry the module's prefix: a module
/// declares only its own tables, never another module's (or a tenant table it does not own).
pub(crate) fn control_plane_prefix_problem(module: &str, table: &str) -> Option<String> {
    let prefix = format!("{module}_");
    (!table.starts_with(&prefix) || table.len() == prefix.len()).then(|| {
        format!(
            "module `{module}` declares control-plane table `{table}`, which is not one of its tables \
             (`{prefix}…`)"
        )
    })
}

/// One column of the live schema.
struct LiveColumn {
    name: String,
    nullable: bool,
}

/// Columns of `table` (empty when the table does not exist).
async fn columns(database: &Database, table: &str) -> Result<Vec<LiveColumn>, String> {
    let connection = database.read();
    let backend = connection.get_database_backend();
    // SQLite: `notnull` = 1 khi NOT NULL; khoá chính `pk > 0` cũng không nhận NULL (bảng của module luôn UUID).
    let statement = match backend {
        DbBackend::Sqlite => crate::database::statement(
            backend,
            "SELECT name, CASE WHEN \"notnull\" = 1 OR pk > 0 THEN 0 ELSE 1 END AS nullable \
             FROM pragma_table_info(?) ORDER BY cid",
            [table.into()],
        ),
        DbBackend::Postgres => crate::database::statement(
            backend,
            "SELECT column_name::text AS name, CASE WHEN is_nullable = 'YES' THEN 1 ELSE 0 END AS nullable \
             FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = $1 \
             ORDER BY ordinal_position",
            [table.into()],
        ),
        other => return Err(format!("unsupported database {other:?}")),
    };
    let rows = connection.query_all_raw(statement).await.map_err(|error| error.to_string())?;
    rows.iter()
        .map(|row| {
            let name = row.try_get::<String>("", "name").map_err(|error| error.to_string())?;
            let nullable = row.try_get::<i32>("", "nullable").map_err(|error| error.to_string())?;
            Ok(LiveColumn { name, nullable: nullable == 1 })
        })
        .collect()
}

// Sổ chung của tiến trình: mọi bảng control-plane mà module đã khai, ghi khi dựng `Modules` (chỉ thêm, không bớt).
// Đọc ở mỗi truy vấn record `global` — tập nhỏ (vài chục tên), khoá đọc rẻ.
static CONTROL_PLANE: OnceLock<RwLock<BTreeSet<&'static str>>> = OnceLock::new();

fn control_plane_set() -> &'static RwLock<BTreeSet<&'static str>> {
    CONTROL_PLANE.get_or_init(|| RwLock::new(BTreeSet::new()))
}

/// Records `tables` as control-plane tables (called when the modules are built).
pub(crate) fn register_control_plane(tables: impl IntoIterator<Item = &'static str>) {
    let mut set = control_plane_set().write().unwrap_or_else(PoisonError::into_inner);
    set.extend(tables);
}

/// Whether `table` is a registered control-plane table.
pub(crate) fn is_control_plane(table: &str) -> bool {
    control_plane_set().read().unwrap_or_else(PoisonError::into_inner).contains(table)
}

/// Only for tests: registers control-plane tables without building modules (a test table of a global record).
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn register_control_plane_for_tests(tables: &[&'static str]) {
    register_control_plane(tables.iter().copied());
}
