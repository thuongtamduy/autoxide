//! Tool MCP của module `audit`: cùng service và permission với `/api/v1/audit`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::Schema;
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use uuid::Uuid;
use validator::Validate;

use crate::http::{AUDIT_READ, AuditEntryResponse};
use crate::service::{AuditService, Filter};

/// Which entries to list; every filter is optional. Paging as for every list.
#[derive(Debug, Default, Deserialize, Validate, Schema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListEntriesInput {
    /// Only changes made by this user.
    actor_user_id: Option<Uuid>,
    /// Only changes to this record, e.g. `auth.user:<id>`.
    subject: Option<String>,
    /// Only this action (event name), e.g. `auth.user_updated`.
    action: Option<String>,
    /// Only changes from this module, e.g. `auth`.
    module: Option<String>,
    /// Maximum number of entries to return, 1 to 200. Defaults to 50.
    #[schema(minimum = 1, maximum = 200)]
    limit: Option<u32>,
    /// Where to continue: the `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
    /// `desc` (newest first, the default) or `asc` (oldest first).
    order: Option<String>,
}

/// An entry to get.
#[derive(Debug, Deserialize, Validate, Schema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntryId {
    /// Id of the entry.
    id: Uuid,
}

pub(crate) fn all(service: &Arc<AuditService>) -> Vec<Tool> {
    let (list, get) = (service.clone(), service.clone());
    vec![
        Tool::new(
            "list_entries",
            "List audit log entries — who changed what, when, before and after — newest first; \
             filter by user, record, action or module. Follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, _: Principal, input: ListEntriesInput| {
                let service = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, input.order)
                        .with_count(input.count.unwrap_or(false));
                    let filter = Filter {
                        actor_user_id: input.actor_user_id,
                        subject: input.subject,
                        action: input.action,
                        module: input.module,
                        actor_kind: Vec::new(),
                    };
                    let page = service.list(&tenant, filter, &page.page()?).await?;
                    Ok::<Page<AuditEntryResponse>, Error>(page.map(AuditEntryResponse::from))
                }
            },
        )
        .requires(AUDIT_READ)
        .read_only(),
        Tool::new(
            "get_entry",
            "Get one audit log entry.",
            move |tenant: TenantContext, _: Principal, input: EntryId| {
                let service = get.clone();
                async move {
                    Ok::<_, Error>(AuditEntryResponse::from(service.get(&tenant, input.id).await?))
                }
            },
        )
        .requires(AUDIT_READ)
        .read_only(),
    ]
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_kernel::database::Database;
    use autoxide_kernel::event::{Delivered, EventHandler};
    use autoxide_kernel::migration::{Migrator, ModuleMigrations};
    use autoxide_testkit::TestDatabase;
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn tool_doc_audit_log_nhu_rest() {
        TestDatabase::run(async |test| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let mut reader = ConfigReader::new(&source);
            let config = Config::read(&mut reader);
            let (config, _) = reader.finish(config).unwrap();
            let database = Database::connect(&config.database).await.unwrap();
            Migrator::new(vec![ModuleMigrations::new("audit", crate::migration::all())])
                .unwrap()
                .up(&database)
                .await
                .unwrap();
            let recorder = crate::recorder::Recorder;
            for (module, name) in [("auth", "auth.user_updated"), ("files", "files.file_deleted")] {
                let event = Delivered::new(
                    autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone())
                        .default_tenant(),
                    module,
                    name.to_owned(),
                    json!({"after": {"x": 1}}),
                );
                recorder.handle(&event).await.unwrap();
            }
            let tenant = autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone())
                .default_tenant();
            let tools = all(&Arc::new(AuditService::new(Default::default())));
            let auditor = Principal::new(Uuid::now_v7(), ["audit.read".to_owned()]);
            let nobody = Principal::new(Uuid::now_v7(), []);
            assert!(tools.iter().all(|tool| !tool.allows(&nobody)));

            let list = tools.iter().find(|tool| tool.name() == "list_entries").unwrap();
            let page = list
                .call(tenant.clone(), auditor.clone(), json!({"module": "files"}))
                .await
                .unwrap();
            assert_eq!(page["data"].as_array().unwrap().len(), 1);
            assert_eq!(page["data"][0]["after"], json!({"x": 1}));
            let id = page["data"][0]["id"].clone();
            let get = tools.iter().find(|tool| tool.name() == "get_entry").unwrap();
            let entry = get.call(tenant.clone(), auditor.clone(), json!({"id": id})).await.unwrap();
            assert_eq!(entry["action"], "files.file_deleted");
            let missing =
                get.call(tenant.clone(), auditor, json!({"id": Uuid::now_v7()})).await.unwrap_err();
            assert_eq!(missing.code(), "audit.entry_not_found");
        })
        .await
        .unwrap();
    }
}
