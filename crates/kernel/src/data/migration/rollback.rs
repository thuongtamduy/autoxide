//! Rolling a database back to the migrations of an earlier release (`autoxide rollback --to X.Y.Z`,
//! D-20261126-upgrade-rollback). The caller says which applied migrations the target release has (`keep`); every
//! other applied migration is reverted, newest first: modules in the reverse of their registration (dependency)
//! order, and inside a module in the reverse of its run order.
//!
//! [`Migrator::rollback_plan`] is the dry run. It also measures the data the rollback would lose: it reverts the
//! schema migrations inside ONE transaction, compares the tables and columns before and after, counts the rows
//! (or non-null values) of what disappears, then rolls the transaction back. Both databases run schema changes
//! inside a transaction, so the dry run changes nothing; on Postgres it briefly holds the locks the statements
//! take. Data upgrades are listed, not run, in the dry run (they open their own transactions).

use std::collections::{BTreeMap, BTreeSet};

use sea_orm::{ConnectionTrait, DbBackend, DbErr, Statement, TransactionTrait as _};

use super::manager::{self, SchemaManager};
use super::{Entry, MigrationError, MigrationState, Migrator, applied_names, failure};
use crate::database::Database;

/// One migration (or data upgrade) to revert.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RollbackStep {
    pub module: &'static str,
    pub name: String,
    /// A data upgrade (`u0001_…`) rather than a schema migration.
    pub upgrade: bool,
}

/// Data a rollback removes: a whole table (`column: None`) or one column, with how many rows (or non-null
/// values) it holds now.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LostData {
    pub table: String,
    pub column: Option<String>,
    pub rows: u64,
}

/// What a rollback of one database does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RollbackPlan {
    /// In the order they will be reverted.
    pub steps: Vec<RollbackStep>,
    /// Tables and columns that disappear, with the data they hold.
    pub data_loss: Vec<LostData>,
}

impl RollbackPlan {
    /// Rows (or values) the rollback deletes, all tables and columns together.
    pub fn lost_rows(&self) -> u64 {
        self.data_loss.iter().map(|lost| lost.rows).sum()
    }
}

impl Migrator {
    /// The dry run of a rollback of `database` to the applied migrations `keep(module, name)` says to keep.
    /// Changes nothing. Refuses when a module would keep a migration that runs after one it reverts (the
    /// target release cannot be reached by reverting the newest ones), or when a migration to revert declares
    /// itself irreversible ([`MigrationTrait::irreversible`](super::MigrationTrait::irreversible)).
    pub async fn rollback_plan(
        &self,
        database: &Database,
        keep: &dyn Fn(&str, &str) -> bool,
    ) -> Result<RollbackPlan, MigrationError> {
        let steps = self.rollback_entries(database, keep).await?;
        let plan_steps: Vec<RollbackStep> = steps
            .iter()
            .map(|(set, entry)| RollbackStep {
                module: set.module,
                name: entry.name().to_owned(),
                upgrade: matches!(entry, Entry::Upgrade(_)),
            })
            .collect();
        if plan_steps.is_empty() {
            return Ok(RollbackPlan::default());
        }
        let data_loss = measure_loss(database, &steps).await.map_err(|error| {
            MigrationError(format!("cannot measure the data the rollback removes: {error}"))
        })?;
        Ok(RollbackPlan { steps: plan_steps, data_loss })
    }

    /// Reverts what [`rollback_plan`](Self::rollback_plan) lists, one migration per transaction, newest first.
    /// Returns the migrations reverted. Stops on the first error: what is reverted stays reverted, running
    /// again continues.
    pub async fn rollback(
        &self,
        database: &Database,
        keep: &dyn Fn(&str, &str) -> bool,
    ) -> Result<Vec<MigrationState>, MigrationError> {
        let steps = self.rollback_entries(database, keep).await?;
        let context = self.context(database);
        let mut reverted = Vec::new();
        for (set, entry) in steps {
            let table = set.history_table();
            entry
                .run(set.module, &table, database, &context, super::Direction::Down)
                .await
                .map_err(|error| failure(set.module, "cannot revert migrations", &error))?;
            reverted.push(MigrationState {
                module: set.module,
                name: entry.name().to_owned(),
                applied: false,
            });
        }
        Ok(reverted)
    }

