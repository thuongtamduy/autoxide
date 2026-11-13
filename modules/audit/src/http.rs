//! Endpoint xem audit log, dưới `/api/v1/audit`.

use std::sync::Arc;

use autoxide_kernel::access::{Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::entity;
use crate::service::{AuditService, Filter};

pub(crate) const AUDIT_READ: Permission = Permission::new(
    "audit.read",
    "Read the audit log: who changed what, when, before and after. The before and after \
         values hold the data of every module (draft content, private media details, user \
         names and emails), so this amounts to reading everything.",
);

pub(crate) fn routes(service: Arc<AuditService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_entries))
        .routes(routes!(get_entry))
        .with_state(service)
}

/// One change, as recorded in the audit log.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct AuditEntryResponse {
    /// Id of the event that recorded the change; ids sort by time.
    id: Uuid,
    /// Module where the change happened.
    #[schema(example = "auth")]
    module: String,
    /// What happened: the event name, `<module>.<what_happened>`.
    #[schema(example = "auth.user_updated")]
    action: String,
    /// User who made the change; `null` for the system (command line, background jobs,
    /// self-registration).
    actor_user_id: Option<Uuid>,
    /// API key used, when the change was made with one.
    actor_api_key_id: Option<Uuid>,
    /// Who made the change: `user` (a signed-in user or API key), `system` (command line,
    /// background jobs) or `public` (someone not signed in, such as a visitor sending a form).
    #[schema(example = "user")]
    actor_kind: String,
    /// Record that changed, as `<kind>:<id>`, e.g. `auth.user:0190…`.
    #[schema(example = "auth.user:01936f0e-6c9f-7cc2-8a52-2b2f4d1c1e5a")]
    subject: Option<String>,
    /// State before the change, when it applies.
    before: Option<Value>,
    /// State after the change, when it applies.
    after: Option<Value>,
    /// Other facts about the change.
    details: Option<Value>,
    /// When the change happened.
    occurred_at: DateTime<Utc>,
    /// When the audit log recorded it (shortly after).
    recorded_at: DateTime<Utc>,
}

impl From<entity::Model> for AuditEntryResponse {
    fn from(row: entity::Model) -> Self {
        let json = |text: Option<String>| text.and_then(|text| serde_json::from_str(&text).ok());
        AuditEntryResponse {
            id: row.id,
            module: row.module,
            action: row.action,
            actor_user_id: row.actor_user_id,
            actor_api_key_id: row.actor_api_key_id,
            // Dòng ghi trước khi có cột: hiểu theo `actor_user_id`.
            actor_kind: row.actor_kind.unwrap_or_else(|| {
                if row.actor_user_id.is_some() { "user" } else { "system" }.to_owned()
            }),
            subject: row.subject,
            before: json(row.before),
            after: json(row.after),
            details: json(row.details),
            occurred_at: row.occurred_at,
            recorded_at: row.recorded_at,
        }
    }
}

/// Narrow the audit log. Filters combine with AND.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct AuditFilter {
    /// Only changes made by this user.
    actor_user_id: Option<Uuid>,
    /// Only changes to this record, e.g. `auth.user:<id>`.
    subject: Option<String>,
    /// Only this kind of change, e.g. `auth.user_updated`.
    action: Option<String>,
    /// Only changes in this module, e.g. `auth`.
    module: Option<String>,
}

