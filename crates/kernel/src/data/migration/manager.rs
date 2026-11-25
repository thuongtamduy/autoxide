//! The migration foundation that the kernel owns itself (D-20261121-own-migrations): the traits modules
//! implement ([`MigrationTrait`], [`MigrationName`]), [`SchemaManager`] (runs DDL statements on a
//! connection or transaction), and the history table `<m>_migrations`. It replaces `sea-orm-migration`,
//! which enables SeaORM's default feature set (pulling in `chrono`).
//!
//! Same names and signatures as `sea-orm-migration` 2.0, which modules were written against, so module
//! migrations need no edits. The history table is built from the same entity (`version` string as primary key,
//! `applied_at` in Unix seconds) as before: existing databases keep working, with no data migration.

use sea_orm::sea_query::{
    Alias, ColumnDef, Expr, ExprTrait as _, ForeignKeyCreateStatement, ForeignKeyDropStatement,
    IndexCreateStatement, IndexDropStatement, IntoIden, Order, Query, Table, TableAlterStatement,
    TableCreateStatement, TableDropStatement, TableRenameStatement, TableTruncateStatement,
};
use sea_orm::{
    ConnectionTrait, DatabaseExecutor, DbBackend, DbErr, DynIden, IntoDatabaseExecutor,
    StatementBuilder,
};

/// Name of a migration (`m0001_create_note`): recorded in the history table.
pub trait MigrationName {
    fn name(&self) -> &str;
}

/// A module migration: `up` applies it, `down` reverts it. The kernel runs each migration in a transaction on
/// both databases (a failure midway leaves nothing behind).
//
// `#[async_trait]`: hàm `async` gọi qua `dyn MigrationTrait` (Rust chưa hỗ trợ trực tiếp).
#[async_trait::async_trait]
pub trait MigrationTrait: MigrationName + Send + Sync {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr>;

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let _ = manager;
        Err(DbErr::Migration(format!("migration `{}` cannot be reverted", self.name())))
    }
}

/// Runs schema-building statements on a connection or a transaction.
pub struct SchemaManager<'c> {
    connection: DatabaseExecutor<'c>,
}

impl<'c> SchemaManager<'c> {
    pub fn new<T>(connection: T) -> Self
    where
        T: IntoDatabaseExecutor<'c>,
    {
        SchemaManager { connection: connection.into_database_executor() }
    }

    /// Runs one prepared statement.
    pub async fn execute<S: StatementBuilder>(&self, statement: S) -> Result<(), DbErr> {
        self.connection.execute(&statement).await.map(drop)
    }

    /// Same as [`execute`](Self::execute) (the old name from `sea-orm-migration`).
    pub async fn exec_stmt<S: StatementBuilder>(&self, statement: S) -> Result<(), DbErr> {
        self.execute(statement).await
    }

    pub fn get_database_backend(&self) -> DbBackend {
        self.connection.get_database_backend()
    }

    /// The underlying connection (or transaction), for data statements inside a migration.
    pub fn get_connection(&self) -> &DatabaseExecutor<'c> {
        &self.connection
    }

    pub async fn create_table(&self, statement: TableCreateStatement) -> Result<(), DbErr> {
        self.execute(statement).await
    }

    pub async fn create_index(&self, statement: IndexCreateStatement) -> Result<(), DbErr> {
        self.execute(statement).await
    }

    pub async fn create_foreign_key(
        &self,
        statement: ForeignKeyCreateStatement,
    ) -> Result<(), DbErr> {
        self.execute(statement).await
    }

    pub async fn alter_table(&self, statement: TableAlterStatement) -> Result<(), DbErr> {
        self.execute(statement).await
    }

    pub async fn drop_table(&self, statement: TableDropStatement) -> Result<(), DbErr> {
        self.execute(statement).await
    }

    pub async fn rename_table(&self, statement: TableRenameStatement) -> Result<(), DbErr> {
        self.execute(statement).await
    }

    pub async fn truncate_table(&self, statement: TableTruncateStatement) -> Result<(), DbErr> {
        self.execute(statement).await
    }

    pub async fn drop_index(&self, statement: IndexDropStatement) -> Result<(), DbErr> {
        self.execute(statement).await
    }

    pub async fn drop_foreign_key(&self, statement: ForeignKeyDropStatement) -> Result<(), DbErr> {
        self.execute(statement).await
    }

    /// Whether table `table` exists in the current schema.
    pub async fn has_table<T: AsRef<str>>(&self, table: T) -> Result<bool, DbErr> {
        has_table(&self.connection, table.as_ref()).await
    }
}

