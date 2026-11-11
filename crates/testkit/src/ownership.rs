//! Kiểm luật "module sở hữu bảng của mình" trên một database đã migrate:
//!
//! 1. mọi bảng mang tiền tố `<module>_` của đúng một module đã đăng ký;
//! 2. không foreign key nào trỏ sang bảng của module khác.
//!
//! Hai luật này trình biên dịch không thấy được — chúng nằm trong cấu trúc
//! database — nên phải hỏi chính database. Câu hỏi khác nhau giữa SQLite và
//! Postgres (mỗi bên có bảng danh mục riêng); cả hai nhánh nằm ở file này và đều
//! được gate chạy.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement};

use crate::TestDatabaseError;

/// Tìm các vi phạm luật sở hữu bảng. `modules` là tên các module đã đăng ký.
/// Trả về danh sách câu mô tả vi phạm (đã sắp xếp); rỗng nghĩa là hợp lệ.
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

/// Module sở hữu `table`: module có tiền tố `<module>_` khớp tên bảng.
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

/// Câu lệnh liệt kê tên mọi bảng của ứng dụng.
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

/// Câu lệnh liệt kê mọi foreign key thành cặp (bảng chứa khoá, bảng được trỏ tới).
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
