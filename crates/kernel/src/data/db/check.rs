//! Startup cross-check: every record a module declares matches the live schema its migrations built — the
//! table exists, every field is a column, and a tenant record's table has `tenant_id` while a global record's
//! does not. A mismatch stops startup with a message naming the record and the fix.

use sea_orm::{ConnectionTrait, DbBackend};

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
}

impl RecordInfo {
    /// The declaration of record `R`.
    pub fn of<R: Record>() -> Self {
        RecordInfo { table: R::TABLE, tenant: R::Scope::TENANT, columns: R::COLUMNS }
    }
}

/// Problems of `records` of `module` against the schema of `database`; empty when they match.
pub(crate) async fn problems(
    database: &Database,
    module: &str,
    records: &[RecordInfo],
) -> Result<Vec<String>, String> {
    let mut problems = Vec::new();
    for record in records {
        let prefix = format!("{module}_");
        if !record.table.starts_with(&prefix) {
            problems.push(format!(
                "module `{module}` declares record of table `{}`, which is not one of its tables (`{prefix}…`)",
                record.table
            ));
            continue;
        }
        let columns = columns(database, record.table).await?;
        if columns.is_empty() {
            problems.push(format!(
                "record of `{}` (module `{module}`): the table does not exist; add the migration that creates it",
                record.table
            ));
            continue;
        }
        for column in record.columns {
            if !columns.iter().any(|existing| existing == column) {
                problems.push(format!(
                    "record of `{}` (module `{module}`): field `{column}` has no column; add it in a migration or \
                     remove the field",
                    record.table
                ));
            }
        }
        let has_tenant = columns.iter().any(|existing| existing == "tenant_id");
        match (record.tenant, has_tenant) {
            (true, false) => problems.push(format!(
                "record of `{}` (module `{module}`) is `scope = tenant` but the table has no `tenant_id`",
                record.table
            )),
            (false, true) => problems.push(format!(
                "record of `{}` (module `{module}`) is `scope = global` but the table has `tenant_id`: declare \
                 `scope = tenant`",
                record.table
            )),
            _ => {}
        }
    }
    Ok(problems)
}

/// Column names of `table` (empty when the table does not exist).
async fn columns(database: &Database, table: &str) -> Result<Vec<String>, String> {
    let connection = database.read();
    let backend = connection.get_database_backend();
    let statement = match backend {
        DbBackend::Sqlite => crate::database::statement(
            backend,
            "SELECT name FROM pragma_table_info(?) ORDER BY cid",
            [table.into()],
        ),
        DbBackend::Postgres => crate::database::statement(
            backend,
            "SELECT column_name::text AS name FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
            [table.into()],
        ),
        other => return Err(format!("unsupported database {other:?}")),
    };
    let rows = connection.query_all_raw(statement).await.map_err(|error| error.to_string())?;
    rows.iter()
        .map(|row| row.try_get::<String>("", "name").map_err(|error| error.to_string()))
        .collect()
}
