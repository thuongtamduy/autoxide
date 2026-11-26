//! Lệnh `autoxide migrate`: xem, áp dụng và gỡ migration của các module.
//!
//! `--all` (bước 33): database điều khiển rồi từng placement theo tên (`Placements`). Không có
//! `--all` thì chỉ database điều khiển; ở chế độ `multi`, `up` in thêm placement còn chờ.
//! `up --all` làm tiếp qua placement hỏng (bước 71): phần đã xong in ra stdout, từng placement hỏng
//! in ra stderr, mã thoát 1 — script deploy biết phải dừng mà các khách khác vẫn được nâng.

use std::process::ExitCode;

use autoxide_kernel::database::Database;
use autoxide_kernel::migration::{MigrationLock, MigrationState, Migrator};
use autoxide_kernel::tenancy::{
    DatabaseRegistry, PlacementFailure, PlacementMigrations, Placements, SchemaState, TenancyMode,
};

use super::{EXIT_FAILURE, report_failure, report_invalid_configuration, run_with_observability};
use crate::settings;
use crate::startup::LOCK_WAIT;

#[derive(Debug, clap::Args)]
pub struct Arguments {
    #[command(subcommand)]
    action: Action,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Action {
    /// List every migration of every module and whether it has been applied.
    Status {
        /// Also every placement (database of tenants), after the main database.
        #[arg(long)]
        all: bool,
    },
    /// Apply every migration that has not been applied yet, for all modules.
    Up {
        /// Also every placement, by name, after the main database. A failed placement does not
        /// stop the others; each failure is reported and the exit code is 1. Safe to run again
        /// after a failure (what is applied is skipped).
        #[arg(long)]
        all: bool,
    },
    /// Revert the most recent migrations of one module.
    Down {
        /// Name of the module whose migrations to revert.
        module: String,
        /// How many migrations to revert, newest first.
        #[arg(long, default_value_t = 1)]
        steps: u32,
    },
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let action = arguments.action.clone();
    let config = settings.config.clone();
    let outcome = run_with_observability(&config, async move {
        let migrator = settings.modules.migrator().map_err(|error| error.to_string())?;
        // `up` lấy khoá migration: máy chủ đang tự migrate lúc khởi động (hay một `migrate up`
        // khác) thì chờ nó xong, không chạy chồng (D-20261121-migrate-on-start).
        let lock = match &action {
            Action::Up { .. } => Some(
                MigrationLock::acquire(&settings.config.database, LOCK_WAIT)
                    .await
                    .map_err(|error| error.to_string())?,
            ),
            _ => None,
        };
        let database = Database::connect(&settings.config.database)
            .await
            .map_err(|error| error.to_string())?;
        let registry = DatabaseRegistry::new(
            database.clone(),
            settings.config.database.clone(),
            settings.config.tenancy.clone(),
        );
        let report = match settings.modules.placements(&registry) {
            Ok(placements) => match &action {
                Action::Status { all: true } | Action::Up { all: true } => {
                    report_all(&action, &placements).await
                }
                _ => {
                    let multi = registry.mode() == TenancyMode::Multi;
                    execute_main(&action, &migrator, &database, multi.then_some(&placements))
                        .await
                        .map(|output| Report { output, failed: Vec::new() })
                }
            },
            Err(error) => Err(error.to_string()),
        };
        // Đóng kết nối cả khi lệnh hỏng; lỗi của lệnh quan trọng hơn lỗi lúc đóng.
        let closed = database.close().await.map_err(|error| format!("database: {error}"));
        if let Some(lock) = lock
            && let Err(error) = lock.release().await
        {
            tracing::warn!(error = %error, "cannot release the migration lock");
        }
        let report = report?;
        closed?;
        Ok(report)
    });
    match outcome {
        Ok(report) => {
            print!("{}", report.output);
            if report.failed.is_empty() {
                return ExitCode::SUCCESS;
            }
            eprint!("{}", render_failures(&report.failed));
            ExitCode::from(EXIT_FAILURE)
        }
        Err(error) => report_failure(&error),
    }
}

/// Kết quả của lệnh: phần in ra stdout, và placement hỏng (chỉ `up --all`) in ra stderr.
pub(crate) struct Report {
    pub(crate) output: String,
    pub(crate) failed: Vec<PlacementFailure>,
}

/// Chạy một hành động trên database điều khiển; `placements` (chế độ `multi`): sau `up`, in thêm
/// placement còn chờ `migrate up --all`.
async fn execute_main(
    action: &Action,
    migrator: &Migrator,
    database: &Database,
    placements: Option<&Placements>,
) -> Result<String, String> {
    let mut report = execute(action, migrator, database).await?;
    if let (Action::Up { .. }, Some(placements)) = (action, placements) {
        let behind: Vec<String> = placements
            .check()
            .await
            .map_err(|error| super::describe_error(&error))?
            .into_iter()
            .filter(|check| check.state != SchemaState::Ready)
            .map(|check| check.name)
            .collect();
        if !behind.is_empty() {
            report.push_str(&format!(
                "Placements still waiting: {}. Run `autoxide migrate up --all`.\n",
                behind.join(", ")
            ));
        }
    }
    Ok(report)
}

/// `status --all` / `up --all`: database điều khiển (`main`) rồi từng placement; placement hỏng
/// thành `Err` (cho test chỉ cần biết thành hay hỏng).
#[cfg(test)]
pub(crate) async fn execute_all(
    action: &Action,
    placements: &Placements,
) -> Result<String, String> {
    let report = report_all(action, placements).await?;
    if report.failed.is_empty() {
        Ok(report.output)
    } else {
        Err(format!("{}{}", report.output, render_failures(&report.failed)))
    }
}

/// Như [`execute_all`], nhưng placement hỏng nằm trong báo cáo: `up` thử hết mọi placement.
pub(crate) async fn report_all(action: &Action, placements: &Placements) -> Result<Report, String> {
    let describe = |error: autoxide_kernel::error::Error| super::describe_error(&error);
    let (sections, failed) = match action {
        Action::Up { .. } => {
            let report = placements.up_all_report().await.map_err(describe)?;
            (report.done, report.failed)
        }
        _ => (placements.status_all().await.map_err(describe)?, Vec::new()),
    };
    Ok(Report { output: render_all(action, &sections), failed })
}

/// Từng placement hỏng một dòng, kèm cách làm tiếp.
fn render_failures(failed: &[PlacementFailure]) -> String {
    let mut output = format!("{} placement(s) failed and are left behind:\n", failed.len());
    for failure in failed {
        output.push_str(&format!("  {}: {}\n", failure.name, failure.error));
    }
    output.push_str(
        "Their tenants get 503 tenant_maintenance until fixed. Fix the cause and run \
         `autoxide migrate up --all` again: what is already applied is skipped.\n",
    );
    output
}

fn render_all(action: &Action, sections: &[PlacementMigrations]) -> String {
    let mut output = String::new();
    for section in sections {
        output.push_str(&format!("== {} ==\n", section.name));
        output.push_str(&match action {
            Action::Up { .. } => render_changes(
                &section.migrations,
                "Applied",
                "Nothing to apply: the database is up to date.",
            ),
            _ => render_status(&section.migrations),
        });
    }
    output
}

/// Chạy một hành động và trả về nội dung in ra cho người dùng.
async fn execute(
    action: &Action,
    migrator: &Migrator,
    database: &Database,
) -> Result<String, String> {
    match action {
        Action::Status { .. } => {
            let states = migrator.status(database).await.map_err(|error| error.to_string())?;
            Ok(render_status(&states))
        }
        Action::Up { .. } => {
            let applied = migrator.up(database).await.map_err(|error| error.to_string())?;
            Ok(render_changes(&applied, "Applied", "Nothing to apply: the database is up to date."))
        }
        Action::Down { module, steps } => {
            let reverted =
                migrator.down(database, module, *steps).await.map_err(|error| error.to_string())?;
            Ok(render_changes(
                &reverted,
                "Reverted",
                &format!("Nothing to revert: module `{module}` has no applied migrations."),
            ))
        }
    }
}

fn render_status(states: &[MigrationState]) -> String {
    if states.is_empty() {
        return "No migrations: no module declares any.\n".to_owned();
    }
    // Canh cột theo tên dài nhất để bảng dễ đọc.
    let module_width = states.iter().map(|state| state.module.len()).max().unwrap_or(0);
    let name_width = states.iter().map(|state| state.name.len()).max().unwrap_or(0);
    let mut output = String::new();
    for state in states {
        let status = if state.applied { "applied" } else { "pending" };
        output.push_str(&format!(
            "{:module_width$}  {:name_width$}  {status}\n",
            state.module, state.name
        ));
    }
    let pending = states.iter().filter(|state| !state.applied).count();
    output.push_str(&format!("\n{} applied, {pending} pending\n", states.len() - pending));
    output
}

fn render_changes(changed: &[MigrationState], verb: &str, nothing: &str) -> String {
    if changed.is_empty() {
        return format!("{nothing}\n");
    }
    let noun = if changed.len() == 1 { "migration" } else { "migrations" };
    let mut output = format!("{verb} {} {noun}:\n", changed.len());
    for state in changed {
        output.push_str(&format!("  {}/{}\n", state.module, state.name));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;
    use autoxide_kernel::migration::ModuleMigrations;
    use autoxide_kernel::migration::prelude::*;
    use autoxide_testkit::TestDatabase;

    struct CreateTable {
        name: &'static str,
        table: &'static str,
    }

    impl MigrationName for CreateTable {
        fn name(&self) -> &str {
            self.name
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for CreateTable {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create().table(self.table).col(schema::pk_uuid("id")).to_owned(),
                )
                .await
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager.drop_table(Table::drop().table(self.table).to_owned()).await
        }
    }

    fn notes() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(CreateTable { name: "m0001_create_note", table: "notes_note" }),
            Box::new(CreateTable { name: "m0002_create_draft", table: "notes_draft" }),
        ]
    }

