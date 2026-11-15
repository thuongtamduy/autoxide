//! Lệnh `autoxide tenant` (bước 33): quản lý khách từ máy chủ — tạo, xem, tạm khoá, mở lại, cấp
//! phát lại khách tạo hỏng giữa chừng. Chạy với quyền của người vận hành máy chủ (như `autoxide
//! user`), gọi cùng service với REST (`TenantAdmin`).

use std::process::ExitCode;

use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{PageQuery, PageRequest};
use autoxide_kernel::tenancy::{NewTenant, Tenant, TenantAdmin, TenantChanges, TenantFilter};

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
    /// Prepare again a tenant whose creation stopped half-way (safe to run again).
    Provision { tenant: String },
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
        let outcome = match runtime.modules.api::<TenantAdmin>() {
            Some(admin) => execute(&admin, action).await.map_err(|error| error.message()),
            None => Err("the kernel did not start its tenant service".to_owned()),
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
    action: Action,
) -> Result<String, autoxide_kernel::error::Error> {
    match action {
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
        Action::Provision { tenant } => Ok(line(&admin.provision(Actor::system(), &tenant).await?)),
    }
}

async fn set_status(
    admin: &TenantAdmin,
    tenant: &str,
    status: &str,
) -> Result<String, autoxide_kernel::error::Error> {
    let found = admin.get(tenant).await?;
    let changes = TenantChanges { name: None, status: Some(status.to_owned()) };
    Ok(line(&admin.update(Actor::system(), found.id, changes, None).await?))
}

/// Một dòng mô tả khách.
fn line(tenant: &Tenant) -> String {
    format!(
        "{}  {}  {}  placement={}  \"{}\"",
        tenant.id, tenant.slug, tenant.status, tenant.placement, tenant.name
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
            let listed = execute(&admin, Action::List).await.unwrap();
            assert!(listed.contains("default  active  placement=main"), "{listed}");
            let shown =
                execute(&admin, Action::Show { tenant: "default".to_owned() }).await.unwrap();
            assert_eq!(shown, listed);
            let create = Action::Create {
                slug: "acme".to_owned(),
                name: "Acme".to_owned(),
                placement: None,
                parent: None,
            };
            assert_eq!(execute(&admin, create).await.unwrap_err().code(), "tenant_single_mode");
            let refused = execute(&admin, Action::Suspend { tenant: "default".to_owned() }).await;
            assert_eq!(refused.unwrap_err().code(), "validation_failed");
            runtime.database.close().await.unwrap();
        })
        .await
        .unwrap();
    }
}
