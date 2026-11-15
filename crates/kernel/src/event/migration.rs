//! Migration tạo bảng event của một module.

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema;

/// Migration tạo bảng `<module>_events`. Module phát event thêm nó vào danh sách
/// migration của mình, ở vị trí cố định như mọi migration khác:
///
/// ```ignore
/// fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
///     vec![
///         Box::new(m0001_create_note::Migration),
///         autoxide_kernel::event::outbox_migration("m0002_create_events", "notes"),
///     ]
/// }
/// ```
pub fn outbox_migration(name: &'static str, module: &'static str) -> Box<dyn MigrationTrait> {
    Box::new(OutboxMigration { name, module })
}

struct OutboxMigration {
    name: &'static str,
    module: &'static str,
}

impl MigrationName for OutboxMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for OutboxMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = super::table_name(self.module);
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(&table))
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("name", 128))
                    .col(schema::uuid_null("actor_user_id"))
                    .col(schema::uuid_null("actor_api_key_id"))
                    .col(schema::string_len_null("subject", 200))
                    // JSON dạng chữ: chạy giống nhau trên SQLite và Postgres.
                    .col(schema::text("payload"))
                    .col(schema::timestamp_with_time_zone("occurred_at"))
                    // Phần giao event.
                    .col(schema::integer("attempts"))
                    .col(schema::timestamp_with_time_zone("next_attempt_at"))
                    .col(schema::timestamp_with_time_zone_null("claimed_until"))
                    .col(schema::timestamp_with_time_zone_null("delivered_at"))
                    .col(schema::timestamp_with_time_zone_null("failed_at"))
                    .col(schema::text_null("last_error"))
                    .to_owned(),
            )
            .await?;
        // Bộ phát tìm event chưa giao theo thời điểm được thử lại.
        manager
            .create_index(
                Index::create()
                    .name(format!("{table}_pending_idx"))
                    .table(Alias::new(&table))
                    .col("delivered_at")
                    .col("failed_at")
                    .col("next_attempt_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = super::table_name(self.module);
        manager.drop_table(Table::drop().table(Alias::new(&table)).to_owned()).await
    }
}

/// Migration thêm cột `actor_kind` (`system` / `user` / `public`) vào bảng `<module>_events`, để
/// event do người lạ gây ra (gửi form, đăng ký bản tin…) phân biệt được với event của hệ thống
/// (D-122). Module phát event thêm nó NGAY SAU [`outbox_migration`] (module mới) hoặc như
/// migration kế tiếp (module có sẵn). Dòng cũ không có giá trị: đọc ra là `system` / `user`
/// theo `actor_user_id`.
pub fn outbox_actor_migration(name: &'static str, module: &'static str) -> Box<dyn MigrationTrait> {
    Box::new(ActorKindMigration { name, module })
}

struct ActorKindMigration {
    name: &'static str,
    module: &'static str,
}

impl MigrationName for ActorKindMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for ActorKindMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = super::table_name(self.module);
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new(&table))
                    .add_column(schema::string_len_null("actor_kind", 16))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = super::table_name(self.module);
        manager
            .alter_table(
                Table::alter().table(Alias::new(&table)).drop_column("actor_kind").to_owned(),
            )
            .await
    }
}

/// Bảng `<module>_events` sau khi nâng theo khách (bước 33 A4): `tenant_id` đứng đầu; index quét
/// của bộ phát (mọi khách) đổi tên `…_pending_sweep_idx`. Module thêm vào migration của mình:
///
/// ```ignore
/// autoxide_kernel::tenancy::migration::adopt(
///     "m0004_tenancy_helpers",
///     vec![autoxide_kernel::event::tenant_table("notes")],
/// ),
/// ```
pub fn tenant_table(module: &str) -> crate::tenancy::migration::TenantTable {
    let table = super::table_name(module);
    crate::tenancy::migration::TenantTable::new(&table)
        .sweep_index(
            format!("{table}_pending_sweep_idx"),
            ["delivered_at", "failed_at", "next_attempt_at"],
        )
        .drop_index(format!("{table}_pending_idx"))
}
