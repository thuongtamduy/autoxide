//! Rolling a database back to the migrations of an earlier release (`autoxide rollback --to X.Y.Z`,
//! D-20261126-upgrade-rollback). The caller says which applied migrations the target release has (`keep`); every
//! other applied migration is reverted, newest first: modules in the reverse of their registration (dependency)
//! order, and inside a module in the reverse of its run order.
//!
//! [`Migrator::rollback_plan`] is the dry run. It also measures the data the rollback would lose: it runs the schema
//! `down`s where they change nothing — on SQLite on a copy of the database (`VACUUM INTO`), on Postgres inside a
//! transaction that is rolled back, bounded by [`DryRunLimits`] (the DDL holds ACCESS EXCLUSIVE locks on the tables
//! it changes until then) — and compares tables, columns and row counts before and after. Data upgrades are listed,
//! not run, in the dry run (they open their own transactions).

mod loss;
pub use loss::DryRunLimits;

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

/// Data a rollback removes: a whole table (`column: None`, `dropped`), one column (values neither NULL nor the
/// column's default), or rows a `down` deletes from a table it keeps (`column: None`, not `dropped`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LostData {
    pub table: String,
    pub column: Option<String>,
    pub rows: u64,
    /// The table itself disappears.
    pub dropped: bool,
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
        limits: DryRunLimits,
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
        let data_loss = loss::measure(database, &steps, limits).await.map_err(|error| {
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
