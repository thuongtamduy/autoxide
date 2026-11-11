//! Migration tạo bảng job của một module.

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema;

/// Migration tạo bảng `<module>_jobs`. Module dùng job queue thêm nó vào danh sách
/// migration của mình, như [`outbox_migration`](crate::event::outbox_migration).
pub fn jobs_migration(name: &'static str, module: &'static str) -> Box<dyn MigrationTrait> {
    Box::new(JobsMigration { name, module })
}

struct JobsMigration {
    name: &'static str,
    module: &'static str,
}

impl MigrationName for JobsMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for JobsMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = super::table_name(self.module);
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(&table))
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("kind", 128))
                    // JSON dạng chữ: chạy giống nhau trên SQLite và Postgres.
                    .col(schema::text("payload"))
                    // Chạy được từ lúc này (hẹn giờ, hoặc chờ sau lần lỗi).
                    .col(schema::timestamp_with_time_zone("run_at"))
                    .col(schema::integer("attempts"))
                    .col(schema::integer("max_attempts"))
                    // Worker đang giữ job và tới lúc nào; hết hạn thì worker khác lấy.
                    .col(schema::string_len_null("claimed_by", 64))
                    .col(schema::timestamp_with_time_zone_null("claimed_until"))
                    .col(schema::timestamp_with_time_zone_null("completed_at"))
                    // Hỏng hẳn sau `max_attempts` lần: dead-letter queue.
                    .col(schema::timestamp_with_time_zone_null("failed_at"))
                    .col(schema::text_null("last_error"))
                    // Khoá chống trùng (vd mỗi khoảng của lịch định kỳ một job). `NULL` nhiều
                    // dòng cùng lúc được, trên cả hai database.
                    .col(schema::string_len_null("unique_key", 200))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name(format!("{table}_pending_idx"))
                    .table(Alias::new(&table))
                    .col("completed_at")
                    .col("failed_at")
                    .col("run_at")
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name(format!("{table}_unique_key"))
                    .table(Alias::new(&table))
                    .col("unique_key")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = super::table_name(self.module);
        manager.drop_table(Table::drop().table(Alias::new(&table)).to_owned()).await
    }
}