    fn tags() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateTable { name: "m0001_create_tag", table: "tags_tag" })]
    }

    fn migrator() -> Migrator {
        Migrator::new(vec![
            ModuleMigrations::new("notes", notes()),
            ModuleMigrations::new("tags", tags()),
        ])
        .unwrap()
    }

    async fn connect(database: &TestDatabase) -> Database {
        let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
        let settings = crate::settings::read(&source).unwrap();
        Database::connect(&settings.config.database).await.unwrap()
    }

    #[tokio::test]
    async fn status_up_down_in_ra_dung_noi_dung() {
        TestDatabase::run(async |database| {
            let db = connect(database).await;
            let migrator = migrator();
            let down = |steps| Action::Down { module: "notes".to_owned(), steps };

            assert_eq!(
                execute(&Action::Status { all: false }, &migrator, &db).await.unwrap(),
                "notes  m0001_create_note   pending\n\
                 notes  m0002_create_draft  pending\n\
                 tags   m0001_create_tag    pending\n\
                 \n0 applied, 3 pending\n"
            );
            assert_eq!(
                execute(&Action::Up { all: false }, &migrator, &db).await.unwrap(),
                "Applied 3 migrations:\n  notes/m0001_create_note\n  notes/m0002_create_draft\n  \
                 tags/m0001_create_tag\n"
            );
            assert_eq!(
                execute(&Action::Up { all: false }, &migrator, &db).await.unwrap(),
                "Nothing to apply: the database is up to date.\n"
            );
            assert_eq!(
                execute(&down(1), &migrator, &db).await.unwrap(),
                "Reverted 1 migration:\n  notes/m0002_create_draft\n"
            );
            assert!(
                execute(&Action::Status { all: false }, &migrator, &db)
                    .await
                    .unwrap()
                    .ends_with("\n2 applied, 1 pending\n")
            );
            execute(&down(9), &migrator, &db).await.unwrap();
            assert_eq!(
                execute(&down(1), &migrator, &db).await.unwrap(),
                "Nothing to revert: module `notes` has no applied migrations.\n"
            );
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn module_khong_ton_tai_thi_bao_ro() {
        TestDatabase::run(async |database| {
            let db = connect(database).await;
            let action = Action::Down { module: "nope".to_owned(), steps: 1 };

            let error = execute(&action, &migrator(), &db).await.unwrap_err();

            assert_eq!(error, "migration: unknown module `nope`; modules with migrations (turned-off modules included): notes, tags");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn up_all_chay_database_dieu_khien_roi_tung_placement() {
        TestDatabase::run(async |test| {
            // Chế độ `multi` chưa mở (bước 33 phần B); placement vẫn migrate được như nhau.
            let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let settings = crate::settings::read(&source).unwrap();
            let database = Database::connect(&settings.config.database).await.unwrap();
            let registry = DatabaseRegistry::new(
                database.clone(),
                settings.config.database.clone(),
                settings.config.tenancy.clone(),
            );
            let placements = settings.modules.placements(&registry).unwrap();
            let up_all = Action::Up { all: true };

            let first = execute_all(&up_all, &placements).await.unwrap();
            assert!(first.starts_with("== main ==\nApplied "), "{first}");

            let url = test.extra_database().await.unwrap();
            let new = autoxide_kernel::tenancy::NewPlacement::new(
                "p1",
                autoxide_kernel::tenancy::PlacementKind::Shared,
                url.parse().unwrap(),
            );
            placements.add(new).await.unwrap();
            // `up` không `--all` (ở chế độ nhiều khách): chỉ `main`, kèm placement còn chờ.
            let migrator = settings.modules.migrator().unwrap();
            let main_only =
                execute_main(&Action::Up { all: false }, &migrator, &database, Some(&placements))
                    .await
                    .unwrap();
            assert_eq!(
                main_only,
                "Nothing to apply: the database is up to date.\n\
                 Placements still waiting: p1. Run `autoxide migrate up --all`.\n"
            );

            let second = execute_all(&up_all, &placements).await.unwrap();
            assert!(
                second.starts_with(
                    "== main ==\nNothing to apply: the database is up to date.\n== p1 ==\nApplied "
                ),
                "{second}"
            );
            let status = execute_all(&Action::Status { all: true }, &placements).await.unwrap();
            assert_eq!(status.matches(", 0 pending\n").count(), 2, "{status}");
            database.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    /// `up --all` làm tiếp qua placement hỏng: phần đã xong vào stdout, placement hỏng nêu tên kèm
    /// cách làm tiếp; `execute_all` thành `Err` để chỗ gọi biết lệnh chưa trọn.
    #[tokio::test]
    async fn up_all_lam_tiep_qua_placement_hong() {
        use autoxide_kernel::migration::prelude::{SchemaManager, Table};
        use autoxide_kernel::migration::schema;

        TestDatabase::run(async |test| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let settings = crate::settings::read(&source).unwrap();
            let database = Database::connect(&settings.config.database).await.unwrap();
            let registry = DatabaseRegistry::new(
                database.clone(),
                settings.config.database.clone(),
                settings.config.tenancy.clone(),
            );
            let placements = settings.modules.placements(&registry).unwrap();
            let up_all = Action::Up { all: true };
            execute_all(&up_all, &placements).await.unwrap();
            let broken = test.extra_database().await.unwrap();
            let good = test.extra_database().await.unwrap();
            for (name, url) in [("a-broken", &broken), ("b-good", &good)] {
                let new = autoxide_kernel::tenancy::NewPlacement::new(
                    name,
                    autoxide_kernel::tenancy::PlacementKind::Dedicated,
                    url.parse().unwrap(),
                );
                placements.add(new).await.unwrap();
            }
            let source = ConfigSource::from_pairs([("DATABASE_URL", broken.as_str())]);
            let blocked =
                Database::connect(&crate::settings::read(&source).unwrap().config.database)
                    .await
                    .unwrap();
            let blocker = Table::create().table("auth_user").col(schema::pk_uuid("id")).to_owned();
            SchemaManager::new(blocked.write()).create_table(blocker).await.unwrap();

            let report = report_all(&up_all, &placements).await.unwrap();
            assert!(report.output.contains("== b-good ==\nApplied "), "{}", report.output);
            assert!(!report.output.contains("a-broken"), "{}", report.output);
            let failed: Vec<&str> = report.failed.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(failed, ["a-broken"]);
            let error = execute_all(&up_all, &placements).await.unwrap_err();
            assert!(error.contains("1 placement(s) failed"), "{error}");
            assert!(error.contains("\n  a-broken: "), "{error}");
            assert!(error.contains("autoxide migrate up --all"), "{error}");
            blocked.close().await.unwrap();
            database.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    #[test]
    fn khong_co_migration_nao_thi_noi_ro() {
        assert_eq!(render_status(&[]), "No migrations: no module declares any.\n");
    }
}
