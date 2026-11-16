//! Lệnh `autoxide tenant` (bước 33): quản lý khách từ máy chủ — tạo, xem, tạm khoá, mở lại, cấp
//! phát lại khách tạo hỏng giữa chừng, dời khách sang placement khác. Chạy với quyền của người vận
//! hành máy chủ (như `autoxide user`), gọi cùng service với REST (`TenantAdmin`, `TenantMoves`).
//! `tenant move` chạy ngay trong lệnh (không qua job nền, không giới hạn thời gian).

use std::process::ExitCode;

use autoxide_kernel::event::Actor;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::pagination::{PageQuery, PageRequest};
use autoxide_kernel::tenancy::{
    NewTenant, Tenant, TenantAdmin, TenantChanges, TenantFilter, TenantMove, TenantMovePlan,
    TenantMoves,
};

use super::{EXIT_FAILURE, report_failure, report_invalid_configuration, run_with_observability};
use crate::runtime::{self, Migrations};
use crate::settings;

#[derive(Debug, clap::Args)]
pub struct Arguments {
    #[command(subcommand)]
    action: Action,
}

#[derive(Debug, Clone, clap::Subcommand)]
enum Action {
    /// Create a tenant and prepare it (needs AUTOXIDE_TENANCY_MODE=multi).
    Create {
        /// Short name: lower-case letters, digits and `-`.
        #[arg(long)]
        slug: String,
        /// Display name.
        #[arg(long)]
        name: String,
        /// Placement (database) for its data; `main` when omitted.
        #[arg(long)]
        placement: Option<String>,
        /// Parent tenant (id), for chains.
        #[arg(long)]
        parent: Option<uuid::Uuid>,
    },
    /// List the tenants, newest first.
    List,
    /// Show one tenant (id or slug).
    Show { tenant: String },
    /// Suspend a tenant: every request to it answers 403 until it is resumed.
    Suspend { tenant: String },
    /// Resume a suspended tenant.
    Resume { tenant: String },
    /// Make a tenant read-only: its data can still be read and exported and its users can sign
    /// in, but every change answers 403 until `writable`.
    ReadOnly { tenant: String },
    /// Let a read-only tenant change its data again.
    Writable { tenant: String },
    /// Prepare again a tenant whose creation stopped half-way (safe to run again).
    Provision { tenant: String },
    /// Move a tenant's data to another placement (database). The tenant answers 503 during the
    /// move. Try `--dry-run` first; run again to resume a move that stopped half-way.
    Move {
        /// Id or slug of the tenant.
        tenant: String,
        /// Name of the placement to move the data to.
        #[arg(long, required_unless_present = "abort")]
        to: Option<String>,
        /// Keep the old copy of the data as a backup; delete it later with `purge-copy`.
        #[arg(long)]
        keep_source: bool,
        /// Only show what the move would do and what blocks it; change nothing.
        #[arg(long, conflicts_with = "abort")]
        dry_run: bool,
        /// Abort a move that stopped before the tenant switched: the copied rows are deleted and
        /// the tenant goes back to its old placement.
        #[arg(long, conflicts_with_all = ["to", "keep_source"])]
        abort: bool,
    },
    /// Delete a copy of a tenant's data left on a placement it no longer uses (by
    /// `move --keep-source`, or an aborted move).
    PurgeCopy {
        /// Id or slug of the tenant.
        tenant: String,
        /// Placement that holds the old copy.
        #[arg(long)]
        placement: String,
    },
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    let action = arguments.action.clone();
    let outcome = run_with_observability(&config, async move {
        let runtime = runtime::start(settings, Migrations::RequireApplied).await?;
        let services =
            runtime.modules.api::<TenantAdmin>().zip(runtime.modules.api::<TenantMoves>());
        let outcome = match services {
            Some((admin, moves)) => {
                execute(&admin, &moves, action).await.map_err(|error| super::describe_error(&error))
            }
            None => Err("the kernel did not start its tenant services".to_owned()),
        };
        runtime.database.close().await.map_err(|error| format!("database: {error}"))?;
        Ok(outcome)
    });
    match outcome {
        Ok(Ok(report)) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Ok(Err(message)) => {
            eprintln!("autoxide tenant: {message}");
            ExitCode::from(EXIT_FAILURE)
        }
        Err(error) => report_failure(&error),
    }
}

