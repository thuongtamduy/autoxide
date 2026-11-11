//! Lệnh `autoxide migrate`: xem, áp dụng và gỡ migration của các module.

use std::process::ExitCode;

use autoxide_kernel::database::Database;
use autoxide_kernel::migration::{MigrationState, Migrator};

use super::{report_failure, report_invalid_configuration, run_with_observability};
use crate::settings;

#[derive(Debug, clap::Args)]
pub struct Arguments {
    #[command(subcommand)]
    action: Action,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Action {
    /// List every migration of every module and whether it has been applied.
    Status,
    /// Apply every migration that has not been applied yet, for all modules.
    Up,
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
        let database = Database::connect(&settings.config.database)
            .await
            .map_err(|error| error.to_string())?;
        let report = execute(&action, &migrator, &database).await;
        // Đóng kết nối cả khi lệnh hỏng; lỗi của lệnh quan trọng hơn lỗi lúc đóng.
        let closed = database.close().await.map_err(|error| format!("database: {error}"));
        let report = report?;
        closed?;
        Ok(report)
    });
    match outcome {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => report_failure(&error),
    }
}

/// Chạy một hành động và trả về nội dung in ra cho người dùng.
async fn execute(
    action: &Action,
    migrator: &Migrator,
    database: &Database,
) -> Result<String, String> {
    match action {
        Action::Status => {
            let states = migrator.status(database).await.map_err(|error| error.to_string())?;
            Ok(render_status(&states))
        }
        Action::Up => {
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
                execute(&Action::Status, &migrator, &db).await.unwrap(),
                "notes  m0001_create_note   pending\n\
                 notes  m0002_create_draft  pending\n\
                 tags   m0001_create_tag    pending\n\
                 \n0 applied, 3 pending\n"
            );
            assert_eq!(
                execute(&Action::Up, &migrator, &db).await.unwrap(),
                "Applied 3 migrations:\n  notes/m0001_create_note\n  notes/m0002_create_draft\n  \
                 tags/m0001_create_tag\n"
            );
            assert_eq!(
                execute(&Action::Up, &migrator, &db).await.unwrap(),
                "Nothing to apply: the database is up to date.\n"
            );
            assert_eq!(
                execute(&down(1), &migrator, &db).await.unwrap(),
                "Reverted 1 migration:\n  notes/m0002_create_draft\n"
            );
            assert!(
                execute(&Action::Status, &migrator, &db)
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

            assert_eq!(error, "migration: unknown module `nope`; registered modules: notes, tags");
        })
        .await
        .unwrap();
    }

    #[test]
    fn khong_co_migration_nao_thi_noi_ro() {
        assert_eq!(render_status(&[]), "No migrations: no module declares any.\n");
    }
}
