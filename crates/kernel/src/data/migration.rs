//! Migration theo module: mỗi module tự khai migration của mình, có bảng theo dõi
//! riêng, và lên / xuống độc lập với module khác.
//!
//! - Module `notes` ghi lịch sử migration vào bảng `notes_migrations`. Module sở
//!   hữu bảng của mình kể cả bảng này, nên gỡ một module hay tách nó thành service
//!   riêng không đụng tới module khác.
//! - Mọi migration chạy trong một transaction trên CẢ HAI database: hỏng giữa
//!   chừng thì không để lại gì. (Mặc định của thư viện là SQLite không dùng
//!   transaction; ở đây ép dùng để hai database xử như nhau.)
//! - Migration đã commit thì không sửa — thêm migration mới (`CLAUDE.md`).
//!
//! Module viết migration bằng `use autoxide_kernel::migration::prelude::*;`.
//!
//! **Upgrade dữ liệu** ([`DataUpgrade`], `Module::upgrades`, D-257): việc nâng cấp dữ liệu cần cấu hình
//! của chương trình (vd mã hoá secret cũ bằng khoá `AUTOXIDE_SECRET_KEY`). Tên `u0001_…`, ghi vào
//! cùng bảng `<m>_migrations`, chạy bởi `migrate up` ngay sau migration schema nó khai trong
//! [`DataUpgrade::after`]; còn upgrade chờ thì `serve` từ chối chạy như còn migration chờ. Upgrade
//! chạy NGOÀI transaction của migration (nó tự mở transaction của mình) và phải chạy lại được từ
//! đầu: hỏng giữa chừng thì lần sau làm tiếp phần chưa xong.

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

mod lock;
mod manager;
pub mod schema;
pub use lock::MigrationLock;

/// Một upgrade dữ liệu của module. Xem đầu file.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait DataUpgrade: Send + Sync {
    /// Tên, duy nhất trong module, dạng `u0001_<việc>`.
    fn name(&self) -> &'static str;

    /// Migration schema của cùng module mà upgrade chạy ngay sau.
    fn after(&self) -> &'static str;

    /// Nâng dữ liệu. Chạy lại được (bỏ qua dòng đã nâng).
    async fn up(&self, context: &UpgradeContext) -> Result<(), Error>;

    /// Đưa dữ liệu về như trước (để bản cũ của chương trình đọc được). Chạy lại được.
    async fn down(&self, context: &UpgradeContext) -> Result<(), Error>;
}

/// Thứ một upgrade dữ liệu cần: database và khoá của chương trình.
#[derive(Clone, Debug)]
pub struct UpgradeContext {
    database: Database,
    secret: SecretKey,
}

impl UpgradeContext {
    /// Ngữ cảnh trên `database` với khoá `secret` — cho test của module; chương trình dựng nó
    /// trong `Migrator`.
    pub fn new(database: Database, secret: SecretKey) -> Self {
        UpgradeContext { database, secret }
    }

    pub fn database(&self) -> &Database {
        &self.database
    }

    /// Khoá của chương trình (`SecretKey::sealer` để mã hoá secret).
    pub fn secret(&self) -> &SecretKey {
        &self.secret
    }
}

/// Những thứ cần để viết một migration.
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

/// Lỗi khi khai báo hoặc chạy migration.
#[derive(Debug)]
pub struct MigrationError(String);

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "migration: {}", self.0)
    }
}

impl std::error::Error for MigrationError {}

/// Các migration của một module, theo thứ tự áp dụng.
#[derive(Clone)]
pub struct ModuleMigrations {
    module: &'static str,
    // `Arc` (con trỏ đếm tham chiếu) vì thư viện migration đòi một danh sách MỚI
    // mỗi lần hỏi trạng thái hay chạy, mà `Box<dyn MigrationTrait>` không sao chép
    // được. Với `Arc`, "sao chép" chỉ là thêm một tham chiếu tới cùng migration.
    migrations: Vec<Arc<dyn MigrationTrait>>,
    upgrades: Vec<Arc<dyn DataUpgrade>>,
}

impl ModuleMigrations {
    /// `module` là tên module (chữ thường, số, `_`; bắt đầu bằng chữ). Mọi bảng của
    /// module phải mang tiền tố `<module>_`.
    pub fn new(module: &'static str, migrations: Vec<Box<dyn MigrationTrait>>) -> Self {
        ModuleMigrations {
            module,
            migrations: migrations.into_iter().map(Arc::from).collect(),
            upgrades: Vec::new(),
        }
    }

    /// Thêm các upgrade dữ liệu của module.
    pub fn with_upgrades(mut self, upgrades: Vec<Box<dyn DataUpgrade>>) -> Self {
        self.upgrades = upgrades.into_iter().map(Arc::from).collect();
        self
    }

    /// Migration schema và upgrade theo thứ tự chạy: mỗi upgrade ngay sau migration nó khai.
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

