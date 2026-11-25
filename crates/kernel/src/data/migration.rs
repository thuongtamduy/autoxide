//! Per-module migrations: each module declares its own migrations, has its own tracking table, and moves
//! up / down independently of other modules.
//!
//! - The `notes` module records its migration history in the table `notes_migrations`. A module owns its
//!   tables, this one included, so removing a module or splitting it into its own service does not touch
//!   other modules.
//! - Every migration runs in a transaction on BOTH databases: a failure midway leaves nothing behind. (The
//!   library default is to not use a transaction on SQLite; here it is forced so both databases behave alike.)
//! - A committed migration is never edited: add a new migration (`CLAUDE.md`).
//!
//! Modules write migrations with `use autoxide_kernel::migration::prelude::*;`.
//!
//! **Data upgrades** ([`DataUpgrade`], `Module::upgrades`, D-257): data upgrade work that needs the program
//! configuration (e.g. re-encrypting old secrets with the `AUTOXIDE_SECRET_KEY` key). Named `u0001_…`,
//! recorded in the same `<m>_migrations` table, run by `migrate up` right after the schema migration declared
//! in [`DataUpgrade::after`]; while an upgrade is pending, `serve` refuses to run, as with a pending
//! migration. An upgrade runs OUTSIDE the migration transaction (it opens its own transaction) and must be
//! re-runnable from the start: after a failure midway, the next run continues with the unfinished part.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use sea_orm::{DbErr, TransactionTrait as _};

use self::manager::SchemaManager;
pub use self::manager::{MigrationName, MigrationTrait};

use crate::database::Database;
use crate::error::Error;
use crate::module::{is_identifier, module_name_problems};
use crate::secret::{KeyConfig, SecretKey};

mod helpers;
mod lock;
mod manager;
pub mod schema;
pub use helpers::{HelperTable, helper_tables};
pub use lock::MigrationLock;
pub(crate) use manager::has_table as table_exists;

/// A data upgrade of a module. See the top of the file.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait DataUpgrade: Send + Sync {
    /// Name, unique within the module, of the form `u0001_<task>`.
    fn name(&self) -> &'static str;

    /// Schema migration of the same module that the upgrade runs right after.
    fn after(&self) -> &'static str;

    /// Upgrades the data. Re-runnable (skips rows already upgraded).
    async fn up(&self, context: &UpgradeContext) -> Result<(), Error>;

    /// Restores the data to its previous form (so an older program version can read it). Re-runnable.
    async fn down(&self, context: &UpgradeContext) -> Result<(), Error>;
}

/// What a data upgrade needs: the database and the program key.
#[derive(Clone, Debug)]
pub struct UpgradeContext {
    database: Database,
    secret: SecretKey,
}

impl UpgradeContext {
    /// Context on `database` with the key `secret`; for module tests. The program builds it inside
    /// `Migrator`.
    pub fn new(database: Database, secret: SecretKey) -> Self {
        UpgradeContext { database, secret }
    }

    pub fn database(&self) -> &Database {
        &self.database
    }

    /// The program key (`SecretKey::sealer` to encrypt secrets).
    pub fn secret(&self) -> &SecretKey {
        &self.secret
    }
}

/// Everything needed to write a migration.
//
// Cùng các tên `sea-orm-migration` từng cung cấp (kernel tự giữ phần đó, D-20261121-own-migrations), nên
// migration của module không phải sửa.
pub mod prelude {
    pub use super::manager::{MigrationName, MigrationTrait, SchemaManager};
    pub use super::schema;
    pub use async_trait;
    pub use sea_orm::sea_query::{self, *};
    pub use sea_orm::{self, ConnectionTrait, DbErr, DeriveIden};
}

/// Error while declaring or running migrations.
#[derive(Debug)]
pub struct MigrationError(String);

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "migration: {}", self.0)
    }
}

impl std::error::Error for MigrationError {}

/// The migrations of one module, in the order they are applied.
#[derive(Clone)]
pub struct ModuleMigrations {
    module: &'static str,
    // `Arc` (con trỏ đếm tham chiếu) vì thư viện migration đòi một danh sách MỚI
    // mỗi lần hỏi trạng thái hay chạy, mà `Box<dyn MigrationTrait>` không sao chép
    // được. Với `Arc`, "sao chép" chỉ là thêm một tham chiếu tới cùng migration.
    migrations: Vec<Arc<dyn MigrationTrait>>,
    upgrades: Vec<Arc<dyn DataUpgrade>>,
    /// Applied only to a database that already has the module's history table (a turned-off module, dormant
    /// mode): its tables stay up to date where it ran, and a new database never gets them.
    where_installed: bool,
}