async fn execute(
    admin: &TenantAdmin,
    moves: &TenantMoves,
    action: Action,
) -> Result<String, autoxide_kernel::error::Error> {
    match action {
        Action::Move { tenant, abort: true, .. } => {
            let aborted = moves.abort(&tenant).await?;
            let mut report = format!(
                "Move {} aborted: the tenant is active again on `{}`.",
                aborted.id, aborted.from
            );
            if let Some(error) = &aborted.error {
                report.push_str(&format!(
                    "\nThe copied rows on `{}` could not be deleted ({error}); delete them with \
                     `autoxide tenant purge-copy {tenant} --placement {}`.",
                    aborted.to, aborted.to
                ));
            }
            Ok(report)
        }
        Action::Move { tenant, to, keep_source, dry_run, .. } => {
            let to = to.unwrap_or_default();
            if dry_run {
                return Ok(describe_plan(
                    &moves.plan(&tenant, &to, keep_source, Language::En).await?,
                ));
            }
            let started = moves.start(Actor::system(), &tenant, &to, keep_source, false).await?;
            if started.state != "copying" || started.error.is_some() {
                eprintln!("Resuming move {} (state `{}`).", started.id, started.state);
            }
            let runner = format!("cli-{}", uuid::Uuid::now_v7());
            let mut state = started.state.clone();
            eprintln!("{}", progress(&state));
            loop {
                let moved = moves.step(started.id, &runner).await?;
                if moved.state != state {
                    state.clone_from(&moved.state);
                    eprintln!("{}", progress(&state));
                }
                if moved.state == "done" || moved.state == "aborted" {
                    return Ok(describe_move(&tenant, &moved));
                }
            }
        }
        Action::PurgeCopy { tenant, placement } => {
            let deleted = moves.purge(&tenant, &placement).await?;
            let total: u64 = deleted.iter().map(|table| table.rows).sum();
            let mut lines: Vec<String> = deleted
                .iter()
                .map(|table| format!("{:>10}  {}", table.rows, table.table))
                .collect();
            lines.push(format!(
                "Deleted {total} rows of tenant `{tenant}` from placement `{placement}`."
            ));
            Ok(lines.join("\n"))
        }
        Action::Create { slug, name, placement, parent } => {
            let input = NewTenant { slug, name, parent_id: parent, placement };
            Ok(line(&admin.create(Actor::system(), input).await?))
        }
        Action::List => {
            let mut lines = Vec::new();
            let mut cursor = None;
            loop {
                let query = PageQuery::new(Some("200".to_owned()), cursor, None);
                let page =
                    admin.list(&TenantFilter::default(), &PageRequest::try_from(query)?).await?;
                lines.extend(page.data.iter().map(line));
                cursor = page.pagination.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
            Ok(lines.join("\n"))
        }
        Action::Show { tenant } => Ok(line(&admin.get(&tenant).await?)),
        Action::Suspend { tenant } => set_status(admin, &tenant, "suspended").await,
        Action::Resume { tenant } => set_status(admin, &tenant, "active").await,
        Action::ReadOnly { tenant } => set_read_only(admin, &tenant, true).await,
        Action::Writable { tenant } => set_read_only(admin, &tenant, false).await,
        Action::Provision { tenant } => Ok(line(&admin.provision(Actor::system(), &tenant).await?)),
    }
}

async fn set_status(
    admin: &TenantAdmin,
    tenant: &str,
    status: &str,
) -> Result<String, autoxide_kernel::error::Error> {
    let found = admin.get(tenant).await?;
    let changes = TenantChanges { status: Some(status.to_owned()), ..TenantChanges::default() };
    Ok(line(&admin.update(Actor::system(), found.id, changes, None).await?))
}

async fn set_read_only(
    admin: &TenantAdmin,
    tenant: &str,
    read_only: bool,
) -> Result<String, autoxide_kernel::error::Error> {
    let found = admin.get(tenant).await?;
    let changes = TenantChanges { read_only: Some(read_only), ..TenantChanges::default() };
    Ok(line(&admin.update(Actor::system(), found.id, changes, None).await?))
}

/// Câu cho người chạy lệnh khi lượt dời sang trạng thái `state`.
fn progress(state: &str) -> &'static str {
    match state {
        "copying" => {
            "The tenant answers 503 now. Waiting for other servers and running jobs, then copying…"
        }
        "copied" => "Copied and counted. Switching the tenant to its new placement…",
        "switched" => "The tenant works from its new placement again. Cleaning the old one…",
        _ => "Done.",
    }
}