/// Whether the module whose history table is `history` is installed on `connection`: the table exists AND
/// records at least one applied migration. `migrate down` of every migration leaves an empty history table:
/// that module counts as not installed (dormant mode: `up` of a turned-off module skips it, nothing is
/// pending, its tables are not created again). The one definition every check uses.
pub(crate) async fn installed<C: ConnectionTrait>(
    connection: &C,
    history: &str,
) -> Result<bool, DbErr> {
    if !has_table(connection, history).await? {
        return Ok(false);
    }
    let query = Query::select().expr(Expr::val(1)).from(table_iden(history)).limit(1).to_owned();
    Ok(connection.query_one(&query).await?.is_some())
}

/// Whether table `table` exists, reading only the catalog (works for users without DDL permission), like
/// `sea-schema`.
pub(crate) async fn has_table<C: ConnectionTrait>(
    connection: &C,
    table: &str,
) -> Result<bool, DbErr> {
    let backend = connection.get_database_backend();
    let mut query = Query::select();
    query.expr_as(Expr::cust("COUNT(*) > 0"), Alias::new("has_table"));
    match backend {
        DbBackend::Postgres => {
            query
                .from((Alias::new("information_schema"), Alias::new("tables")))
                .and_where(Expr::col(Alias::new("table_schema")).eq(Expr::cust("CURRENT_SCHEMA()")))
                .and_where(Expr::col(Alias::new("table_type")).eq("BASE TABLE"))
                .and_where(Expr::col(Alias::new("table_name")).eq(table));
        }
        DbBackend::Sqlite => {
            query
                .from(Alias::new("sqlite_master"))
                .and_where(Expr::col(Alias::new("type")).eq("table"))
                .and_where(Expr::col(Alias::new("name")).ne("sqlite_sequence"))
                .and_where(Expr::col(Alias::new("name")).eq(table));
        }
        other => {
            return Err(DbErr::BackendNotSupported { db: other.as_str(), ctx: "has_table" });
        }
    }
    let row = connection
        .query_one(&query)
        .await?
        .ok_or_else(|| DbErr::Custom("cannot check whether a table exists".to_owned()))?;
    row.try_get("", "has_table")
}

/// Columns of the history table `<m>_migrations` (like `seaql_migrations` of `sea-orm-migration`): `version` is
/// the migration name, primary key; `applied_at` is the Unix seconds when it was applied.
const VERSION: &str = "version";
const APPLIED_AT: &str = "applied_at";

fn table_iden(table: &str) -> DynIden {
    Alias::new(table).into_iden()
}

/// Creates the history table if missing, with the same columns and types as `sea-orm-migration` builds from its
/// entity (`version` non-null string as primary key, `applied_at` non-null 64-bit integer).
pub(super) async fn install<C: ConnectionTrait>(connection: &C, table: &str) -> Result<(), DbErr> {
    let statement = Table::create()
        .table(table_iden(table))
        .if_not_exists()
        .col(ColumnDef::new(Alias::new(VERSION)).string().not_null().primary_key())
        .col(ColumnDef::new(Alias::new(APPLIED_AT)).big_integer().not_null())
        .take();
    connection.execute(&statement).await.map(drop)
}

/// Names of the migrations recorded in the history table, by name. Empty when the table does not exist
/// (creates nothing: read only).
pub(super) async fn applied<C: ConnectionTrait>(
    connection: &C,
    table: &str,
) -> Result<Vec<String>, DbErr> {
    if !has_table(connection, table).await? {
        return Ok(Vec::new());
    }
    let query = Query::select()
        .column(Alias::new(VERSION))
        .from(table_iden(table))
        .order_by(Alias::new(VERSION), Order::Asc)
        .take();
    connection
        .query_all(&query)
        .await?
        .into_iter()
        .map(|row| row.try_get::<String>("", VERSION))
        .collect()
}

/// Records `name` as applied (current Unix seconds).
pub(super) async fn record<C: ConnectionTrait>(
    connection: &C,
    table: &str,
    name: &str,
) -> Result<(), DbErr> {
    let applied_at = crate::time::Timestamp::now().as_unix_seconds();
    let statement = Query::insert()
        .into_table(table_iden(table))
        .columns([Alias::new(VERSION), Alias::new(APPLIED_AT)])
        .values([name.into(), applied_at.into()])
        .map_err(|error| DbErr::Custom(error.to_string()))?
        .take();
    connection.execute(&statement).await.map(drop)
}

/// Deletes the row of `name` (the migration just reverted).
pub(super) async fn forget<C: ConnectionTrait>(
    connection: &C,
    table: &str,
    name: &str,
) -> Result<(), DbErr> {
    let statement = Query::delete()
        .from_table(table_iden(table))
        .and_where(Expr::col(Alias::new(VERSION)).eq(name))
        .take();
    connection.execute(&statement).await.map(drop)
}