/// List the audit log.
///
/// Requires `audit.read`. Newest first unless `order=asc`. Changes appear a moment
/// after they happen.
#[utoipa::path(
    get,
    path = "/",
    operation_id = "list_entries",
    security(("bearer" = [])),
    params(AuditFilter, PageQuery),
    responses(
        (status = 200, description = "One page of audit entries.", body = Page<AuditEntryResponse>),
        (status = 400, description = "A filter has the wrong type, e.g. `actor_user_id` is not a UUID.", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `audit.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_entries(
    State(service): State<Arc<AuditService>>,
    principal: Principal,
    Query(filter): Query<AuditFilter>,
    Query(page): Query<PageQuery>,
) -> Result<Page<AuditEntryResponse>, Error> {
    principal.require(&AUDIT_READ)?;
    let filter = Filter {
        actor_user_id: filter.actor_user_id,
        subject: filter.subject,
        action: filter.action,
        module: filter.module,
    };
    let page = service.list(filter, &PageRequest::try_from(page)?).await?;
    Ok(page.map(AuditEntryResponse::from))
}

/// Get one audit entry.
///
/// Requires `audit.read`.
#[utoipa::path(
    get,
    path = "/{id}",
    operation_id = "get_entry",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry.")),
    responses(
        (status = 200, description = "The entry.", body = Data<AuditEntryResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `audit.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No entry has this id; code `audit.entry_not_found`.", body = Problem),
    )
)]
async fn get_entry(
    State(service): State<Arc<AuditService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<AuditEntryResponse>, Error> {
    principal.require(&AUDIT_READ)?;
    Ok(Data { data: service.get(id).await?.into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::event::{Actor, Delivered, EventHandler};
    use autoxide_testkit::{TestApp, TestDatabase};
    use axum::http::StatusCode;
    use chrono::SubsecRound;
    use serde_json::json;

    /// `Bearer auditor` có `audit.read`; `Bearer nobody` không có gì.
    async fn setup(test: &TestDatabase) -> (TestApp, crate::recorder::Recorder) {
        let app = TestApp::builder(test)
            .user("auditor", 0, &["audit.read"])
            .user("nobody", 0, &[])
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let recorder = crate::recorder::Recorder::new(app.database().clone());
        (app, recorder)
    }

    fn event(name: &str, actor: Uuid, subject: &str) -> Delivered {
        Delivered::new(
            "auth",
            name.to_owned(),
            json!({"before": {"name": "a"}, "after": {"name": "b"}}),
        )
        .actor(Actor::user(actor))
        .subject(Some(subject.to_owned()))
        .occurred_at(Utc::now().trunc_subsecs(6))
    }

    async fn get(app: &TestApp, uri: &str, as_who: &str) -> (StatusCode, Value) {
        let answer = app.get(uri).token(as_who).send().await.unwrap();
        (answer.status(), serde_json::from_slice(answer.body()).unwrap())
    }

    #[tokio::test]
    async fn xem_va_loc_audit_log() {
        TestDatabase::run(async |test| {
            let (app, recorder) = setup(test).await;
            let (ada, bob) = (Uuid::now_v7(), Uuid::now_v7());
            let first = event("auth.user_updated", ada, "auth.user:1");
            recorder.handle(&first).await.unwrap();
            recorder.handle(&event("auth.role_created", ada, "auth.role:1")).await.unwrap();
            recorder.handle(&event("auth.user_updated", bob, "auth.user:2")).await.unwrap();

            let (status, all) = get(&app, "/api/v1/audit?order=asc", "auditor").await;
            assert_eq!(status, StatusCode::OK, "{all}");
            assert_eq!(all["data"].as_array().unwrap().len(), 3);
            assert_eq!(all["data"][0]["id"], json!(first.id));
            assert_eq!(all["data"][0]["before"], json!({"name": "a"}));
            assert_eq!(all["data"][0]["after"], json!({"name": "b"}));

            let uri = format!("/api/v1/audit?actor_user_id={ada}&action=auth.user_updated");
            let (_, filtered) = get(&app, &uri, "auditor").await;
            assert_eq!(filtered["data"].as_array().unwrap().len(), 1);
            assert_eq!(filtered["data"][0]["subject"], "auth.user:1");
            let (_, by_subject) = get(&app, "/api/v1/audit?subject=auth.user:2", "auditor").await;
            assert_eq!(by_subject["data"][0]["actor_user_id"], json!(bob));
            assert_eq!(by_subject["data"][0]["actor_kind"], "user");
            let (_, paged) = get(&app, "/api/v1/audit?limit=2", "auditor").await;
            assert!(paged["pagination"]["next_cursor"].is_string());

            let (status, one) = get(&app, &format!("/api/v1/audit/{}", first.id), "auditor").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(one["data"]["action"], "auth.user_updated");
            let missing = format!("/api/v1/audit/{}", Uuid::now_v7());
            let (status, problem) = get(&app, &missing, "auditor").await;
            assert_eq!(
                (status, problem["code"].clone()),
                (StatusCode::NOT_FOUND, json!("audit.entry_not_found"))
            );

            let (status, denied) = get(&app, "/api/v1/audit", "nobody").await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(denied["code"], "permission_denied");
            let (status, _) = get(&app, "/api/v1/audit?actor_user_id=not-a-uuid", "auditor").await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
        })
        .await
        .unwrap();
    }
}
