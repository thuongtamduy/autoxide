//! Phần nền của migration mà kernel tự giữ (D-20261121-own-migrations): trait module cài ([`MigrationTrait`],
//! [`MigrationName`]), [`SchemaManager`] (chạy câu DDL trên kết nối hay transaction), và bảng lịch sử
//! `<m>_migrations`. Thay `sea-orm-migration`, vốn bật bộ feature mặc định của SeaORM (kéo `chrono`).
//!
//! Cùng tên, cùng chữ ký với `sea-orm-migration` 2.0 mà module đã viết theo, nên migration của module không phải
//! sửa. Bảng lịch sử dựng từ cùng một entity (`version` chuỗi làm khoá chính, `applied_at` giây Unix) như trước:
//! database cũ đọc tiếp được, không migration dữ liệu.

use sea_orm::sea_query::{
    Alias, Expr, ExprTrait as _, ForeignKeyCreateStatement, ForeignKeyDropStatement,
    IndexCreateStatement, IndexDropStatement, IntoIden, Order, Query, TableAlterStatement,
    TableCreateStatement, TableDropStatement, TableRenameStatement, TableTruncateStatement,
};
use sea_orm::{
    ConnectionTrait, DatabaseExecutor, DbBackend, DbErr, DynIden, FromQueryResult as _,
    IntoDatabaseExecutor, Iterable as _, Schema, StatementBuilder,
};

/// Tên của một migration (`m0001_create_note`): ghi vào bảng lịch sử.
pub trait MigrationName {
    fn name(&self) -> &str;
}

/// Một migration của module: `up` áp dụng, `down` gỡ. Kernel chạy mỗi migration trong một transaction trên cả
/// hai database (hỏng giữa chừng thì không để lại gì).
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

/// Chạy câu lệnh dựng schema trên một kết nối hay transaction.
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

    /// Chạy một câu lệnh dựng sẵn.
    pub async fn execute<S: StatementBuilder>(&self, statement: S) -> Result<(), DbErr> {
        self.connection.execute(&statement).await.map(drop)
    }

    /// Như [`execute`](Self::execute) (tên cũ của `sea-orm-migration`).
    pub async fn exec_stmt<S: StatementBuilder>(&self, statement: S) -> Result<(), DbErr> {
        self.execute(statement).await
    }

    pub fn get_database_backend(&self) -> DbBackend {
        self.connection.get_database_backend()
    }

    /// Kết nối (hay transaction) bên dưới, cho câu lệnh dữ liệu trong migration.
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

    /// Bảng `table` có trong schema hiện tại không.
    pub async fn has_table<T: AsRef<str>>(&self, table: T) -> Result<bool, DbErr> {
        has_table(&self.connection, table.as_ref()).await
    }
}

/// Bảng `table` có không — chỉ đọc catalog (chạy được với người dùng không có quyền DDL), như `sea-schema`.
pub(super) async fn has_table<C: ConnectionTrait>(
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

/// Một dòng của bảng lịch sử `<m>_migrations`. Tên `seaql_migrations` chỉ là tên mặc định của entity (như
/// `sea-orm-migration`); mọi câu lệnh đều đổi sang tên bảng của module.
mod history {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "seaql_migrations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub version: String,
        pub applied_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

fn table_iden(table: &str) -> DynIden {
    Alias::new(table).into_iden()
}

/// Tạo bảng lịch sử nếu chưa có (cùng câu `CREATE TABLE` như `sea-orm-migration`).
pub(super) async fn install<C: ConnectionTrait>(connection: &C, table: &str) -> Result<(), DbErr> {
    let schema = Schema::new(connection.get_database_backend());
    let mut statement = schema.create_table_from_entity(history::Entity);
    statement.table(table_iden(table)).if_not_exists();
    connection.execute(&statement).await.map(drop)
}

/// Tên các migration đã ghi trong bảng lịch sử, theo tên. Bảng chưa có thì rỗng (không tạo gì: chỉ đọc).
pub(super) async fn applied<C: ConnectionTrait>(
    connection: &C,
    table: &str,
) -> Result<Vec<String>, DbErr> {
    if !has_table(connection, table).await? {
        return Ok(Vec::new());
    }
    let query = Query::select()
        .columns(history::Column::iter().map(IntoIden::into_iden))
        .from(table_iden(table))
        .order_by(history::Column::Version, Order::Asc)
        .take();
    connection
        .query_all(&query)
        .await?
        .into_iter()
        .map(|row| history::Model::from_query_result(&row, "").map(|model| model.version))
        .collect()
}

/// Ghi `name` là đã áp dụng (giây Unix hiện tại).
pub(super) async fn record<C: ConnectionTrait>(
    connection: &C,
    table: &str,
    name: &str,
) -> Result<(), DbErr> {
    let applied_at = crate::time::Timestamp::now().as_unix_seconds();
    let statement = Query::insert()
        .into_table(table_iden(table))
        .columns([history::Column::Version, history::Column::AppliedAt])
        .values([name.into(), applied_at.into()])
        .map_err(|error| DbErr::Custom(error.to_string()))?
        .take();
    connection.execute(&statement).await.map(drop)
}

/// Xoá dòng của `name` (migration vừa gỡ).
pub(super) async fn forget<C: ConnectionTrait>(
    connection: &C,
    table: &str,
    name: &str,
) -> Result<(), DbErr> {
    let statement = Query::delete()
        .from_table(table_iden(table))
        .and_where(Expr::col(history::Column::Version).eq(name))
        .take();
    connection.execute(&statement).await.map(drop)
}