    /// The entries to revert, newest first.
    async fn rollback_entries(
        &self,
        database: &Database,
        keep: &dyn Fn(&str, &str) -> bool,
    ) -> Result<Vec<(&super::ModuleMigrations, Entry)>, MigrationError> {
        let mut steps = Vec::new();
        for set in self.modules.iter().rev() {
            let applied = applied_names(set, database.read()).await?;
            if applied.is_empty() {
                continue;
            }
            let mut reverting = Vec::new();
            let mut kept_after: Option<String> = None;
            for entry in set.entries().into_iter().rev() {
                if !applied.contains(entry.name()) {
                    continue;
                }
                if keep(set.module, entry.name()) {
                    kept_after.get_or_insert_with(|| entry.name().to_owned());
                    continue;
                }
                if let Some(kept) = &kept_after {
                    return Err(MigrationError(format!(
                        "module `{}`: the target release keeps `{kept}` but not `{}`, which runs before it; \
                         a rollback can only revert the newest migrations of a module",
                        set.module,
                        entry.name()
                    )));
                }
                if let Entry::Schema(migration) = &entry
                    && migration.irreversible()
                {
                    return Err(MigrationError(format!(
                        "module `{}`: migration `{}` cannot be reverted (it removed data an earlier release \
                         needs); restore the backup taken before the upgrade instead",
                        set.module,
                        entry.name()
                    )));
                }
                reverting.push(entry);
            }
            steps.extend(reverting.into_iter().map(|entry| (set, entry)));
        }
        Ok(steps)
    }
}

/// Runs the schema `down`s inside one transaction, compares tables and columns before and after, counts what
/// disappears, rolls back.
async fn measure_loss(
    database: &Database,
    steps: &[(&super::ModuleMigrations, Entry)],
) -> Result<Vec<LostData>, DbErr> {
    let transaction = database.write().begin().await?;
    let before = shape(&transaction).await?;
    let mut counted = Vec::new();
    for (set, entry) in steps {
        let Entry::Schema(migration) = entry else {
            continue;
        };
        // Đếm trước khi gỡ: bảng / cột bị bỏ ở bước này thì sau đó không còn để đếm.
        let current = shape(&transaction).await?;
        let nested = transaction.begin().await?;
        migration.down(&SchemaManager::new(&nested)).await?;
        manager::forget(&nested, &set.history_table(), migration.name()).await?;
        nested.commit().await?;
        let after = shape(&transaction).await?;
        for (table, columns) in &current {
            match after.get(table) {
                None if before.contains_key(table) => {
                    counted.push(LostData { table: table.clone(), column: None, rows: 0 });
                }
                Some(remaining) => {
                    for column in columns.difference(remaining) {
                        if before.get(table).is_some_and(|original| original.contains(column)) {
                            counted.push(LostData {
                                table: table.clone(),
                                column: Some(column.clone()),
                                rows: 0,
                            });
                        }
                    }
                }
                None => {}
            }
        }
    }
    transaction.rollback().await?;
    // Đếm trên dữ liệu thật (sau khi huỷ transaction mọi bảng / cột còn nguyên).
    let backend = database.write().get_database_backend();
    for lost in &mut counted {
        lost.rows = count(database, backend, &lost.table, lost.column.as_deref()).await?;
    }
    Ok(counted)
}

type Shape = BTreeMap<String, BTreeSet<String>>;

/// Tables and their columns.
async fn shape<C: ConnectionTrait>(connection: &C) -> Result<Shape, DbErr> {
    let backend = connection.get_database_backend();
    let sql = match backend {
        DbBackend::Sqlite => {
            "SELECT m.name, p.name FROM sqlite_master m, pragma_table_info(m.name) p \
             WHERE m.type = 'table' AND m.name NOT LIKE 'sqlite_%'"
        }
        _ => {
            "SELECT table_name::text, column_name::text FROM information_schema.columns \
             WHERE table_schema = current_schema()"
        }
    };
    let rows = connection.query_all_raw(Statement::from_string(backend, sql)).await?;
    let mut shape = Shape::new();
    for row in rows {
        let table: String = row.try_get_by_index(0)?;
        let column: String = row.try_get_by_index(1)?;
        shape.entry(table).or_default().insert(column);
    }
    Ok(shape)
}

/// Rows of `table`, or its non-null values of `column`. Names come from the database catalog; checked to be
/// plain identifiers before they go into SQL.
async fn count(
    database: &Database,
    backend: DbBackend,
    table: &str,
    column: Option<&str>,
) -> Result<u64, DbErr> {
    let plain = |name: &str| name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !plain(table) || !column.is_none_or(plain) {
        return Err(DbErr::Custom(format!("unexpected table or column name `{table}`")));
    }
    let sql = match column {
        None => format!("SELECT COUNT(*) FROM \"{table}\""),
        Some(column) => format!("SELECT COUNT(\"{column}\") FROM \"{table}\""),
    };
    let row = database.read().query_one_raw(Statement::from_string(backend, sql)).await?;
    let rows: i64 = match row {
        Some(row) => row.try_get_by_index(0)?,
        None => 0,
    };
    Ok(u64::try_from(rows).unwrap_or(0))
}
