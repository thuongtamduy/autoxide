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

use sea_orm::DbErr;
use sea_orm::sea_query::{Alias, DynIden, IntoIden};
use sea_orm_migration::{MigrationName, MigrationTrait, MigratorTraitSelf, SchemaManager};

use crate::database::Database;
use crate::error::Error;
use crate::module::{is_identifier, module_name_problems};
use crate::secret::{KeyConfig, SecretKey};

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
pub mod prelude {
    pub use sea_orm_migration::prelude::*;
    pub use sea_orm_migration::schema;
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

    /// Trạng thái mọi migration, theo thứ tự module đăng ký rồi thứ tự khai báo.
    /// Chỉ đọc: không tạo bảng nào, chạy được trên database trống.
    pub async fn status(&self, database: &Database) -> Result<Vec<MigrationState>, MigrationError> {
        let mut states = Vec::new();
        for set in &self.modules {
            let migrations = ModuleMigrator { set, context: None }
                .get_migration_with_status_read_only(database.read())
                .await
                .map_err(|error| failure(set.module, "cannot read migration status", &error))?;
            states.extend(migrations.iter().map(|migration| MigrationState {
                module: set.module,
                name: migration.name().to_owned(),
                applied: migration.status() == sea_orm_migration::MigrationStatus::Applied,
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
        let mut applied = Vec::new();
        let context = self.context(database);
        for set in &self.modules {
            let migrator = ModuleMigrator { set, context: Some(context.clone()) };
            let pending = migrator
                .get_pending_migrations_read_only(database.read())
                .await
                .map_err(|error| failure(set.module, "cannot read migration status", &error))?;
            if pending.is_empty() {
                continue;
            }
            migrator
                .up(database.write(), None)
                .await
                .map_err(|error| failure(set.module, "cannot apply migrations", &error))?;
            applied.extend(pending.iter().map(|migration| MigrationState {
                module: set.module,
                name: migration.name().to_owned(),
                applied: true,
            }));
        }
        Ok(applied)
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
        let migrator = ModuleMigrator { set, context: Some(self.context(database)) };
        let applied = migrator
            .get_migration_with_status_read_only(database.read())
            .await
            .map_err(|error| failure(set.module, "cannot read migration status", &error))?;
        let reverted: Vec<MigrationState> = applied
            .iter()
            .filter(|migration| migration.status() == sea_orm_migration::MigrationStatus::Applied)
            .rev()
            .take(steps as usize)
            .map(|migration| MigrationState {
                module: set.module,
                name: migration.name().to_owned(),
                applied: false,
            })
            .collect();
        if reverted.is_empty() {
            return Ok(reverted);
        }
        migrator
            .down(database.write(), Some(steps))
            .await
            .map_err(|error| failure(set.module, "cannot revert migrations", &error))?;
        Ok(reverted)
    }
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

/// Nối một [`ModuleMigrations`] vào bộ máy của sea-orm-migration. `context`: `None` khi chỉ đọc
/// trạng thái (upgrade không chạy).
struct ModuleMigrator<'a> {
    set: &'a ModuleMigrations,
    context: Option<UpgradeContext>,
}

impl MigratorTraitSelf for ModuleMigrator<'_> {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        self.set
            .entries()
            .into_iter()
            // `as Box<dyn …>`: đổi kiểu cụ thể thành "bất kỳ kiểu nào cài trait này".
            .map(|entry| match entry {
                Entry::Schema(migration) => {
                    Box::new(Transactional(migration)) as Box<dyn MigrationTrait>
                }
                Entry::Upgrade(upgrade) => Box::new(Upgrading {
                    module: self.set.module,
                    upgrade,
                    context: self.context.clone(),
                }) as Box<dyn MigrationTrait>,
            })
            .collect()
    }

    fn migration_table_name(&self) -> DynIden {
        Alias::new(self.set.history_table()).into_iden()
    }
}

/// Một upgrade dữ liệu trong bộ máy migration: chạy ngoài transaction (upgrade tự mở transaction).
struct Upgrading {
    module: &'static str,
    upgrade: Arc<dyn DataUpgrade>,
    context: Option<UpgradeContext>,
}

impl Upgrading {
    fn context(&self) -> Result<&UpgradeContext, DbErr> {
        self.context.as_ref().ok_or_else(|| {
            DbErr::Custom(format!(
                "data upgrade `{}` of module `{}` needs the configuration; run it with \
                 `autoxide migrate up`",
                self.upgrade.name(),
                self.module
            ))
        })
    }
}

impl MigrationName for Upgrading {
    fn name(&self) -> &str {
        self.upgrade.name()
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Upgrading {
    async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        let context = self.context()?;
        self.upgrade.up(context).await.map_err(|error| DbErr::Custom(error.to_string()))
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        let context = self.context()?;
        self.upgrade.down(context).await.map_err(|error| DbErr::Custom(error.to_string()))
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(false)
    }
}

/// Bọc một migration để nó luôn chạy trong transaction, trên database nào cũng vậy.
struct Transactional(Arc<dyn MigrationTrait>);

impl MigrationName for Transactional {
    fn name(&self) -> &str {
        self.0.name()
    }
}

// `async fn` trong trait dùng qua `dyn` chưa được Rust hỗ trợ trực tiếp; macro
// `async_trait` của thư viện lo việc đó.
#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Transactional {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.0.up(manager).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.0.down(manager).await
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }
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
