//! Lệnh `autoxide rollback --to X.Y.Z` (bước 71 G3, D-20261126-upgrade-rollback): lùi database chính và mọi placement
//! về đúng các migration của bản X.Y.Z, để binary X.Y.Z chạy lại được.
//!
//! - Chạy bằng binary MỚI: chỉ nó có `down` của các migration mới.
//! - Tập migration của bản X.Y.Z lấy từ bảng nhúng trong binary (`releases.rs`); mọi migration đã áp dụng mà không có
//!   trong bản đó bị gỡ, mới nhất trước.
//! - `--dry-run` chỉ in kế hoạch: từng database, migration sẽ gỡ, bảng / cột sẽ mất kèm số dòng. Chạy thật mà có dữ
//!   liệu mất thì cần `--accept-data-loss`; luôn cần `--yes` (xác nhận mọi máy chủ bản hiện tại đã dừng: chưa có sổ
//!   tiến trình để tự kiểm).
//! - Giữ khoá chính rồi khoá placement (D-20261126-placement-migration-exclusive); ghi một lượt `rollback` trong
//!   `kernel_migration_runs`. Placement trước, database chính sau cùng (ngược thứ tự nâng); placement nào hỏng thì
//!   database chính không bị lùi.

use std::process::ExitCode;

use autoxide_kernel::database::Database;
use autoxide_kernel::migration::{MigrationLock, Migrator, RollbackPlan};
use autoxide_kernel::tenancy::{DatabaseRegistry, MAIN_PLACEMENT, PlacementId, Placements};

use super::{EXIT_FAILURE, report_failure, report_invalid_configuration, run_with_observability};
use crate::releases::{MigrationReleases, compare};
use crate::settings;
use crate::startup::LOCK_WAIT;

#[derive(Debug, Clone, clap::Args)]
pub struct Arguments {
    /// The release to go back to, e.g. `0.19.0`. Run this command with the binary of the CURRENT release: only it
    /// knows how to revert its own migrations.
    #[arg(long)]
    to: String,
    /// Only show what would be reverted, per database, and the tables and columns that would be dropped with
    /// how many rows they hold. Changes nothing.
    #[arg(long)]
    dry_run: bool,
    /// Go on although the rollback drops tables or columns that hold data (see `--dry-run`).
    #[arg(long)]
    accept_data_loss: bool,
    /// Confirm that every server and worker of the current release is stopped.
    #[arg(long)]
    yes: bool,
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let arguments = arguments.clone();
    let config = settings.config.clone();
    let outcome = run_with_observability(&config, async move {
        let releases = MigrationReleases::embedded()?;
        let migrator = settings.modules.migrator().map_err(|error| error.to_string())?;
        let main_lock = MigrationLock::acquire(&settings.config.database, LOCK_WAIT)
            .await
            .map_err(|error| error.to_string())?;
        let database = Database::connect(&settings.config.database)
            .await
            .map_err(|error| error.to_string())?;
        let registry = DatabaseRegistry::new(
            database.clone(),
            settings.config.database.clone(),
            settings.config.tenancy.clone(),
        );
        let outcome = match settings.modules.placements(&registry) {
            Ok(placements) => {
                let migrator = migrator.keyed_on(&database);
                execute(&arguments, &releases, &migrator, &registry, &placements).await
            }
            Err(error) => Err(error.to_string()),
        };
        let closed = database.close().await.map_err(|error| format!("database: {error}"));
        if let Err(error) = main_lock.release().await {
            tracing::warn!(error = %error, "cannot release the migration lock");
        }
        let outcome = outcome?;
        closed?;
        Ok(outcome)
    });
    match outcome {
        Ok(Outcome { output, failed }) => {
            print!("{output}");
            if failed { ExitCode::from(EXIT_FAILURE) } else { ExitCode::SUCCESS }
        }
        Err(error) => report_failure(&error),
    }
}

/// Phần in ra stdout; `failed`: có database lùi hỏng (mã thoát 1).
pub(crate) struct Outcome {
    pub(crate) output: String,
    pub(crate) failed: bool,
}

/// Mọi việc sau khi đã giữ khoá chính: kiểm bản đích, lấy khoá placement, lập kế hoạch, lùi.
pub(crate) async fn execute(
    arguments: &Arguments,
    releases: &MigrationReleases,
    migrator: &Migrator,
    registry: &DatabaseRegistry,
    placements: &Placements,
) -> Result<Outcome, String> {
    let current = env!("CARGO_PKG_VERSION");
    let target = arguments.to.trim_start_matches('v');
    if !releases.knows(target) {
        return Err(format!(
            "`{target}` is not a release this binary knows; known releases: {}",
            releases.releases().join(", ")
        ));
    }
    if compare(target, current) == std::cmp::Ordering::Greater {
        return Err(format!(
            "`{target}` is newer than this binary ({current}); run the binary of `{target}` instead"
        ));
    }
    let describe = |error: autoxide_kernel::error::Error| super::describe_error(&error);
    let mut lock = placements.lock(LOCK_WAIT).await.map_err(describe)?;
    let outcome =
        rollback_locked(arguments, releases, migrator, registry, placements, target, &mut lock)
            .await;
    if let Err(error) = lock.release().await {
        tracing::warn!(error = %error, "cannot release the migration lock");
    }
    outcome
}