/// Kế hoạch dời, cho người đọc.
fn describe_plan(plan: &TenantMovePlan) -> String {
    let mut lines = vec![format!(
        "Move `{}` from placement `{}` to `{}`{}:",
        plan.tenant,
        plan.from,
        plan.to,
        if plan.keep_source { ", keeping the old copy" } else { "" }
    )];
    lines.extend(plan.tables.iter().map(|table| format!("{:>10}  {}", table.rows, table.table)));
    lines.push(format!(
        "{} rows in {} tables would be copied. The tenant answers 503 for at least {} seconds, \
         then while copying.",
        plan.rows,
        plan.tables.len(),
        plan.settle_seconds
    ));
    if plan.files > 0 {
        lines.push(format!(
            "{} files would be copied to the file storage of `{}`.",
            plan.files, plan.to
        ));
    }
    if let Some(id) = plan.resumes {
        lines.push(format!("Running it resumes the stopped move {id}."));
    }
    if plan.blocking.is_empty() {
        lines.push("Nothing blocks the move: run the same command without `--dry-run`.".to_owned());
    } else {
        lines.push("Blocked:".to_owned());
        lines.extend(
            plan.blocking.iter().map(|blocker| format!("  {}: {}", blocker.code, blocker.message)),
        );
    }
    lines.join("\n")
}

/// Lượt dời đã xong, cho người đọc.
fn describe_move(tenant: &str, moved: &TenantMove) -> String {
    let mut report = format!(
        "Moved `{tenant}` from `{}` to `{}`: {} rows in {} tables.",
        moved.from,
        moved.to,
        moved.rows,
        moved.tables.len()
    );
    if moved.keep_source {
        report.push_str(&format!(
            " The old copy stays on `{}`; delete it with `autoxide tenant purge-copy {tenant} \
             --placement {}`.",
            moved.from, moved.from
        ));
    } else {
        report.push_str(&format!(" The old copy on `{}` was deleted.", moved.from));
    }
    report
}

