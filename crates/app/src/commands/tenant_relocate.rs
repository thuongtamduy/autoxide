//! `autoxide tenant split|merge|relocation-status` (bước 38): tách một nhánh tổ chức thành khách
//! riêng, gộp khách vào khách khác, xem một lượt — chạy ngay trong lệnh (không qua job nền), gọi
//! cùng service với REST (`TenantRelocations`). `--dry-run` chỉ in kế hoạch; `--resume <id>` chạy
//! tiếp một lượt đã dừng giữa chừng.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::tenancy::{
    RelocationInput, RelocationKind, RelocationPlan, TenantRelocation, TenantRelocations,
};
use uuid::Uuid;

/// Cảnh báo pháp lý in trước khi chạy thật (Luật 91/2025/QH15: chuyển dữ liệu cá nhân giữa hai
/// bên kiểm soát dữ liệu cần căn cứ).
const NOTICE: &str = "Moving data between two tenants can transfer personal data between two \
                      controllers: make sure you have a legal basis (contract, consent). The reason \
                      is kept with the run and in the audit log.";

/// Lập kế hoạch (`dry_run`) hay chạy lượt `input` tới xong; `resume`: chạy tiếp lượt đã có.
pub(super) async fn relocate(
    service: &TenantRelocations,
    input: Option<RelocationInput>,
    dry_run: bool,
    resume: Option<Uuid>,
) -> Result<String, Error> {
    let id = match (resume, input) {
        (Some(id), _) => {
            eprintln!("Resuming split / merge {id}.");
            id
        }
        (None, Some(input)) if dry_run => {
            return Ok(describe_plan(&service.plan(&input, Language::En).await?));
        }
        (None, Some(input)) => {
            eprintln!("{NOTICE}");
            service.start(Actor::system(), &input, None, false).await?.id
        }
        (None, None) => return Err(Error::internal("nothing to run")),
    };
    let runner = format!("cli-{}", Uuid::now_v7());
    let mut state = String::new();
    loop {
        let step = service.step(id, &runner).await?;
        if step.state != state {
            state.clone_from(&step.state);
            eprintln!("State: {state}");
        }
        if step.state == "done" || step.state == "failed" {
            return Ok(describe(&step));
        }
    }
}

/// Lượt `id`, cho người đọc.
pub(super) async fn status(service: &TenantRelocations, id: Uuid) -> Result<String, Error> {
    Ok(describe(&service.get(id).await?))
}

/// Đầu vào của `tenant split`.
#[allow(clippy::too_many_arguments)]
pub(super) fn split_input(
    tenant: String,
    org: Option<Uuid>,
    slug: Option<String>,
    name: Option<String>,
    placement: Option<String>,
    no_keep_parent: bool,
    reason: Option<String>,
) -> RelocationInput {
    RelocationInput {
        kind: RelocationKind::Split,
        tenant,
        org_id: org,
        into: None,
        under_org: None,
        slug,
        name,
        placement,
        keep_parent: !no_keep_parent,
        reason: reason.unwrap_or_default(),
    }
}

/// Đầu vào của `tenant merge`.
pub(super) fn merge_input(
    tenant: String,
    into: Option<String>,
    under_org: Option<Uuid>,
    reason: Option<String>,
) -> RelocationInput {
    RelocationInput {
        kind: RelocationKind::Merge,
        tenant,
        org_id: None,
        into,
        under_org,
        slug: None,
        name: None,
        placement: None,
        keep_parent: true,
        reason: reason.unwrap_or_default(),
    }
}

fn describe_plan(plan: &RelocationPlan) -> String {
    let what = match plan.kind {
        RelocationKind::Split => format!(
            "Split unit {} of `{}` into tenant `{}` ({}) on placement `{}`:",
            plan.org_id, plan.source, plan.target, plan.target_tenant_id, plan.placement
        ),
        RelocationKind::Merge => {
            format!("Merge `{}` into `{}` under unit {}:", plan.source, plan.target, plan.org_id)
        }
    };
    let mut lines = vec![what];
    lines.extend(plan.tables.iter().filter(|table| table.rows > 0).map(|table| {
        let existing = if table.existing > 0 {
            format!(" ({} already there)", table.existing)
        } else {
            String::new()
        };
        format!("{:>10}  {:<6} {}{existing}", table.rows, table.action, table.table)
    }));
    lines.push(format!("{} rows would move or be copied.", plan.rows));
    for warning in &plan.warnings {
        lines.push(format!("Warning ({}): {}", warning.table, warning.message));
    }
    for conflict in &plan.conflicts {
        lines.push(format!(
            "Conflict ({}, {} rows): {} First rows: {}",
            conflict.table,
            conflict.count,
            conflict.message,
            conflict.rows.join("; ")
        ));
    }
    if plan.ready {
        lines.push("Nothing blocks: run the same command without `--dry-run`.".to_owned());
    } else {
        lines.push("Blocked:".to_owned());
        lines.extend(
            plan.blocking.iter().map(|blocker| format!("  {}: {}", blocker.code, blocker.message)),
        );
    }
    lines.join("\n")
}