impl ModuleMigrations {
    /// `module` is the module name (lowercase, digits, `_`; starts with a letter). Every table of the
    /// module must carry the prefix `<module>_`.
    pub fn new(module: &'static str, migrations: Vec<Box<dyn MigrationTrait>>) -> Self {
        ModuleMigrations {
            module,
            migrations: migrations.into_iter().map(Arc::from).collect(),
            upgrades: Vec::new(),
            where_installed: false,
        }
    }

    /// Applies these migrations only to a database that already has the module's history table (the
    /// module once ran there); elsewhere the module counts as having nothing pending. Used for modules that
    /// are in the program but turned off (dormant mode).
    #[must_use]
    pub fn where_installed(mut self) -> Self {
        self.where_installed = true;
        self
    }

    /// The module has nothing to do on `connection`: it is applied only where installed, and is not.
    async fn skipped_on<C: sea_orm::ConnectionTrait>(
        &self,
        connection: &C,
    ) -> Result<bool, MigrationError> {
        if !self.where_installed {
            return Ok(false);
        }
        let installed = manager::has_table(connection, &self.history_table())
            .await
            .map_err(|error| failure(self.module, "cannot read migration status", &error))?;
        Ok(!installed)
    }

    /// Adds the data upgrades of the module.
    pub fn with_upgrades(mut self, upgrades: Vec<Box<dyn DataUpgrade>>) -> Self {
        self.upgrades = upgrades.into_iter().map(Arc::from).collect();
        self
    }

    /// Schema migrations and upgrades in run order: each upgrade right after the migration it declares.
    fn entries(&self) -> Vec<Entry> {
        let mut entries = Vec::new();
        for migration in &self.migrations {
            entries.push(Entry::Schema(Arc::clone(migration)));
            for upgrade in
                self.upgrades.iter().filter(|upgrade| upgrade.after() == migration.name())
            {
                entries.push(Entry::Upgrade(Arc::clone(upgrade)));
            }
        }
        entries
    }

    pub fn module(&self) -> &'static str {
        self.module
    }

    /// Name of the table that records the migration history of the module.
    pub fn history_table(&self) -> String {
        format!("{}_migrations", self.module)
    }

    fn names(&self) -> Vec<String> {
        self.migrations
            .iter()
            .map(|migration| migration.name().to_owned())
            .chain(self.upgrades.iter().map(|upgrade| upgrade.name().to_owned()))
            .collect()
    }
}

impl fmt::Debug for ModuleMigrations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleMigrations")
            .field("module", &self.module)
            .field("migrations", &self.names())
            .finish()
    }
}

/// State of one migration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MigrationState {
    pub module: &'static str,
    pub name: String,
    pub applied: bool,
}

/// Runs the migrations of every registered module.
#[derive(Debug)]
pub struct Migrator {
    modules: Vec<ModuleMigrations>,
    /// Key for data upgrades. Default: the root key the kernel keeps in the database.
    keys: KeyConfig,
    /// Database that holds the auto-generated root key (when `AUTOXIDE_SECRET_KEY` is unset). `None`: the
    /// database being migrated itself. Placement uses the key of the control database ([`Migrator::keyed_on`]).
    key_database: Option<Database>,
}

impl Migrator {
    /// Checks the declarations and builds the migrator. A declaration error (duplicate name, malformed name)
    /// is the module author's error, reported at startup rather than waiting until run time.
    pub fn new(modules: Vec<ModuleMigrations>) -> Result<Self, MigrationError> {
        let module_names: Vec<&str> = modules.iter().map(|set| set.module).collect();
        let mut problems = module_name_problems(&module_names);
        for set in &modules {
            let mut seen_names = BTreeSet::new();
            for name in set.names() {
                if !is_identifier(&name) {
                    problems.push(format!(
                        "module `{}`: migration name `{name}` must be lowercase letters, digits \
                         and `_`, starting with a letter",
                        set.module
                    ));
                }
                if !seen_names.insert(name.clone()) {
                    problems.push(format!(
                        "module `{}`: migration `{name}` is declared twice",
                        set.module
                    ));
                }
            }
        }
        for set in &modules {
            for upgrade in &set.upgrades {
                if !upgrade.name().starts_with('u') {
                    problems.push(format!(
                        "module `{}`: data upgrade `{}` must be named `u0001_<what>`",
                        set.module,
                        upgrade.name()
                    ));
                }
                if !set.migrations.iter().any(|migration| migration.name() == upgrade.after()) {
                    problems.push(format!(
                        "module `{}`: data upgrade `{}` runs after `{}`, which is not one of its \
                         migrations",
                        set.module,
                        upgrade.name(),
                        upgrade.after()
                    ));
                }
            }
        }
        if problems.is_empty() {
            Ok(Migrator { modules, keys: KeyConfig::default(), key_database: None })
        } else {
            Err(MigrationError(problems.join("; ")))
        }
    }

