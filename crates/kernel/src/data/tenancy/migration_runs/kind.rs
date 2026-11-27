// Loại lượt của `kernel_migration_runs` (D-20261126-upgrade-rollback): `placements` (migrate các placement — lệnh,
// API, lúc khởi động), `rollback`, sau này `upgrade`; kèm phiên bản đi và đến của lượt nâng / lùi.

use crate::migration::prelude::{DbErr, MigrationName, MigrationTrait, SchemaManager, Table};
use crate::migration::schema;

/// Kind of a placement migration run.
pub(crate) const PLACEMENTS: &str = "placements";
/// Kind of a rollback run.
pub(crate) const ROLLBACK: &str = "rollback";

pub(crate) struct AddMigrationRunKind;

impl MigrationName for AddMigrationRunKind {
    fn name(&self) -> &str {
        "m0043_migration_run_kind"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddMigrationRunKind {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Mỗi cột một lệnh ALTER: SQLite chỉ nhận một cột mỗi lệnh.
        for column in [
            schema::string_len("kind", 16).default(PLACEMENTS).to_owned(),
            schema::string_len_null("from_version", 32),
            schema::string_len_null("to_version", 32),
        ] {
            let mut column = column;
            manager
                .alter_table(
                    Table::alter()
                        .table("kernel_migration_runs")
                        .add_column(&mut column)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in ["to_version", "from_version", "kind"] {
            manager
                .alter_table(
                    Table::alter().table("kernel_migration_runs").drop_column(column).to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}
