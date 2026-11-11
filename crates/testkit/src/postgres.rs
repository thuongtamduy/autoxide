//! Phần Postgres của [`TestDatabase`](crate::TestDatabase): mỗi database test là
//! một database thật, tạo và xoá qua một kết nối "quản trị".
//!
//! `CREATE DATABASE` / `DROP DATABASE` là lệnh quản trị chỉ Postgres có và
//! không viết được bằng query builder của SeaORM, nên hai lệnh đó là SQL viết
//! tay — chỉ trong đồ test, ghi ở docs/DECISIONS.md D-031.

use std::time::SystemTime;

use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbErr};
use uuid::Uuid;

use crate::stale::is_stale;

/// Tiền tố tên database của mỗi test.
pub(crate) const DATABASE_PREFIX: &str = "autoxide_test_";

/// Kết nối quản trị: chỉ cần một kết nối, dùng xong đóng ngay.
pub(crate) async fn connect_admin(admin_url: &str) -> Result<DatabaseConnection, DbErr> {
    let mut options = ConnectOptions::new(admin_url.to_owned());
    options.max_connections(1).sqlx_logging(false);
    Database::connect(options).await
}

/// Tạo một database trống mang `id` trong tên, trả về tên của nó. `id` là
/// UUIDv7: thời điểm tạo nằm sẵn trong tên (xem module `stale`).
pub(crate) async fn create_database(admin: &DatabaseConnection, id: Uuid) -> Result<String, DbErr> {
    // Tên chỉ gồm chữ thường, số và `_` nên an toàn khi ghép vào câu lệnh.
    let name = format!("{DATABASE_PREFIX}{}", id.simple());
    admin.execute_unprepared(&format!("CREATE DATABASE \"{name}\"")).await?;
    Ok(name)
}

/// `DROP DATABASE` kèm FORCE: ngắt các kết nối còn mở tới database đó (bài test
/// có thể chưa đóng pool của nó).
pub(crate) async fn drop_database(admin: &DatabaseConnection, name: &str) -> Result<(), DbErr> {
    admin.execute_unprepared(&format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)")).await?;
    Ok(())
}

/// Xoá các database test đã quá cũ. Trả về số database đã xoá.
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

/// Thay phần tên database trong một URL dạng `scheme://authority/database?query`.
/// Trả `None` nếu URL không có `://` hoặc không có authority.
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

/// Che mật khẩu trong URL trước khi đưa vào thông báo lỗi.
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