    /// The program key for data upgrades (set by `Modules::migrator`).
    pub(crate) fn with_keys(mut self, keys: KeyConfig) -> Self {
        self.keys = keys;
        self
    }

    /// Data upgrades use the root key of `control` (the control database), whichever database is migrated;
    /// the module encrypts the secrets of every tenant with that key, so upgrades on a placement must do so
    /// too.
    #[must_use]
    pub fn keyed_on(mut self, control: &Database) -> Self {
        self.key_database = Some(control.clone());
        self
    }

    /// Names of every migration and data upgrade in the exact order `up` runs them, as `<module>/<name>`.
    /// Needs no database. Used as the cache key of the migrated template database in tests
    /// (`autoxide-testkit`, D-20261120-test-db-template): changing a migration changes the key.
    pub fn names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for set in &self.modules {
            for entry in set.entries() {
                let name = match entry {
                    Entry::Schema(migration) => migration.name().to_owned(),
                    Entry::Upgrade(upgrade) => upgrade.name().to_owned(),
                };
                names.push(format!("{}/{name}", set.module));
            }
        }
        names
    }

    /// Whether the program root key is configured (`AUTOXIDE_SECRET_KEY`). If not, data upgrades need the
    /// key that generates the root key and stores it in the database being migrated, so migration results
    /// differ; the template-database cache key in tests includes this part (D-20261120-test-db-template).
    pub fn uses_configured_key(&self) -> bool {
        self.keys.is_configured()
    }

    /// State of every migration, in module registration order then declaration order.
    /// Read-only: creates no table and works on an empty database.
    pub async fn status(&self, database: &Database) -> Result<Vec<MigrationState>, MigrationError> {
        let mut states = Vec::new();
        for set in &self.modules {
            if set.skipped_on(database.read()).await? {
                continue;
            }
            let applied = applied_names(set, database.read()).await?;
            states.extend(set.entries().iter().map(|entry| MigrationState {
                module: set.module,
                name: entry.name().to_owned(),
                applied: applied.contains(entry.name()),
            }));
        }
        Ok(states)
    }

    /// Migrations not yet applied.
    pub async fn pending(
        &self,
        database: &Database,
    ) -> Result<Vec<MigrationState>, MigrationError> {
        let states = self.status(database).await?;
        Ok(states.into_iter().filter(|state| !state.applied).collect())
    }

    /// Applies every missing migration of every module. Returns the migrations just applied. Stops on
    /// error; earlier migrations stay applied.
    pub async fn up(&self, database: &Database) -> Result<Vec<MigrationState>, MigrationError> {
        let mut done = Vec::new();
        let context = self.context(database);
        for set in &self.modules {
            if set.skipped_on(database.read()).await? {
                continue;
            }
            let applied = applied_names(set, database.read()).await?;
            let pending: Vec<Entry> =
                set.entries().into_iter().filter(|entry| !applied.contains(entry.name())).collect();
            if pending.is_empty() {
                continue;
            }
            let table = set.history_table();
            let write = database.write();
            manager::install(write, &table).await.map_err(|error| {
                failure(set.module, "cannot create the migration history", &error)
            })?;
            for entry in pending {
                entry
                    .run(set.module, &table, database, &context, Direction::Up)
                    .await
                    .map_err(|error| failure(set.module, "cannot apply migrations", &error))?;
                done.push(MigrationState {
                    module: set.module,
                    name: entry.name().to_owned(),
                    applied: true,
                });
            }
        }
        Ok(done)
    }

    /// Reverts the newest `steps` migrations of ONE module. Returns the migrations just reverted, newest
    /// first.
    pub async fn down(
        &self,
        database: &Database,
        module: &str,
        steps: u32,
    ) -> Result<Vec<MigrationState>, MigrationError> {
        let set = self.modules.iter().find(|set| set.module == module).ok_or_else(|| {
            let known: Vec<&str> = self.modules.iter().map(|set| set.module).collect();
            MigrationError(format!(
                "unknown module `{module}`; registered modules: {}",
                if known.is_empty() { "(none)".to_owned() } else { known.join(", ") }
            ))
        })?;
        let context = self.context(database);
        let applied = applied_names(set, database.read()).await?;
        let reverting: Vec<Entry> = set
            .entries()
            .into_iter()
            .filter(|entry| applied.contains(entry.name()))
            .rev()
            .take(steps as usize)
            .collect();
        let table = set.history_table();
        let mut reverted = Vec::new();
        for entry in reverting {
            entry
                .run(set.module, &table, database, &context, Direction::Down)
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
}

/// Names of the migrations (and upgrades) already applied for the module. A name present in the database but
/// absent from the code is an error: the database was migrated by another (newer) version of the program.
async fn applied_names<C: sea_orm::ConnectionTrait>(
    set: &ModuleMigrations,
    connection: &C,
) -> Result<BTreeSet<String>, MigrationError> {
    let recorded = manager::applied(connection, &set.history_table())
        .await
        .map_err(|error| failure(set.module, "cannot read migration status", &error))?;
    let known: BTreeSet<String> = set.names().into_iter().collect();
    let missing: Vec<String> = recorded
        .iter()
        .filter(|name| !known.contains(*name))
        .map(|name| {
            format!(
                "Migration file of version '{name}' is missing, this migration has been applied but \
                 its file is missing"
            )
        })
        .collect();
    if !missing.is_empty() {
        return Err(failure(
            set.module,
            "cannot read migration status",
            &DbErr::Custom(missing.join("\n")),
        ));
    }
    Ok(recorded.into_iter().collect())
}

fn failure(module: &str, action: &str, error: &DbErr) -> MigrationError {
    MigrationError(format!("module `{module}`: {action}: {error}"))
}

impl Migrator {
    fn context(&self, database: &Database) -> UpgradeContext {
        let keyed = self.key_database.as_ref().unwrap_or(database);
        UpgradeContext::new(database.clone(), self.keys.key_for(keyed))
    }
}

/// One entry in the run list of a module.
enum Entry {
    Schema(Arc<dyn MigrationTrait>),
    Upgrade(Arc<dyn DataUpgrade>),
}

#[derive(Clone, Copy)]
enum Direction {
    Up,
    Down,
}

impl Entry {
    fn name(&self) -> &str {
        match self {
            Entry::Schema(migration) => migration.name(),
            Entry::Upgrade(upgrade) => upgrade.name(),
        }
    }

    /// Applies (or reverts) and then records (or deletes) the history row. Schema migration: both in ONE
    /// transaction, on either database; a failure midway leaves nothing behind. Data upgrade: outside the
    /// transaction (it opens its own), recording the history after it finishes.
    async fn run(
        &self,
        module: &str,
        table: &str,
        database: &Database,
        context: &UpgradeContext,
        direction: Direction,
    ) -> Result<(), DbErr> {
        match self {
            Entry::Schema(migration) => {
                let transaction = database.write().begin().await?;
                let schema = SchemaManager::new(&transaction);
                match direction {
                    Direction::Up => {
                        migration.up(&schema).await?;
                        manager::record(&transaction, table, migration.name()).await?;
                    }
                    Direction::Down => {
                        migration.down(&schema).await?;
                        manager::forget(&transaction, table, migration.name()).await?;
                    }
                }
                transaction.commit().await
            }
            Entry::Upgrade(upgrade) => {
                let outcome = match direction {
                    Direction::Up => upgrade.up(context).await,
                    Direction::Down => upgrade.down(context).await,
                };
                outcome.map_err(|error| {
                    DbErr::Custom(format!(
                        "data upgrade `{}` of module `{module}`: {error}",
                        upgrade.name()
                    ))
                })?;
                match direction {
                    Direction::Up => manager::record(database.write(), table, upgrade.name()).await,
                    Direction::Down => {
                        manager::forget(database.write(), table, upgrade.name()).await
                    }
                }
            }
        }
    }
}

/// The "database not migrated" error (`kernel.migrations_pending`) for CLI commands: states only the COUNT
/// of pending migrations and the first few names, plus the command to run. It does not list them all; a
/// brand-new database has hundreds of migrations and a multi-KB error line would hide the fix command.
/// Full list: `autoxide migrate status`.
pub fn pending_error(pending: &[MigrationState]) -> Error {
    const SHOWN: usize = 3;
    let first: Vec<String> = pending
        .iter()
        .take(SHOWN)
        .map(|state| format!("{}/{}", state.module, state.name))
        .collect();
    let more = pending.len().saturating_sub(SHOWN);
    let first =
        if more == 0 { first.join(", ") } else { format!("{}, and {more} more", first.join(", ")) };
    Error::conflict(
        "kernel.migrations_pending",
        "{count} migration(s) have not been applied to the database (first: {first}); run \
         `autoxide migrate up` first, or `autoxide migrate status` to list them",
    )
    .with_param("count", pending.len())
    .with_param("first", first)
}

#[cfg(test)]
mod tests {
    use super::prelude::*;
    use super::*;

    struct Named(&'static str);

    impl MigrationName for Named {
        fn name(&self) -> &str {
            self.0
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Named {
        async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
            Ok(())
        }
    }

    fn one() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Named("m0001_first"))]
    }

    fn duplicated() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Named("m0001_first")), Box::new(Named("m0001_first"))]
    }

    fn badly_named() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Named("Create Users"))]
    }

    fn error(modules: Vec<ModuleMigrations>) -> String {
        Migrator::new(modules).unwrap_err().to_string()
    }

    #[test]
    fn khai_bao_hop_le_duoc_nhan() {
        let migrator = Migrator::new(vec![
            ModuleMigrations::new("auth", one()),
            ModuleMigrations::new("notes", one()),
        ]);
        assert!(migrator.is_ok());
        assert!(Migrator::new(Vec::new()).is_ok());
        assert_eq!(ModuleMigrations::new("auth", one()).history_table(), "auth_migrations");
    }

    #[test]
    fn ten_migration_theo_thu_tu_chay() {
        let migrator = Migrator::new(vec![
            ModuleMigrations::new("auth", one()),
            ModuleMigrations::new(
                "notes",
                vec![Box::new(Named("m0001_a")), Box::new(Named("m0002_b"))],
            ),
        ])
        .unwrap();
        assert_eq!(migrator.names(), ["auth/m0001_first", "notes/m0001_a", "notes/m0002_b"]);
    }

    #[test]
    fn ten_module_sai_dang_bi_tu_choi() {
        for module in ["", "Auth", "1auth", "auth-keys", "auth_", "au th"] {
            let message = error(vec![ModuleMigrations::new(module, one())]);
            assert!(
                message.contains("must be lowercase letters, digits and `_`"),
                "{module}: {message}"
            );
        }
    }

    #[test]
    fn module_dang_ky_hai_lan_bi_tu_choi() {
        let message =
            error(vec![ModuleMigrations::new("auth", one()), ModuleMigrations::new("auth", one())]);
        assert!(message.contains("module `auth` is registered twice"), "{message}");
    }

    #[test]
    fn migration_trung_ten_hoac_sai_dang_bi_tu_choi() {
        let message = error(vec![ModuleMigrations::new("auth", duplicated())]);
        assert!(message.contains("module `auth`: migration `m0001_first` is declared twice"));

        let message = error(vec![ModuleMigrations::new("auth", badly_named())]);
        assert!(message.contains("migration name `Create Users` must be lowercase"), "{message}");
    }

    #[test]
    fn ten_module_la_tien_to_cua_module_khac_bi_tu_choi() {
        let message = error(vec![
            ModuleMigrations::new("auth", one()),
            ModuleMigrations::new("auth_keys", one()),
        ]);
        assert!(message.contains("module names `auth` and `auth_keys` are ambiguous"), "{message}");
        // `auth` và `authz` không đụng nhau: tiền tố bảng là `auth_` và `authz_`.
        assert!(
            Migrator::new(vec![
                ModuleMigrations::new("auth", one()),
                ModuleMigrations::new("authz", one())
            ])
            .is_ok()
        );
    }

    #[test]
    fn moi_loi_khai_bao_duoc_bao_cung_luc() {
        let message = error(vec![
            ModuleMigrations::new("Auth", one()),
            ModuleMigrations::new("notes", duplicated()),
        ]);
        assert!(message.contains("`Auth`") && message.contains("declared twice"), "{message}");
    }
}
