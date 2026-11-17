//! Cột `search_key` của `content_redirects` (đề bài giao diện W-059): `q` của danh sách chuyển
//! hướng tìm trong đường dẫn cũ và địa chỉ đích, bỏ dấu, viết thường — như nhau trên SQLite và
//! Postgres (`autoxide_kernel::search`). Dòng có sẵn được tính lại ngay trong migration.

use autoxide_kernel::migration::prelude::*;
use sea_orm::ConnectionTrait;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0014_redirect_search"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("content_redirects")
                    .add_column(ColumnDef::new("search_key").text().not_null().default(""))
                    .to_owned(),
            )
            .await?;
        let connection = manager.get_connection();
        let select = Query::select()
            .columns(["id", "from_path", "to_url"])
            .from("content_redirects")
            .to_owned();
        for row in connection.query_all(&select).await? {
            let id: uuid::Uuid = row.try_get("", "id")?;
            let from: String = row.try_get("", "from_path")?;
            let to: Option<String> = row.try_get("", "to_url")?;
            // Cùng hàm với service: đổi cách tính thì thêm migration tính lại.
            let key = crate::publishing::redirects::search_key(&from, to.as_deref());
            let update = Query::update()
                .table("content_redirects")
                .value("search_key", key)
                .and_where(Expr::col("id").eq(id))
                .to_owned();
            connection.execute(&update).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table("content_redirects").drop_column("search_key").to_owned(),
            )
            .await
    }
}
