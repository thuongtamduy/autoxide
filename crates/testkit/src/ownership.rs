//! Checks the rule "a module owns its own tables" on a migrated database:
//!
//! 1. every table carries the `<module>_` prefix of exactly one registered module;
//! 2. no foreign key points to a table of another module.
//!
//! The compiler cannot see these two rules — they live in the structure of the
//! database — so the database itself must be asked. The question differs between SQLite and
//! Postgres (each has its own catalog tables); both branches live in this file and are both
//! run by the gate.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement};

use crate::TestDatabaseError;

/// Finds violations of the table ownership rule. `modules` is the names of the registered modules.
/// Returns a sorted list of sentences describing the violations; empty means valid.
pub async fn schema_ownership_violations(
    connection: &DatabaseConnection,
    modules: &[&str],
) -> Result<Vec<String>, TestDatabaseError> {
    let backend = connection.get_database_backend();
    let tables = query_names(connection, tables_sql(backend)?).await?;
    let foreign_keys = query_pairs(connection, foreign_keys_sql(backend)?).await?;

    let mut violations = Vec::new();
    for table in &tables {
        if owner(table, modules).is_none() {
            violations.push(format!(
                "table `{table}` belongs to no registered module: name it `<module>_…`"
            ));
        }
    }
    for (table, referenced) in &foreign_keys {
        let (from, to) = (owner(table, modules), owner(referenced, modules));
        // Bảng không có chủ đã bị báo ở trên; ở đây chỉ xét cặp có đủ hai chủ.
        if let (Some(from), Some(to)) = (from, to)
            && from != to
        {
            violations.push(format!(
                "foreign key from `{table}` (module `{from}`) to `{referenced}` (module `{to}`): \
                 modules must not share foreign keys — keep only the ID"
            ));
        }
    }
    violations.sort();
    violations.dedup();
    Ok(violations)
}

/// The module that owns `table`: the module whose `<module>_` prefix matches the table name.
fn owner<'a>(table: &str, modules: &[&'a str]) -> Option<&'a str> {
    modules
        .iter()
        .copied()
        .filter(|module| {
            table.strip_prefix(module).is_some_and(|rest| rest.len() > 1 && rest.starts_with('_'))
        })
        // Nếu nhiều module khớp (tên module lồng nhau), chọn tên dài nhất.
        .max_by_key(|module| module.len())
}

/// Statement listing the names of all tables of the application.
fn tables_sql(backend: DbBackend) -> Result<&'static str, TestDatabaseError> {
    match backend {
        DbBackend::Sqlite => {
            Ok("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        }
        DbBackend::Postgres => Ok("SELECT table_name::text FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_type = 'BASE TABLE'"),
        other => Err(unsupported(other)),
    }
}

/// Statement listing every foreign key as a pair (table holding the key, table pointed to).
fn foreign_keys_sql(backend: DbBackend) -> Result<&'static str, TestDatabaseError> {
    match backend {
        DbBackend::Sqlite => Ok("SELECT m.name, f.\"table\" FROM sqlite_master m \
             JOIN pragma_foreign_key_list(m.name) f WHERE m.type = 'table'"),
        DbBackend::Postgres => Ok("SELECT source.relname::text, target.relname::text \
             FROM pg_constraint c \
             JOIN pg_class source ON source.oid = c.conrelid \
             JOIN pg_class target ON target.oid = c.confrelid \
             JOIN pg_namespace n ON n.oid = c.connamespace \
             WHERE c.contype = 'f' AND n.nspname = current_schema()"),
        other => Err(unsupported(other)),
    }
}

fn unsupported(backend: DbBackend) -> TestDatabaseError {
    TestDatabaseError(format!("schema ownership check does not support {backend:?}"))
}

async fn query_names(
    connection: &DatabaseConnection,
    sql: &'static str,
) -> Result<Vec<String>, DbErr> {
    let statement = Statement::from_string(connection.get_database_backend(), sql);
    connection
        .query_all_raw(statement)
        .await?
        .iter()
        .map(|row| row.try_get_by_index::<String>(0))
        .collect()
}

async fn query_pairs(
    connection: &DatabaseConnection,
    sql: &'static str,
) -> Result<Vec<(String, String)>, DbErr> {
    let statement = Statement::from_string(connection.get_database_backend(), sql);
    connection
        .query_all_raw(statement)
        .await?
        .iter()
        .map(|row| Ok((row.try_get_by_index::<String>(0)?, row.try_get_by_index::<String>(1)?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chu_cua_bang_la_module_co_tien_to_khop() {
        let modules = ["auth", "notes"];
        assert_eq!(owner("auth_user", &modules), Some("auth"));
        assert_eq!(owner("auth_migrations", &modules), Some("auth"));
        assert_eq!(owner("notes_note", &modules), Some("notes"));
    }

    #[test]
    fn bang_khong_co_tien_to_module_thi_khong_co_chu() {
        let modules = ["auth", "notes"];
        for table in ["user", "authuser", "auth", "auth_", "audit_entry", "seaql_migrations"] {
            assert_eq!(owner(table, &modules), None, "{table}");
        }
        assert_eq!(owner("auth_user", &[]), None);
    }
}
