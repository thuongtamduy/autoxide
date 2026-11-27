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
use std::time::Duration;

use autoxide_kernel::migration::{DryRunLimits, MigrationLock, Migrator, RollbackPlan};
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
    /// how many rows they hold. Changes nothing. On PostgreSQL it runs the reverts inside a transaction that is
    /// rolled back, holding locks on the tables it changes for that time (see `--lock-timeout-seconds`).
    #[arg(long)]
    dry_run: bool,
    /// Go on although the rollback drops tables or columns that hold data (see `--dry-run`).
    #[arg(long)]
    accept_data_loss: bool,
    /// Confirm that every server and worker of the current release is stopped.
    #[arg(long)]
    yes: bool,
    /// Also revert the migrations of modules the release table does not track (modules this binary adds with
    /// `.module(…)`): without it they are left as they are.
    #[arg(long)]
    revert_untracked: bool,
    /// PostgreSQL: how long the dry run waits for a table another session uses before it gives up (at least 1:
    /// on PostgreSQL 0 would mean waiting forever).
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u64).range(1..))]
    lock_timeout_seconds: u64,
    /// PostgreSQL: longest run of one statement of the dry run (at least 1).
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..))]
    statement_timeout_seconds: u64,
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
        let mut main_lock = MigrationLock::acquire(&settings.config.database, LOCK_WAIT)
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
                let context = Context {
                    releases: &releases,
                    migrator: &migrator,
                    registry: &registry,
                    placements: &placements,
                };
                execute(&arguments, &context, &mut main_lock).await
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

/// Những thứ lệnh dùng (gom lại để hàm không nhận quá nhiều tham số).
pub(crate) struct Context<'a> {
    pub(crate) releases: &'a MigrationReleases,
    pub(crate) migrator: &'a Migrator,
    pub(crate) registry: &'a DatabaseRegistry,
    pub(crate) placements: &'a Placements,
}

/// Migration đầu tiên của phần đa khách: bản đích chưa có nó thì không biết placement là gì.
const TENANCY: (&str, &str) = ("kernel", "m0003_create_tenancy");
/// Migration thêm cột `kind` của `kernel_migration_runs`: bản đích còn giữ nó thì ghi kết thúc lượt sau cùng được.
const RUN_KIND: (&str, &str) = ("kernel", "m0043_migration_run_kind");

/// Mọi việc sau khi đã giữ khoá chính (`main_lock`): kiểm bản đích, lấy khoá placement, lập kế hoạch, lùi.
pub(crate) async fn execute(
    arguments: &Arguments,
    context: &Context<'_>,
    main_lock: &mut MigrationLock,
) -> Result<Outcome, String> {
    let current = env!("CARGO_PKG_VERSION");
    let target = arguments.to.trim_start_matches('v');
    if !context.releases.knows(target) {
        return Err(format!(
            "`{target}` is not a release this binary knows; known releases: {}",
            context.releases.releases().join(", ")
        ));
    }
    if compare(target, current) == std::cmp::Ordering::Greater {
        return Err(format!(
            "`{target}` is newer than this binary ({current}); run the binary of `{target}` instead"
        ));
    }
    let describe = |error: autoxide_kernel::error::Error| super::describe_error(&error);
    let mut lock = context.placements.lock(LOCK_WAIT).await.map_err(describe)?;
    let outcome = rollback_locked(arguments, context, target, &mut lock, main_lock).await;
    if let Err(error) = lock.release().await {
        tracing::warn!(error = %error, "cannot release the migration lock");
    }
    outcome
}

