//! Tính lại `auth_user.search_text` dạng bỏ dấu (`search::fold`), để "dat" tìm ra "Đạt"
//! như nhau trên SQLite và Postgres (D-104).

use autoxide_kernel::migration::prelude::*;
use sea_orm::ConnectionTrait;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0017_fold_user_search"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Dùng chính hàm của kernel: đổi cách bỏ dấu về sau thì phải thêm migration tính
        // lại như migration này (xem `autoxide_kernel::search`).
        rewrite(manager, |email, name| autoxide_kernel::search::fold(&format!("{email} {name}")))
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Về cách của m0016: chỉ viết thường.
        rewrite(manager, |email, name| format!("{email} {name}").to_lowercase()).await
    }
}

async fn rewrite(
    manager: &SchemaManager<'_>,
    compute: impl Fn(&str, &str) -> String,
) -> Result<(), DbErr> {
    let connection = manager.get_connection();
    let select = Query::select().columns(["id", "email", "name"]).from("auth_user").to_owned();
    for row in connection.query_all(&select).await? {
        let id: uuid::Uuid = row.try_get("", "id")?;
        let email: String = row.try_get("", "email")?;
        let name: String = row.try_get("", "name")?;
        let update = Query::update()
            .table("auth_user")
            .value("search_text", compute(&email, &name))
            .and_where(Expr::col("id").eq(id))
            .to_owned();
        connection.execute(&update).await?;
    }
    Ok(())
}
