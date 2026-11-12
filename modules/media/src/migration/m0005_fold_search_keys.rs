//! Tìm không phân biệt dấu (D-104): `media_item.name_key` tính lại bằng `search::fold`; thêm
//! `media_tag.search_key` (nhãn giữ `name_key` chỉ viết thường để chặn trùng — "Anh" và
//! "Ảnh" là hai nhãn khác nhau).

use autoxide_kernel::migration::prelude::*;
use sea_orm::ConnectionTrait;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0005_fold_search_keys"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("media_tag")
                    .add_column(schema::text("search_key").default(""))
                    .to_owned(),
            )
            .await?;
        // Dùng chính hàm của kernel: đổi cách bỏ dấu về sau thì phải thêm migration tính lại.
        let fold = autoxide_kernel::search::fold;
        rewrite(manager, "media_item", "name_key", fold).await?;
        rewrite(manager, "media_tag", "search_key", fold).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        rewrite(manager, "media_item", "name_key", autoxide_kernel::search::normalize).await?;
        manager
            .alter_table(Table::alter().table("media_tag").drop_column("search_key").to_owned())
            .await
    }
}

/// Đặt cột `column` của mọi dòng trong `table` thành `compute(name)`.
async fn rewrite(
    manager: &SchemaManager<'_>,
    table: &'static str,
    column: &'static str,
    compute: fn(&str) -> String,
) -> Result<(), DbErr> {
    let connection = manager.get_connection();
    let select = Query::select().columns(["id", "name"]).from(table).to_owned();
    for row in connection.query_all(&select).await? {
        let id: uuid::Uuid = row.try_get("", "id")?;
        let name: String = row.try_get("", "name")?;
        let update = Query::update()
            .table(table)
            .value(column, compute(&name))
            .and_where(Expr::col("id").eq(id))
            .to_owned();
        connection.execute(&update).await?;
    }
    Ok(())
}