/// Một dòng mô tả khách.
fn line(tenant: &Tenant) -> String {
    format!(
        "{}  {}  {}{}  placement={}  \"{}\"",
        tenant.id,
        tenant.slug,
        tenant.status,
        if tenant.read_only { " (read-only)" } else { "" },
        tenant.placement,
        tenant.name
    )
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::config::ConfigSource;
    use autoxide_kernel::database::Database;
    use autoxide_testkit::TestDatabase;

    use super::*;

    #[tokio::test]
    async fn liet_ke_va_tu_choi_tao_khach_o_che_do_mot_khach() {
        TestDatabase::run(async |test| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let settings = crate::settings::read(&source).unwrap();
            let database = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&database).await.unwrap();
            database.close().await.unwrap();
            let runtime = runtime::start(settings, Migrations::RequireApplied).await.unwrap();
            let admin = runtime.modules.api::<TenantAdmin>().unwrap();
            let moves = runtime.modules.api::<TenantMoves>().unwrap();
            let execute = async |action| execute(&admin, &moves, action).await;
            let listed = execute(Action::List).await.unwrap();
            assert!(listed.contains("default  active  placement=main"), "{listed}");
            let shown = execute(Action::Show { tenant: "default".to_owned() }).await.unwrap();
            assert_eq!(shown, listed);
            let create = Action::Create {
                slug: "acme".to_owned(),
                name: "Acme".to_owned(),
                placement: None,
                parent: None,
            };
            assert_eq!(execute(create).await.unwrap_err().code(), "tenant_single_mode");
            let refused = execute(Action::Suspend { tenant: "default".to_owned() }).await;
            assert_eq!(refused.unwrap_err().code(), "validation_failed");
            runtime.database.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    /// `tenant move`: dry-run in kế hoạch không đổi gì; dời thật chạy ngay tới xong; giữ bản cũ rồi
    /// xoá bằng `purge-copy`; huỷ khi không có lượt dời nào thì báo lỗi rõ.
    #[tokio::test]
    async fn doi_khach_bang_lenh() {
        TestDatabase::run(async |test| {
            let source = ConfigSource::from_pairs([
                ("DATABASE_URL", test.url()),
                ("AUTOXIDE_TENANCY_MODE", "multi"),
                ("AUTOXIDE_SECRET_KEY", "a-configured-secret-that-is-long-enough-0123"),
                ("AUTOXIDE_TENANCY_CACHE_SECONDS", "1"),
                ("AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS", "1"),
            ]);
            let settings = crate::settings::read(&source).unwrap();
            let database = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&database).await.unwrap();
            database.close().await.unwrap();
            let runtime = runtime::start(settings, Migrations::RequireApplied).await.unwrap();
            let registry = runtime.modules.tenancy().unwrap();
            let placements = runtime.modules.placements(&registry).unwrap();
            let url = test.extra_database().await.unwrap();
            let new = autoxide_kernel::tenancy::NewPlacement::new(
                "spare",
                autoxide_kernel::tenancy::PlacementKind::Dedicated,
                url.parse().unwrap(),
            );
            placements.add(new).await.unwrap();
            placements.up_all().await.unwrap();
            let admin = runtime.modules.api::<TenantAdmin>().unwrap();
            let moves = runtime.modules.api::<TenantMoves>().unwrap();
            let execute = async |action| execute(&admin, &moves, action).await;
            let create = Action::Create {
                slug: "acme".to_owned(),
                name: "Acme".to_owned(),
                placement: None,
                parent: None,
            };
            execute(create).await.unwrap();
            let action = |to: &str, keep_source: bool, dry_run: bool| Action::Move {
                tenant: "acme".to_owned(),
                to: Some(to.to_owned()),
                keep_source,
                dry_run,
                abort: false,
            };

            let plan = execute(action("spare", false, true)).await.unwrap();
            assert!(plan.contains("Move `acme` from placement `main` to `spare`"), "{plan}");
            assert!(plan.contains("Nothing blocks the move"), "{plan}");
            let shown = execute(Action::Show { tenant: "acme".to_owned() }).await.unwrap();
            assert!(shown.contains("acme  active  placement=main"), "{shown}");

            let moved = execute(action("spare", false, false)).await.unwrap();
            assert!(moved.starts_with("Moved `acme` from `main` to `spare`"), "{moved}");
            assert!(moved.contains("was deleted"), "{moved}");
            let shown = execute(Action::Show { tenant: "acme".to_owned() }).await.unwrap();
            assert!(shown.contains("acme  active  placement=spare"), "{shown}");

            let back = execute(action("main", true, false)).await.unwrap();
            assert!(back.contains("purge-copy acme --placement spare"), "{back}");
            let purge =
                Action::PurgeCopy { tenant: "acme".to_owned(), placement: "spare".to_owned() };
            let purged = execute(purge).await.unwrap();
            assert!(purged.contains("rows of tenant `acme` from placement `spare`"), "{purged}");
            let blocked = execute(action("main", false, true)).await.unwrap();
            assert!(blocked.contains("tenant_move_same_placement"), "{blocked}");

            let abort = Action::Move {
                tenant: "acme".to_owned(),
                to: None,
                keep_source: false,
                dry_run: false,
                abort: true,
            };
            assert_eq!(execute(abort).await.unwrap_err().code(), "tenant_move_not_found");
            runtime.database.close().await.unwrap();
        })
        .await
        .unwrap();
    }
}