    /// Tên bảng ghi lịch sử migration của module.
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

/// Trạng thái của một migration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MigrationState {
    pub module: &'static str,
    pub name: String,
    pub applied: bool,
}

/// Chạy migration của mọi module đã đăng ký.
#[derive(Debug)]
pub struct Migrator {
    modules: Vec<ModuleMigrations>,
    /// Khoá cho upgrade dữ liệu. Mặc định: khoá gốc kernel giữ trong database.
    keys: KeyConfig,
    /// Database giữ khoá gốc tự sinh (khi `AUTOXIDE_SECRET_KEY` chưa đặt). `None`: chính
    /// database đang migrate. Placement dùng khoá của database điều khiển ([`Migrator::keyed_on`]).
    key_database: Option<Database>,
}

impl Migrator {
    /// Kiểm khai báo rồi dựng migrator. Lỗi khai báo (tên trùng, tên sai dạng) là
    /// lỗi của người viết module, báo ngay lúc khởi động chứ không đợi tới khi chạy.
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

    /// Khoá của chương trình cho upgrade dữ liệu (`Modules::migrator` đặt).
    pub(crate) fn with_keys(mut self, keys: KeyConfig) -> Self {
        self.keys = keys;
        self
    }

    /// Upgrade dữ liệu dùng khoá gốc của `control` (database điều khiển), dù migrate database nào —
    /// module mã hoá secret của mọi khách bằng khoá đó, nên upgrade trên placement cũng phải vậy.
    #[must_use]
    pub fn keyed_on(mut self, control: &Database) -> Self {
        self.key_database = Some(control.clone());
        self
    }

    /// Tên mọi migration và upgrade dữ liệu theo đúng thứ tự `up` chạy, dạng `<module>/<tên>`.
    /// Không cần database. Dùng làm khoá cache của database mẫu đã migrate trong test
    /// (`autoxide-testkit`, D-20261120-test-db-template): đổi migration thì khoá đổi.
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

    /// Khoá gốc của chương trình có được cấu hình (`AUTOXIDE_SECRET_KEY`) không. Không thì upgrade
    /// dữ liệu cần khoá sinh khoá gốc và lưu nó trong database đang migrate — kết quả migration khác
    /// nhau, nên khoá cache của database mẫu trong test có phần này (D-20261120-test-db-template).
    pub fn uses_configured_key(&self) -> bool {
        self.keys.is_configured()
    }

    /// Trạng thái mọi migration, theo thứ tự module đăng ký rồi thứ tự khai báo.
    /// Chỉ đọc: không tạo bảng nào, chạy được trên database trống.
    pub async fn status(&self, database: &Database) -> Result<Vec<MigrationState>, MigrationError> {
        let mut states = Vec::new();
        for set in &self.modules {
            let applied = applied_names(set, database.read()).await?;
            states.extend(set.entries().iter().map(|entry| MigrationState {
                module: set.module,
                name: entry.name().to_owned(),
                applied: applied.contains(entry.name()),
            }));
        }
        Ok(states)
    }

    /// Các migration chưa áp dụng.
    pub async fn pending(
        &self,
        database: &Database,
    ) -> Result<Vec<MigrationState>, MigrationError> {
        let states = self.status(database).await?;
        Ok(states.into_iter().filter(|state| !state.applied).collect())
    }

    /// Áp dụng mọi migration còn thiếu của mọi module. Trả về những migration vừa
    /// áp dụng. Gặp lỗi thì dừng; các migration trước đó vẫn được giữ.
    pub async fn up(&self, database: &Database) -> Result<Vec<MigrationState>, MigrationError> {
        let mut done = Vec::new();
        let context = self.context(database);
        for set in &self.modules {
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

    /// Gỡ `steps` migration mới nhất của MỘT module. Trả về những migration vừa gỡ,
    /// mới nhất trước.
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

/// Tên các migration (và upgrade) đã áp dụng của module. Tên có trong database mà không có trong code là lỗi:
/// database được bản khác (mới hơn) của chương trình migrate.
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

/// Một mục trong danh sách chạy của module.
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

    /// Áp dụng (hay gỡ) rồi ghi (hay xoá) dòng lịch sử. Migration schema: cả hai trong MỘT transaction, trên
    /// database nào cũng vậy — hỏng giữa chừng thì không để lại gì. Upgrade dữ liệu: ngoài transaction (nó tự
    /// mở transaction của mình), ghi lịch sử sau khi xong.
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

/// Lỗi "database chưa migrate" (`kernel.migrations_pending`) cho lệnh CLI: chỉ nêu SỐ migration
/// chưa chạy và vài tên đầu, kèm lệnh cần chạy. Không liệt kê hết — database mới tinh có hàng
/// trăm migration và một dòng lỗi dài vài KB che mất lệnh sửa. Danh sách đầy đủ:
/// `autoxide migrate status`.
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