fn describe(relocation: &TenantRelocation) -> String {
    let mut report = format!(
        "{} {} ({}): {}; {} rows written to the target.",
        relocation.kind.as_str(),
        relocation.id,
        relocation.reason,
        relocation.state,
        relocation.tables.iter().map(|table| table.rows).sum::<u64>()
    );
    match (&relocation.error, relocation.state.as_str()) {
        (Some(error), "failed") => report.push_str(&format!(
            "\nStopped before changing anything: {error}\nFix it and start again."
        )),
        (Some(error), _) => report.push_str(&format!(
            "\nStopped: {error}\nRun `autoxide tenant {} --resume {}` to continue.",
            relocation.kind.as_str(),
            relocation.id
        )),
        _ => {}
    }
    report
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::config::ConfigSource;
    use autoxide_kernel::database::Database;
    use autoxide_kernel::event::Actor;
    use autoxide_kernel::tenancy::{NewTenant, OrgId, OrgTree, TenantAdmin};
    use autoxide_testkit::TestDatabase;

    use super::*;
    use crate::runtime::{self, Migrations};

    /// `tenant split` / `merge` / `relocation-status`: dry-run in kế hoạch không đổi gì; tách thật
    /// chạy ngay tới xong; xem lượt; gộp lại.
    #[tokio::test]
    async fn tach_va_gop_khach_bang_lenh() {
        TestDatabase::run(async |test| {
            let source = ConfigSource::from_pairs([
                ("DATABASE_URL", test.url()),
                ("AUTOXIDE_TENANCY_MODE", "multi"),
                ("AUTOXIDE_SECRET_KEY", "a-configured-secret-that-is-long-enough-0123"),
                ("AUTOXIDE_TENANCY_CACHE_SECONDS", "1"),
                ("AUTOXIDE_RELOCATION_DRAIN_SECONDS", "5"),
            ]);
            let settings = crate::settings::read(&source).unwrap();
            autoxide_testkit::prepare_from_template(
                &settings.config.database,
                &settings.modules.migrator().unwrap(),
            )
            .await;
            let database = Database::connect(&settings.config.database).await.unwrap();
            settings.modules.migrator().unwrap().up(&database).await.unwrap();
            database.close().await.unwrap();
            let runtime = runtime::start(settings, Migrations::RequireApplied).await.unwrap();
            // Bước đóng băng không chờ TTL cache + thời gian tối đa của request.
            runtime
                .modules
                .api::<autoxide_kernel::tenancy::TenantRelocations>()
                .unwrap()
                .skip_settle();
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
            let service = runtime.modules.api::<TenantRelocations>().unwrap();
            let input = NewTenant {
                slug: "acme".to_owned(),
                name: "Acme".to_owned(),
                parent_id: None,
                placement: None,
            };
            let acme = admin.create(Actor::system(), input).await.unwrap();
            let tenant = registry
                .tenant(autoxide_kernel::tenancy::TenantId::from_uuid(acme.id))
                .await
                .unwrap();
            let north = OrgId::from_uuid(Uuid::now_v7());
            let write = tenant.database().begin_write().await.unwrap();
            let root = OrgTree::insert_root(&write, tenant.id()).await.unwrap();
            OrgTree::insert(&write, tenant.id(), north, root, 8).await.unwrap();
            write.commit().await.unwrap();
            // Event của lúc tạo khách: giao hết (tách chờ hàng đợi của khách rỗng).
            let dispatcher = runtime.modules.dispatcher().unwrap();
            while dispatcher.run_once().await.unwrap() > 0 {}

            let split = || {
                Some(split_input(
                    "acme".to_owned(),
                    Some(north.as_uuid()),
                    Some("acme-north".to_owned()),
                    Some("Acme North".to_owned()),
                    Some("spare".to_owned()),
                    true,
                    Some("contract".to_owned()),
                ))
            };
            let plan = relocate(&service, split(), true, None).await.unwrap();
            assert!(plan.contains("Split unit"), "{plan}");
            assert!(plan.contains("Nothing blocks"), "{plan}");
            let done = relocate(&service, split(), false, None).await.unwrap();
            assert!(done.contains("split") && done.contains(": done;"), "{done}");
            let id: Uuid = done.split_whitespace().nth(1).unwrap().parse().unwrap();
            let shown = status(&service, id).await.unwrap();
            assert!(shown.contains(": done;"), "{shown}");
            // Event `kernel.relocation_finished` và job dời kho báo cáo của khách mới: chạy hết.
            let worker = runtime.modules.worker(4).unwrap();
            while dispatcher.run_once().await.unwrap() + worker.run_once().await.unwrap() > 0 {}
            let merge = merge_input(
                "acme-north".to_owned(),
                Some("acme".to_owned()),
                Some(root.as_uuid()),
                Some("back".to_owned()),
            );
            let back = relocate(&service, Some(merge), false, None).await.unwrap();
            assert!(back.starts_with("merge") && back.contains(": done;"), "{back}");
            assert!(relocate(&service, None, false, Some(id)).await.unwrap().contains(": done;"));
            runtime.database.close().await.unwrap();
        })
        .await
        .unwrap();
    }
}
