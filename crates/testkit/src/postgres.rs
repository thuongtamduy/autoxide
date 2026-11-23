//! The Postgres part of [`TestDatabase`](crate::TestDatabase): every test database is
//! a real database, created and dropped over an "admin" connection.
//!
//! `CREATE DATABASE` / `DROP DATABASE` are admin commands that only Postgres has and
//! that SeaORM's query builder cannot express, so those two commands are hand-written
//! SQL — only in test helpers, recorded in docs/DECISIONS.md D-031.

use std::time::SystemTime;

use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbErr};
use uuid::Uuid;

use crate::stale::is_stale;

/// Database name prefix of every test.
pub(crate) const DATABASE_PREFIX: &str = "autoxide_test_";

/// Admin connection: only one connection is needed, closed right after use.
pub(crate) async fn connect_admin(admin_url: &str) -> Result<DatabaseConnection, DbErr> {
    let mut options = ConnectOptions::new(admin_url.to_owned());
    options.max_connections(1).sqlx_logging(false);
    Database::connect(options).await
}

/// Creates an empty database with `id` in its name, returns its name. `id` is a
/// UUIDv7: the creation time is already in the name (see module `stale`).
pub(crate) async fn create_database(admin: &DatabaseConnection, id: Uuid) -> Result<String, DbErr> {
    // Tên chỉ gồm chữ thường, số và `_` nên an toàn khi ghép vào câu lệnh.
    let name = format!("{DATABASE_PREFIX}{}", id.simple());
    admin.execute_unprepared(&format!("CREATE DATABASE \"{name}\"")).await?;
    Ok(name)
}

/// `DROP DATABASE` with FORCE: drops the connections still open to that database (the test
/// may not have closed its pool).
pub(crate) async fn drop_database(admin: &DatabaseConnection, name: &str) -> Result<(), DbErr> {
    admin.execute_unprepared(&format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)")).await?;
    Ok(())
}

/// Deletes test databases that are too old. Returns the number of databases deleted.
pub(crate) async fn remove_stale_databases(
    admin: &DatabaseConnection,
    now: SystemTime,
) -> Result<usize, DbErr> {
    // `_` trong LIKE khớp một ký tự bất kỳ nên mẫu này bắt rộng hơn cần thiết;
    // `is_stale` bên dưới mới là chỗ kiểm tên đúng dạng.
    let select = Query::select()
        .expr(Expr::col("datname").cast_as("text"))
        .from("pg_database")
        .and_where(Expr::col("datname").like(format!("{DATABASE_PREFIX}%")))
        .to_owned();
    let mut removed = 0;
    for row in admin.query_all(&select).await? {
        let name: String = row.try_get_by_index(0)?;
        // Hai lần chạy test có thể cùng dọn một database: `IF EXISTS` lo việc đó;
        // lỗi khác thì bỏ qua, đây chỉ là dọn rác.
        if is_stale(&name, DATABASE_PREFIX, now) && drop_database(admin, &name).await.is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Replaces the database name part of a URL of the form `scheme://authority/database?query`.
/// Returns `None` if the URL has no `://` or no authority.
pub(crate) fn with_database(url: &str, database: &str) -> Option<String> {
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (url, None),
    };
    let (scheme, rest) = base.split_once("://")?;
    // Phần trước dấu `/` đầu tiên là authority (user:password@host:port).
    let authority = rest.split_once('/').map_or(rest, |(authority, _)| authority);
    if authority.is_empty() {
        return None;
    }
    let mut result = format!("{scheme}://{authority}/{database}");
    if let Some(query) = query {
        result.push('?');
        result.push_str(query);
    }
    Some(result)
}

/// Masks the password in a URL before putting it into an error message.
pub(crate) fn redact_password(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    // Mật khẩu có thể chứa `@` nên lấy dấu `@` CUỐI trong phần authority.
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let Some(at) = rest[..authority_end].rfind('@') else {
        return url.to_owned();
    };
    match rest[..at].split_once(':') {
        Some((user, _password)) => format!("{scheme}://{user}:***{}", &rest[at..]),
        None => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thay_ten_database_trong_url() {
        assert_eq!(
            with_database("postgres://u:p@localhost:5432/postgres", "t1").unwrap(),
            "postgres://u:p@localhost:5432/t1"
        );
        assert_eq!(
            with_database("postgres://u:p@localhost/postgres?sslmode=disable", "t1").unwrap(),
            "postgres://u:p@localhost/t1?sslmode=disable"
        );
        // URL không nêu database.
        assert_eq!(with_database("postgres://localhost", "t1").unwrap(), "postgres://localhost/t1");
        assert_eq!(with_database("postgresql://u@h:1/a/b", "t1").unwrap(), "postgresql://u@h:1/t1");
    }

    #[test]
    fn url_khong_hop_le_thi_khong_thay_duoc() {
        assert_eq!(with_database("localhost/postgres", "t1"), None);
        assert_eq!(with_database("postgres:///postgres", "t1"), None);
    }

    #[test]
    fn mat_khau_bi_che_trong_thong_bao() {
        assert_eq!(redact_password("postgres://u:secret@h:5432/db"), "postgres://u:***@h:5432/db");
        assert_eq!(redact_password("postgres://u:p@ss@h/db"), "postgres://u:***@h/db");
        // Không có mật khẩu thì giữ nguyên.
        assert_eq!(redact_password("postgres://u@h/db"), "postgres://u@h/db");
        assert_eq!(redact_password("postgres://h/db"), "postgres://h/db");
        assert_eq!(redact_password("not a url"), "not a url");
    }
}