async fn rollback_locked(
    arguments: &Arguments,
    releases: &MigrationReleases,
    migrator: &Migrator,
    registry: &DatabaseRegistry,
    placements: &Placements,
    target: &str,
    lock: &mut MigrationLock,
) -> Result<Outcome, String> {
    let describe = |error: autoxide_kernel::error::Error| super::describe_error(&error);
    let keep = |module: &str, name: &str| releases.in_release(target, module, name);
    // Placement trước (theo tên), database chính sau cùng.
    let mut databases: Vec<(String, Database)> = Vec::new();
    for found in placements.list().await.map_err(describe)? {
        if found.id == MAIN_PLACEMENT.as_uuid() {
            continue;
        }
        let database = registry
            .placement_database(PlacementId::from_uuid(found.id))
            .await
            .map_err(describe)?;
        databases.push((found.name, database));
    }
    databases.push(("main".to_owned(), registry.control().clone()));

    let mut plans: Vec<(String, RollbackPlan)> = Vec::new();
    for (name, database) in &databases {
        let plan = migrator
            .rollback_plan(database, &keep)
            .await
            .map_err(|error| format!("{name}: {error}"))?;
        plans.push((name.clone(), plan));
    }
    let mut output = render_plans(target, &plans);
    if plans.iter().all(|(_, plan)| plan.steps.is_empty()) {
        output
            .push_str(&format!("Nothing to roll back: every database already matches {target}.\n"));
        return Ok(Outcome { output, failed: false });
    }
    if arguments.dry_run {
        output.push_str("Dry run: nothing was changed.\n");
        return Ok(Outcome { output, failed: false });
    }
    let lost: u64 = plans.iter().map(|(_, plan)| plan.lost_rows()).sum();
    if lost > 0 && !arguments.accept_data_loss {
        return Err(format!(
            "{output}The rollback drops data ({lost} row(s) or value(s) above). Take a backup first \
             (`autoxide backup`), then run again with `--accept-data-loss`."
        ));
    }
    if !arguments.yes {
        return Err(format!(
            "{output}Stop every server and worker of the current release, then run again with `--yes`."
        ));
    }

    let current = env!("CARGO_PKG_VERSION");
    let run = placements.begin_rollback_run(lock, current, target).await.map_err(describe)?;
    let mut failures = Vec::new();
    let (main, others) = databases.split_last().ok_or("no database")?;
    for (name, database) in others {
        if let Err(error) = lock.verify().await {
            failures.push(error.to_string());
            break;
        }
        match migrator.rollback(database, &keep).await {
            Ok(reverted) => output.push_str(&format!("{name}: reverted {}\n", reverted.len())),
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    let error = (!failures.is_empty()).then(|| failures.join("\n"));
    // Ghi kết thúc lượt TRƯỚC khi lùi database chính: lùi xa có thể bỏ chính các cột của bảng lượt.
    placements.finish_run(run, error.clone()).await.map_err(describe)?;
    if let Some(error) = error {
        output.push_str(&format!(
            "{error}\nThe main database was NOT rolled back: fix the failure and run again (what is reverted \
             stays reverted).\n"
        ));
        return Ok(Outcome { output, failed: true });
    }
    let (name, database) = main;
    match migrator.rollback(database, &keep).await {
        Ok(reverted) => output.push_str(&format!("{name}: reverted {}\n", reverted.len())),
        Err(error) => {
            output.push_str(&format!("{name}: {error}\n"));
            return Ok(Outcome { output, failed: true });
        }
    }
    output.push_str(&format!("Rolled back to {target}: start the servers of {target} now.\n"));
    Ok(Outcome { output, failed: false })
}

fn render_plans(target: &str, plans: &[(String, RollbackPlan)]) -> String {
    let mut output = format!("Rollback to {target}:\n");
    for (name, plan) in plans {
        output.push_str(&format!("== {name} ==\n"));
        if plan.steps.is_empty() {
            output.push_str("Nothing to revert.\n");
            continue;
        }
        for step in &plan.steps {
            let kind = if step.upgrade { " (data upgrade)" } else { "" };
            output.push_str(&format!("Revert {}/{}{kind}\n", step.module, step.name));
        }
        for lost in &plan.data_loss {
            match &lost.column {
                None => {
                    output.push_str(&format!("Drops table {} ({} row(s))\n", lost.table, lost.rows))
                }
                Some(column) => output.push_str(&format!(
                    "Drops column {}.{column} ({} value(s))\n",
                    lost.table, lost.rows
                )),
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;
    use autoxide_kernel::tenancy::{NewPlacement, PlacementKind, TenancyConfig, TenancyMode};
    use autoxide_testkit::TestDatabase;

    const NEWEST: &str = "kernel/m0043_migration_run_kind";

    fn arguments(dry_run: bool, accept: bool, yes: bool) -> Arguments {
        Arguments { to: "0.0.1".to_owned(), dry_run, accept_data_loss: accept, yes }
    }

    /// Bản giả `0.0.1`: mọi migration của binary trừ migration mới nhất của kernel.
    fn releases(migrator: &Migrator) -> MigrationReleases {
        let introduced: serde_json::Map<String, serde_json::Value> = migrator
            .names()
            .into_iter()
            .filter(|name| name != NEWEST)
            .map(|name| (name, serde_json::Value::from("0.0.1")))
            .collect();
        MigrationReleases::parse(
            &serde_json::json!({"releases": ["0.0.1"], "introduced": introduced}).to_string(),
        )
    }

    /// Lùi database chính và placement về bản trước: chạy thử không đổi gì; thiếu `--accept-data-loss` / `--yes`
    /// thì từ chối kèm kế hoạch; chạy thật gỡ đúng migration mới ở mọi database, rồi `migrate up` nâng lại được.
    #[tokio::test]
    async fn lui_database_chinh_va_placement_ve_ban_truoc() {
        TestDatabase::run(async |test| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let settings = crate::settings::read(&source).unwrap();
            let database = Database::connect(&settings.config.database).await.unwrap();
            let registry = DatabaseRegistry::new(
                database.clone(),
                settings.config.database.clone(),
                TenancyConfig::with_mode(TenancyMode::Multi),
            );
            let placements = settings.modules.placements(&registry).unwrap();
            placements.up_all().await.unwrap();
            let url = test.extra_database().await.unwrap();
            placements
                .add(NewPlacement::new("p1", PlacementKind::Dedicated, url.parse().unwrap()))
                .await
                .unwrap();
            placements.up_all().await.unwrap();
            let migrator = settings.modules.migrator().unwrap().keyed_on(&database);
            let releases = releases(&migrator);
            let main_lock = placements.lock_main(LOCK_WAIT).await.unwrap();
            let run = async |arguments: Arguments| {
                execute(&arguments, &releases, &migrator, &registry, &placements).await
            };

            let dry = run(arguments(true, false, false)).await.unwrap();
            assert!(!dry.failed);
            assert_eq!(
                dry.output.matches(&format!("Revert {NEWEST}\n")).count(),
                2,
                "{}",
                dry.output
            );
            assert!(
                dry.output.contains("== p1 ==") && dry.output.contains("Dry run"),
                "{}",
                dry.output
            );
            // `up_all` đã ghi lượt vào `kernel_migration_runs`: cột `kind` sắp bị bỏ đang có dữ liệu.
            assert!(
                dry.output.contains("Drops column kernel_migration_runs.kind"),
                "{}",
                dry.output
            );
            assert!(migrator.pending(&database).await.unwrap().is_empty(), "chạy thử không đổi gì");

            let refused = run(arguments(false, false, true)).await.err().unwrap();
            assert!(refused.contains("--accept-data-loss"), "{refused}");
            let refused = run(arguments(false, true, false)).await.err().unwrap();
            assert!(refused.contains("--yes"), "{refused}");

            let done = run(arguments(false, true, true)).await.unwrap();
            assert!(!done.failed, "{}", done.output);
            assert!(done.output.contains("Rolled back to 0.0.1"), "{}", done.output);
            let p1 = registry
                .placement_database(PlacementId::from_uuid(
                    placements
                        .list()
                        .await
                        .unwrap()
                        .into_iter()
                        .find(|found| found.name == "p1")
                        .unwrap()
                        .id,
                ))
                .await
                .unwrap();
            for target in [&database, &p1] {
                let pending: Vec<String> = migrator
                    .pending(target)
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|state| format!("{}/{}", state.module, state.name))
                    .collect();
                assert_eq!(pending, [NEWEST]);
            }
            // Đã khớp bản đích: lần sau không còn gì để lùi.
            let again = run(arguments(true, false, false)).await.unwrap();
            assert!(again.output.contains("Nothing to roll back"), "{}", again.output);
            main_lock.release().await.unwrap();
            migrator.up(&database).await.unwrap();
            database.close().await.unwrap();
        })
        .await
        .unwrap();
    }
}
