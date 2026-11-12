//! Cột `auth_user.search_text`: email và tên viết thường, để tìm người dùng.
//!
//! Viết thường trong Rust (Unicode đầy đủ, vd `Đạt` → `đạt`) rồi mới lưu: hàm `LOWER`
//! của SQLite chỉ đổi chữ ASCII, của Postgres đổi mọi chữ — tìm bằng `LOWER` trong SQL
//! sẽ cho hai kết quả khác nhau trên hai database.

use autoxide_kernel::migration::prelude::*;
use sea_orm::ConnectionTrait;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0016_add_user_search"
    }
}

/// Chép lại cách tính ở `user::service::search_text` lúc viết migration này: migration
/// đã commit không được đổi theo code về sau.
fn search_text(email: &str, name: &str) -> String {
    format!("{email} {name}").to_lowercase()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("auth_user")
                    .add_column(schema::string("search_text").default(""))
                    .to_owned(),
            )
            .await?;
        // Điền cho tài khoản có sẵn, từng dòng (bảng người dùng không lớn tới mức cần
        // làm theo lô).
        let connection = manager.get_connection();
        let select = Query::select().columns(["id", "email", "name"]).from("auth_user").to_owned();
        for row in connection.query_all(&select).await? {
            let id: uuid::Uuid = row.try_get("", "id")?;
            let email: String = row.try_get("", "email")?;
            let name: String = row.try_get("", "name")?;
            let update = Query::update()
                .table("auth_user")
                .value("search_text", search_text(&email, &name))
                .and_where(Expr::col("id").eq(id))
                .to_owned();
            connection.execute(&update).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(Table::alter().table("auth_user").drop_column("search_text").to_owned())
            .await
    }
}
