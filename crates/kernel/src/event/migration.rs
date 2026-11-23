//! Migration that creates the event table of a module.

use crate::migration::prelude::*;
use crate::migration::schema;

/// Migration that creates the `<module>_events` table. A module that emits events adds it to its own list
/// of migrations, at a fixed position like every other migration:
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

/// Migration that adds the `actor_kind` column (`system` / `user` / `public`) to the `<module>_events`
/// table, so events caused by strangers (form submissions, newsletter signups...) can be told apart from
/// system events (D-122). A module that emits events adds it RIGHT AFTER [`outbox_migration`] (new module)
/// or as the next migration (existing module). An old row has no value: it reads as `system` / `user` by
/// `actor_user_id`.
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

/// The `<module>_events` table after the tenant upgrade (step 33 A4): `tenant_id` comes first; the
/// dispatcher's sweep index (all tenants) is renamed `…_pending_sweep_idx`. A module adds this to its own
/// migrations:
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

/// Migration that adds `actor_access` (`direct` / `delegation` / `support`) and `actor_access_id` to the
/// `<module>_events` table (step 34, D-310): the path by which the actor entered the tenant. Every module
/// that emits events adds it at the END of its own migration list. An old row has no value: it reads as
/// `direct`.
pub fn outbox_access_migration(
    name: &'static str,
    module: &'static str,
) -> Box<dyn MigrationTrait> {
    Box::new(AccessMigration { name, module })
}

struct AccessMigration {
    name: &'static str,
    module: &'static str,
}

impl MigrationName for AccessMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AccessMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = super::table_name(self.module);
        for column in
            [schema::string_len_null("actor_access", 16), schema::uuid_null("actor_access_id")]
        {
            manager
                .alter_table(Table::alter().table(Alias::new(&table)).add_column(column).to_owned())
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = super::table_name(self.module);
        for column in ["actor_access", "actor_access_id"] {
            manager
                .alter_table(
                    Table::alter().table(Alias::new(&table)).drop_column(column).to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}
