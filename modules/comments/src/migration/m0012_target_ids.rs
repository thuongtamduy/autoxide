//! Cột `uid` (UUIDv7) của `comments_targets`, để liệt kê đích theo trang bằng cursor như mọi danh
//! sách (khoá chính (`target_type`, `target_id`) là hai chuỗi; `fetch_page` cần một UUID duy nhất) —
//! đề bài giao diện B8, D-270. Dòng có sẵn được cấp ID ở đây, sinh trong ứng dụng; dòng mới nhận ID
//! khi tạo. Cột để nullable vì SQLite không thêm được cột `NOT NULL` không có mặc định; index duy
//! nhất giữ không trùng.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0012_target_ids"
    }
}

const TABLE: &str = "comments_targets";
const INDEX: &str = "comments_targets_uid_key";

#[autoxide_kernel::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table(TABLE).add_column(schema::uuid_null("uid")).to_owned(),
            )
            .await?;
        let connection = manager.get_connection();
        let select = Query::select().columns(["target_type", "target_id"]).from(TABLE).to_owned();
        for row in connection.query_all(&select).await? {
            let kind: String = row.try_get("", "target_type")?;
            let id: String = row.try_get("", "target_id")?;
            let update = Query::update()
                .table(TABLE)
                .value("uid", uuid::Uuid::now_v7())
                .and_where(Expr::col("target_type").eq(kind))
                .and_where(Expr::col("target_id").eq(id))
                .to_owned();
            connection.execute(&update).await?;
        }
        manager
            .create_index(Index::create().name(INDEX).table(TABLE).col("uid").unique().to_owned())
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_index(Index::drop().name(INDEX).table(TABLE).to_owned()).await?;
        manager.alter_table(Table::alter().table(TABLE).drop_column("uid").to_owned()).await
    }
}
