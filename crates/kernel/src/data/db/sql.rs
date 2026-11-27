//! Hand-written SQL, for the few queries the builder cannot express (measured to be slow otherwise, data
//! rule). Named parameters `:name`; on a tenant handle the text must use `:tenant`, which the kernel binds.

use sea_orm::DbBackend;
use sea_orm::sea_query;

use super::handle::{Access as _, Handle};
use super::value::{DbValue, FromRow, Row};
use crate::error::Error;

/// Hand-written SQL built from a handle: `handle.sql("SELECT … WHERE tenant_id = :tenant AND day >= :from")
/// .bind("from", day).all::<(String, i64)>()`.
///
/// The SQL must run on both SQLite and Postgres. On a tenant handle the text must contain `:tenant` (bound
/// to the handle's tenant) — a **secondary** guard only: it reads the text, so it cannot tell a real filter
/// from `:tenant` in a comment. The tenant guarantee comes from the query builder; each use of `sql` needs a
/// measured reason and its own tenant-isolation test, and `cargo xtask check` counts the uses.
pub struct Sql<'h, H> {
    handle: &'h H,
    text: String,
    binds: Vec<(String, sea_query::Value)>,
    limit: Option<std::time::Duration>,
}

impl<'h, H: Handle> Sql<'h, H> {
    pub(crate) fn new(handle: &'h H, text: &str) -> Self {
        Sql { handle, text: text.to_owned(), binds: Vec::new(), limit: None }
    }

    /// Binds `:name` to `value`.
    #[must_use]
    pub fn bind(mut self, name: &str, value: impl DbValue) -> Self {
        self.binds.push((name.to_owned(), value.into_value().0));
        self
    }

    /// Stops the query when it runs longer than `limit`, or when the caller stops waiting: the database ends it
    /// (Postgres `statement_timeout`, SQLite interrupt) and [`all`](Self::all) returns
    /// [`QUERY_TIMEOUT`](super::QUERY_TIMEOUT). For reads on a read handle only; on a write transaction `all`
    /// fails. Use it for queries whose cost depends on what a caller sends (public search).
    #[must_use]
    pub fn within(mut self, limit: std::time::Duration) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Runs a query; every row read as `Out` (a tuple).
    pub async fn all<Out: FromRow>(self) -> Result<Vec<Out>, Error> {
        let connection = self.handle.connection();
        let statement = self.render(connection.backend())?;
        let rows = match self.limit {
            Some(limit) => connection.all_within(statement, limit).await?,
            None => connection.all(statement).await?,
        };
        rows.iter().map(|row| Out::from_row(&Row(row))).collect()
    }

    /// Runs a statement that returns no rows; the number of rows changed.
    pub async fn execute(self) -> Result<u64, Error> {
        let connection = self.handle.connection();
        let statement = self.render(connection.backend())?;
        Ok(connection.execute(statement).await?)
    }

    fn render(&self, backend: DbBackend) -> Result<sea_orm::Statement, Error> {
        let (sql, values) =
            render_text(&self.text, self.handle.access().tenant(), &self.binds, backend)?;
        Ok(crate::database::statement(backend, sql, values))
    }
}

// Đổi `:tên` thành `$1…` (Postgres) hay `?` (SQLite) theo thứ tự xuất hiện. `::` (ép kiểu của Postgres)
// không phải tham số.
fn render_text(
    text: &str,
    tenant: Option<uuid::Uuid>,
    binds: &[(String, sea_query::Value)],
    backend: DbBackend,
) -> Result<(String, Vec<sea_query::Value>), Error> {
    let mut sql = String::with_capacity(text.len());
    let mut values = Vec::new();
    let mut saw_tenant = false;
    let characters: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        let previous = index.checked_sub(1).map(|i| characters[i]);
        let next = characters.get(index + 1).copied();
        let starts_name = next.is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
        if character != ':' || previous == Some(':') || next == Some(':') || !starts_name {
            sql.push(character);
            index += 1;
            continue;
        }
        let mut name = String::new();
        index += 1;
        while let Some(&c) = characters.get(index) {
            if c.is_ascii_alphanumeric() || c == '_' {
                name.push(c);
                index += 1;
            } else {
                break;
            }
        }
        let value = if name == "tenant" {
            saw_tenant = true;
            match tenant {
                Some(tenant) => sea_query::Value::from(tenant),
                None => {
                    return Err(Error::internal("`:tenant` in SQL on a handle without a tenant"));
                }
            }
        } else {
            match binds.iter().find(|(bound, _)| *bound == name) {
                Some((_, value)) => value.clone(),
                None => {
                    return Err(Error::internal(format!("SQL parameter `:{name}` is not bound")));
                }
            }
        };
        values.push(value);
        match backend {
            DbBackend::Postgres => sql.push_str(&format!("${}", values.len())),
            _ => sql.push('?'),
        }
    }
    if tenant.is_some() && !saw_tenant {
        return Err(Error::internal(
            "hand-written SQL on a tenant handle must filter with `:tenant`",
        ));
    }
    Ok((sql, values))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tham_so_co_ten_va_ep_kieu_postgres() {
        let binds = vec![("from".to_owned(), sea_query::Value::from(3_i64))];
        let tenant = Some(uuid::Uuid::nil());
        let text = "SELECT a::text FROM t WHERE tenant_id = :tenant AND n >= :from AND m = :from";
        let (sql, values) = render_text(text, tenant, &binds, DbBackend::Postgres).unwrap();
        assert_eq!(sql, "SELECT a::text FROM t WHERE tenant_id = $1 AND n >= $2 AND m = $3");
        assert_eq!(values.len(), 3);
        let (sql, _) = render_text(text, tenant, &binds, DbBackend::Sqlite).unwrap();
        assert_eq!(sql, "SELECT a::text FROM t WHERE tenant_id = ? AND n >= ? AND m = ?");
    }

    #[test]
    fn thieu_tenant_hay_thieu_tham_so_la_loi() {
        let tenant = Some(uuid::Uuid::nil());
        assert!(render_text("SELECT 1 FROM t", tenant, &[], DbBackend::Sqlite).is_err());
        assert!(
            render_text(
                "SELECT 1 FROM t WHERE x = :x AND tenant_id = :tenant",
                tenant,
                &[],
                DbBackend::Sqlite
            )
            .is_err()
        );
        assert!(
            render_text("SELECT 1 FROM t WHERE tenant_id = :tenant", None, &[], DbBackend::Sqlite)
                .is_err()
        );
        assert!(render_text("SELECT 1 FROM t", None, &[], DbBackend::Sqlite).is_ok());
    }
}