async fn rollback_locked(
    arguments: &Arguments,
    context: &Context<'_>,
    target: &str,
    lock: &mut MigrationLock,
    main_lock: &mut MigrationLock,
) -> Result<Outcome, String> {
    let describe = |error: autoxide_kernel::error::Error| super::describe_error(&error);
    let releases = context.releases;
    let keep = |module: &str, name: &str| {
        (!releases.tracks_module(module) && !arguments.revert_untracked)
            || releases.in_release(target, module, name)
    };
    // Placement trước (theo tên), database chính sau cùng.
    let mut placements: Vec<(String, Database)> = Vec::new();
    for found in context.placements.list().await.map_err(describe)? {
        if found.id == MAIN_PLACEMENT.as_uuid() {
            continue;
        }
        let database = context
            .registry
            .placement_database(PlacementId::from_uuid(found.id))
            .await
            .map_err(describe)?;
        placements.push((found.name, database));
    }
    if !placements.is_empty()
        && releases.tracks_module(TENANCY.0)
        && !releases.in_release(target, TENANCY.0, TENANCY.1)
    {
        return Err(format!(
            "`{target}` has no tenant databases (placements), and this installation has some; it cannot go back that far"
        ));
    }
    let main = context.registry.control().clone();
    let mut limits = DryRunLimits::new(
        Duration::from_secs(arguments.lock_timeout_seconds),
        Duration::from_secs(arguments.statement_timeout_seconds),
    );
    if arguments.yes && !arguments.dry_run {
        // Máy chủ đã dừng (`--yes`): SQLite đo thẳng, không chép lại database.
        limits = limits.with_servers_stopped();
    }

    let mut plans: Vec<(String, RollbackPlan)> = Vec::new();
    for (name, database) in
        placements.iter().chain(std::iter::once(&("main".to_owned(), main.clone())))
    {
        let plan = context
            .migrator
            .rollback_plan(database, &keep, limits)
            .await
            .map_err(|error| format!("{name}: {error}"))?;
        plans.push((name.clone(), plan));
    }
    let mut output = render_plans(target, &plans);
    let untracked = untracked_modules(context.migrator, releases);
    if !untracked.is_empty() {
        let verb = if arguments.revert_untracked {
            "reverted too (--revert-untracked)"
        } else {
            "left as is"
        };
        output.push_str(&format!(
            "Modules not tracked by the release table, {verb}: {}\n",
            untracked.join(", ")
        ));
    }
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
    let run =
        context.placements.begin_rollback_run(lock, current, target).await.map_err(describe)?;
    let mut failures = Vec::new();
    for (name, database) in &placements {
        if let Err(error) = lock.verify().await {
            failures.push(error.to_string());
            break;
        }
        // `migrating` trong lúc lùi: khách nhận 503 thay vì lỗi SQL; tiến trình chết thì lượt sau đặt `behind`.
        context.placements.mark_rollback(lock, name, false).await.map_err(describe)?;
        let reverted = context.migrator.rollback(database, &keep).await;
        context.placements.mark_rollback(lock, name, true).await.map_err(describe)?;
        match reverted {
            Ok(reverted) => output.push_str(&format!("{name}: reverted {}\n", reverted.len())),
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    // Bản đích còn cột `kind` của bảng lượt thì ghi kết thúc lượt sau khi lùi database chính; không thì phải ghi
    // TRƯỚC (lùi database chính bỏ chính các cột đó), và lỗi của database chính chỉ in ra, không vào lượt.
    let finish_last = releases.in_release(target, RUN_KIND.0, RUN_KIND.1);
    if !failures.is_empty() {
        let error = failures.join("\n");
        context.placements.finish_run(run, Some(error.clone())).await.map_err(describe)?;
        output.push_str(&format!(
            "{error}\nThe main database was NOT rolled back: fix the failure and run again (what is reverted \
             stays reverted).\n"
        ));
        return Ok(Outcome { output, failed: true });
    }
    if !finish_last {
        context.placements.finish_run(run, None).await.map_err(describe)?;
    }
    let main_outcome = match main_lock.verify().await {
        Ok(()) => context.migrator.rollback(&main, &keep).await.map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };
    if finish_last {
        let error = main_outcome.as_ref().err().map(|error| format!("main: {error}"));
        context.placements.finish_run(run, error).await.map_err(describe)?;
    }
    match main_outcome {
        Ok(reverted) => output.push_str(&format!("main: reverted {}\n", reverted.len())),
        Err(error) => {
            output.push_str(&format!("main: {error}\n"));
            return Ok(Outcome { output, failed: true });
        }
    }
    output.push_str(&format!("Rolled back to {target}: start the servers of {target} now.\n"));
    Ok(Outcome { output, failed: false })
}

/// Module của binary mà bảng phiên bản không ghi (module ghép từ ngoài).
fn untracked_modules(migrator: &Migrator, releases: &MigrationReleases) -> Vec<String> {
    let mut modules: Vec<String> = migrator
        .names()
        .into_iter()
        .filter_map(|name| name.split_once('/').map(|(module, _)| module.to_owned()))
        .filter(|module| !releases.tracks_module(module))
        .collect();
    modules.dedup();
    modules
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
            let line = match (&lost.column, lost.dropped) {
                (None, true) => format!("Drops table {} ({} row(s))\n", lost.table, lost.rows),
                (None, false) => format!("Deletes {} row(s) of table {}\n", lost.rows, lost.table),
                (Some(column), _) => {
                    format!("Drops column {}.{column} ({} value(s))\n", lost.table, lost.rows)
                }
            };
            output.push_str(&line);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    use autoxide_kernel::config::ConfigSource;
    use autoxide_kernel::error::Error;
    use autoxide_kernel::migration::prelude::*;
    use autoxide_kernel::migration::{DataUpgrade, ModuleMigrations, UpgradeContext};
    use autoxide_kernel::tenancy::{NewPlacement, PlacementKind, TenancyConfig, TenancyMode};
    use autoxide_testkit::TestDatabase;

    const NEWEST: &str = "kernel/m0043_migration_run_kind";

    fn arguments(dry_run: bool, accept: bool, yes: bool) -> Arguments {
        Arguments {
            to: "0.0.1".to_owned(),
            dry_run,
            accept_data_loss: accept,
            yes,
            revert_untracked: false,
            lock_timeout_seconds: 2,
            statement_timeout_seconds: 60,
        }
    }

    /// Bảng phiên bản giả: bản `0.0.1` có mọi tên trong `names` trừ `newer`.
    fn releases(names: Vec<String>, newer: &[&str]) -> MigrationReleases {
        let introduced: serde_json::Map<String, serde_json::Value> = names
            .into_iter()
            .map(|name| {
                let first = if newer.contains(&name.as_str()) { "0.0.2" } else { "0.0.1" };
                (name, serde_json::Value::from(first))
            })
            .collect();
        let table = serde_json::json!({"releases": ["0.0.1", "0.0.2"], "introduced": introduced});
        MigrationReleases::parse(&table.to_string())
    }

    struct Setup {
        database: Database,
        registry: DatabaseRegistry,
        placements: Placements,
        p1: Database,
    }

    /// Database chính đủ migration, chế độ nhiều khách, một placement `p1`.
    async fn setup(test: &TestDatabase) -> Setup {
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
        let new = NewPlacement::new("p1", PlacementKind::Dedicated, url.parse().unwrap());
        placements.add(new).await.unwrap();
        placements.up_all().await.unwrap();
        let id = placements.list().await.unwrap().into_iter().find(|p| p.name == "p1").unwrap().id;
        let p1 = registry.placement_database(PlacementId::from_uuid(id)).await.unwrap();
        Setup { database, registry, placements, p1 }
    }

    async fn pending(migrator: &Migrator, database: &Database) -> Vec<String> {
        migrator
            .pending(database)
            .await
            .unwrap()
            .into_iter()
            .map(|state| format!("{}/{}", state.module, state.name))
            .collect()
    }

    /// Lùi database chính và placement về bản trước: chạy thử không đổi gì; cột chỉ toàn giá trị mặc định không
    /// tính là mất dữ liệu; thiếu `--yes` thì từ chối kèm kế hoạch; chạy thật gỡ đúng migration mới ở mọi database.
    #[tokio::test]
    async fn lui_database_chinh_va_placement_ve_ban_truoc() {
        TestDatabase::run(async |test| {
            let setup = setup(test).await;
            let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let settings = crate::settings::read(&source).unwrap();
            let migrator = settings.modules.migrator().unwrap().keyed_on(&setup.database);
            let releases = releases(migrator.names(), &[NEWEST]);
            let context = Context {
                releases: &releases,
                migrator: &migrator,
                registry: &setup.registry,
                placements: &setup.placements,
            };
            let mut main_lock = setup.placements.lock_main(LOCK_WAIT).await.unwrap();

            let dry =
                execute(&arguments(true, false, false), &context, &mut main_lock).await.unwrap();
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
            // `kind` chỉ có giá trị mặc định (`placements`), hai cột phiên bản toàn NULL: lùi không mất gì.
            assert!(!dry.output.contains("Drops"), "{}", dry.output);
            assert!(
                migrator.pending(&setup.database).await.unwrap().is_empty(),
                "chạy thử không đổi gì"
            );

            let refused = execute(&arguments(false, false, false), &context, &mut main_lock)
                .await
                .err()
                .unwrap();
            assert!(refused.contains("--yes"), "{refused}");

            let done =
                execute(&arguments(false, false, true), &context, &mut main_lock).await.unwrap();
            assert!(
                !done.failed && done.output.contains("Rolled back to 0.0.1"),
                "{}",
                done.output
            );
            for target in [&setup.database, &setup.p1] {
                assert_eq!(pending(&migrator, target).await, [NEWEST]);
            }
            let again =
                execute(&arguments(true, false, false), &context, &mut main_lock).await.unwrap();
            assert!(again.output.contains("Nothing to roll back"), "{}", again.output);
            main_lock.release().await.unwrap();
            migrator.up(&setup.database).await.unwrap();
            setup.database.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    /// `--lock-timeout-seconds 0` bị từ chối ngay khi đọc tham số (trên Postgres 0 là chờ mãi).
    #[test]
    fn lock_timeout_bang_khong_bi_tu_choi() {
        #[derive(clap::Parser)]
        struct Cli {
            #[command(flatten)]
            arguments: Arguments,
        }
        let parse = |value: &str| {
            <Cli as clap::Parser>::try_parse_from([
                "rollback",
                "--to",
                "0.19.0",
                "--lock-timeout-seconds",
                value,
            ])
        };
        assert!(parse("0").is_err());
        assert_eq!(parse("5").unwrap().arguments.lock_timeout_seconds, 5);
    }

    /// Lần đầu `down` của data upgrade hỏng (giả lập lỗi giữa chừng), lần sau được.
    static FAIL_DOWN: AtomicBool = AtomicBool::new(true);

    struct Flaky;

    #[async_trait::async_trait]
    impl DataUpgrade for Flaky {
        fn name(&self) -> &'static str {
            "u0001_fill"
        }

        fn after(&self) -> &'static str {
            "m0002_create_extra"
        }

        async fn up(&self, _context: &UpgradeContext) -> Result<(), Error> {
            Ok(())
        }

        async fn down(&self, _context: &UpgradeContext) -> Result<(), Error> {
            if FAIL_DOWN.swap(false, Ordering::SeqCst) {
                return Err(Error::internal("deliberate failure"));
            }
            Ok(())
        }
    }

    struct CreateTable(&'static str, &'static str);

    impl MigrationName for CreateTable {
        fn name(&self) -> &str {
            self.0
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for CreateTable {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(Table::create().table(self.1).col(schema::pk_uuid("id")).to_owned())
                .await
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager.drop_table(Table::drop().table(self.1).to_owned()).await
        }
    }

    fn flaky() -> ModuleMigrations {
        ModuleMigrations::new(
            "flaky",
            vec![
                Box::new(CreateTable("m0001_create_base", "flaky_base")),
                Box::new(CreateTable("m0002_create_extra", "flaky_extra")),
            ],
        )
        .with_upgrades(vec![Box::new(Flaky)])
    }

    /// Placement hỏng giữa chừng (data upgrade lùi lỗi): database chính không bị đụng, lượt ghi `failed`; chạy lại
    /// làm tiếp tới xong. Module không có trong bảng phiên bản (ở đây là mọi module thật) để nguyên, trừ khi có
    /// `--revert-untracked`.
    #[tokio::test]
    async fn placement_hong_thi_khong_lui_database_chinh_roi_chay_lai_duoc() {
        TestDatabase::run(async |test| {
            let setup = setup(test).await;
            let outside = ModuleMigrations::new(
                "outside",
                vec![Box::new(CreateTable("m0001_create_thing", "outside_thing"))],
            );
            let migrator = Migrator::new(vec![flaky(), outside]).unwrap().keyed_on(&setup.database);
            migrator.up(&setup.database).await.unwrap();
            migrator.up(&setup.p1).await.unwrap();
            // `outside` (như module ghép từ ngoài) không có trong bảng phiên bản.
            let flaky_names: Vec<String> =
                migrator.names().into_iter().filter(|name| name.starts_with("flaky/")).collect();
            let releases = releases(flaky_names, &["flaky/m0002_create_extra", "flaky/u0001_fill"]);
            let context = Context {
                releases: &releases,
                migrator: &migrator,
                registry: &setup.registry,
                placements: &setup.placements,
            };
            let mut main_lock = setup.placements.lock_main(LOCK_WAIT).await.unwrap();
            FAIL_DOWN.store(true, Ordering::SeqCst);

            let dry =
                execute(&arguments(true, false, false), &context, &mut main_lock).await.unwrap();
            assert!(
                dry.output.contains("Revert flaky/u0001_fill (data upgrade)"),
                "{}",
                dry.output
            );
            assert!(
                dry.output.contains("not tracked by the release table, left as is: outside"),
                "{}",
                dry.output
            );
            assert!(!dry.output.contains("Revert outside/"), "{}", dry.output);
            let mut all = arguments(true, false, false);
            all.revert_untracked = true;
            let dry_all = execute(&all, &context, &mut main_lock).await.unwrap();
            assert!(
                dry_all.output.contains("Revert outside/m0001_create_thing"),
                "{}",
                dry_all.output
            );

            let first =
                execute(&arguments(false, false, true), &context, &mut main_lock).await.unwrap();
            assert!(first.failed, "{}", first.output);
            assert!(first.output.contains("main database was NOT rolled back"), "{}", first.output);
            assert!(
                pending(&migrator, &setup.database).await.is_empty(),
                "database chính không bị đụng"
            );
            // p1 đã lùi được một phần, đang `behind` (khách nhận 503 tới khi xong).
            let p1 = setup
                .placements
                .list()
                .await
                .unwrap()
                .into_iter()
                .find(|p| p.name == "p1")
                .unwrap();
            assert_eq!(p1.schema_state, "behind");

            let second =
                execute(&arguments(false, false, true), &context, &mut main_lock).await.unwrap();
            assert!(!second.failed, "{}", second.output);
            for target in [&setup.database, &setup.p1] {
                assert_eq!(
                    pending(&migrator, target).await,
                    ["flaky/m0002_create_extra", "flaky/u0001_fill"]
                );
            }
            // Hai lượt `rollback`: lần đầu `failed` (nêu p1), lần sau `done`.
            let select = sea_orm::sea_query::Query::select()
                .columns(["status", "error"].map(sea_orm::sea_query::Alias::new))
                .from(sea_orm::sea_query::Alias::new("kernel_migration_runs"))
                .and_where(
                    sea_orm::sea_query::Expr::col(sea_orm::sea_query::Alias::new("kind"))
                        .eq("rollback"),
                )
                .order_by(sea_orm::sea_query::Alias::new("id"), sea_orm::sea_query::Order::Asc)
                .to_owned();
            let rows =
                sea_orm::ConnectionTrait::query_all(setup.database.read(), &select).await.unwrap();
            let runs: Vec<(String, Option<String>)> = rows
                .into_iter()
                .map(|row| (row.try_get("", "status").unwrap(), row.try_get("", "error").unwrap()))
                .collect();
            assert_eq!(runs.len(), 2, "{runs:?}");
            assert_eq!(runs[0].0, "failed");
            assert!(runs[0].1.as_deref().unwrap_or("").starts_with("p1: "), "{runs:?}");
            assert_eq!((runs[1].0.as_str(), runs[1].1.as_deref()), ("done", None));
            main_lock.release().await.unwrap();
            setup.database.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    /// `down` hỏng ngay trong chạy thử (bảng của migration đã bị bỏ tay): lệnh từ chối, không đổi gì.
    #[tokio::test]
    async fn down_hong_trong_chay_thu_thi_tu_choi_sach() {
        TestDatabase::run(async |test| {
            let setup = setup(test).await;
            let migrator = Migrator::new(vec![flaky()]).unwrap().keyed_on(&setup.database);
            migrator.up(&setup.database).await.unwrap();
            let releases =
                releases(migrator.names(), &["flaky/m0002_create_extra", "flaky/u0001_fill"]);
            let context = Context {
                releases: &releases,
                migrator: &migrator,
                registry: &setup.registry,
                placements: &setup.placements,
            };
            SchemaManager::new(setup.database.write())
                .drop_table(Table::drop().table("flaky_extra").to_owned())
                .await
                .unwrap();
            let mut main_lock = setup.placements.lock_main(LOCK_WAIT).await.unwrap();
            let error = execute(&arguments(true, false, false), &context, &mut main_lock)
                .await
                .err()
                .unwrap();
            assert!(error.starts_with("main: ") && error.contains("cannot measure"), "{error}");
            // SQLite: bản sao của chạy thử đã bị xoá dù `down` hỏng giữa chừng.
            if let Some(rest) = test.url().strip_prefix("sqlite://") {
                let file = std::path::Path::new(rest.split('?').next().unwrap());
                let left = std::fs::read_dir(file.parent().unwrap())
                    .unwrap()
                    .filter(|entry| {
                        entry
                            .as_ref()
                            .unwrap()
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".autoxide-rollback-")
                    })
                    .count();
                assert_eq!(left, 0);
            }
            assert!(pending(&migrator, &setup.database).await.is_empty(), "không đổi gì");
            main_lock.release().await.unwrap();
            setup.database.close().await.unwrap();
        })
        .await
        .unwrap();
    }
}
