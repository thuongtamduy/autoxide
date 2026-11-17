//! Endpoint xem audit log, dưới `/api/v1/audit`.

use std::sync::Arc;

use autoxide_kernel::access::{Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
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
        .routes(routes!(list_activity))
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
    /// Display name of `actor_user_id`, with `include=actor`: `null` when the account no longer
    /// exists; absent when no user made the change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    actor_name: Option<Option<String>>,
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
            actor_name: None,
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
    /// Only changes made by this kind of actor: `user`, `system` or `public` (see `actor_kind`).
    #[param(example = "user")]
    actor_kind: Option<String>,
    /// `actor`: add `actor_name`, the display name of who made each change, in the same answer.
    #[param(inline)]
    include: Option<AuditInclude>,
}

/// Extra data to add to each entry.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuditInclude {
    Actor,
}

/// One line of recent activity.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct ActivityResponse {
    /// Id of the audit entry.
    id: Uuid,
    /// The event, `<module>.<what_happened>`.
    #[schema(example = "content.entry_published")]
    action: String,
    /// What happened, as a sentence in the language of `Accept-Language`.
    #[schema(example = "Ada published “Đường đến Đà Lạt”")]
    sentence: String,
    /// Record that changed, as `<kind>:<id>`.
    subject: Option<String>,
    actor_user_id: Option<Uuid>,
    occurred_at: DateTime<Utc>,
}

/// How many lines.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ActivityQuery {
    /// At most this many lines, 1 to 100. Defaults to 20.
    limit: Option<u32>,
}

/// List recent activity.
///
/// Any signed-in user. Readable sentences ("Ada published …") for the events modules mark as
/// activity, newest first — only those whose module lets the caller see them (for example
/// published entries need `content.read`). Unlike `GET /api/v1/audit` it needs no `audit.read`
/// and shows no before / after values.
#[utoipa::path(
    get,
    path = "/activity",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_activity",
    security(("bearer" = [])),
    params(ActivityQuery),
    responses(
        (status = 200, description = "Recent activity, newest first.", body = Data<Vec<ActivityResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "`limit` is not between 1 and 100; see `errors`.", body = Problem),
    )
)]
async fn list_activity(
    State(service): State<Arc<AuditService>>,
    tenant: TenantContext,
    principal: Principal,
    language: autoxide_kernel::i18n::Language,
    Query(query): Query<ActivityQuery>,
) -> Result<Data<Vec<ActivityResponse>>, Error> {
    let limit = query.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(Error::validation(vec![autoxide_kernel::error::FieldError::new(
            "limit",
            "invalid",
            "must be between 1 and 100",
        )]));
    }
    let lines = service.recent_activity(&tenant, &principal, limit, language).await?;
    Ok(Data {
        data: lines
            .into_iter()
            .map(|line| ActivityResponse {
                id: line.row.id,
                action: line.row.action,
                sentence: line.sentence,
                subject: line.row.subject,
                actor_user_id: line.row.actor_user_id,
                occurred_at: line.row.occurred_at,
            })
            .collect(),
    })
}

/// List the audit log.
///
/// Requires `audit.read`. Newest first unless `order=asc`. Changes appear a moment
/// after they happen. With `include=actor`, each entry carries the name of who made it.
#[utoipa::path(
    get,
    path = "/",
    extensions(("x-permissions" = json!(["audit.read"]))),
    operation_id = "list_entries",
    security(("bearer" = [])),
    params(AuditFilter, PageQuery),
    responses(
        (status = 200, description = "One page of audit entries.", body = Page<AuditEntryResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `audit.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid, e.g. `actor_user_id` is not a UUID; code `validation_failed`, every problem in `errors`.", body = Problem),
    )
)]
async fn list_entries(
    State(service): State<Arc<AuditService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<AuditFilter>,
    Query(page): Query<PageQuery>,
) -> Result<Page<AuditEntryResponse>, Error> {
    principal.require(&AUDIT_READ)?;
    let with_actor = matches!(filter.include, Some(AuditInclude::Actor));
    let actor_kind = filter.actor_kind.map(|kind| kind.trim().to_owned());
    if let Some(kind) = &actor_kind
        && !["user", "system", "public"].contains(&kind.as_str())
    {
        return Err(Error::validation(vec![autoxide_kernel::error::FieldError::new(
            "actor_kind",
            "invalid",
            "must be `user`, `system` or `public`",
        )]));
    }
    let filter = Filter {
        actor_user_id: filter.actor_user_id,
        subject: filter.subject,
        action: filter.action,
        module: filter.module,
        actor_kind,
    };
    let page = service.list(&tenant, filter, &PageRequest::try_from(page)?).await?;
    if !with_actor {
        return Ok(page.map(AuditEntryResponse::from));
    }
    let mut ids: Vec<Uuid> = page.data.iter().filter_map(|row| row.actor_user_id).collect();
    ids.sort_unstable();
    ids.dedup();
    let names = service.actor_names(&tenant, &ids).await?;
    Ok(page.map(|row| {
        let name = row.actor_user_id.map(|id| names.get(&id).cloned());
        AuditEntryResponse { actor_name: name, ..AuditEntryResponse::from(row) }
    }))
}

/// Get one audit entry.
///
/// Requires `audit.read`.
#[utoipa::path(
    get,
    path = "/{id}",
    extensions(("x-permissions" = json!(["audit.read"]))),
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
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<AuditEntryResponse>, Error> {
    principal.require(&AUDIT_READ)?;
    Ok(Data { data: service.get(&tenant, id).await?.into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::database::Database;
    use autoxide_kernel::event::{Actor, Delivered, EventHandler};
    use autoxide_testkit::{TestApp, TestDatabase};
    use axum::http::StatusCode;
    use chrono::SubsecRound;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
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
        let recorder = crate::recorder::Recorder;
        (app, recorder)
    }

    fn event(database: &Database, name: &str, actor: Uuid, subject: &str) -> Delivered {
        Delivered::new(
            autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone()).default_tenant(),
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
            let first = event(app.database(), "auth.user_updated", ada, "auth.user:1");
            recorder.handle(&first).await.unwrap();
            recorder
                .handle(&event(app.database(), "auth.role_created", ada, "auth.role:1"))
                .await
                .unwrap();
            recorder
                .handle(&event(app.database(), "auth.user_updated", bob, "auth.user:2"))
                .await
                .unwrap();

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
            let (_, users) = get(&app, "/api/v1/audit?actor_kind=user", "auditor").await;
            assert_eq!(users["data"].as_array().unwrap().len(), 3);
            let (_, system) = get(&app, "/api/v1/audit?actor_kind=system", "auditor").await;
            assert_eq!(system["data"], json!([]));
            let (status, bad) = get(&app, "/api/v1/audit?actor_kind=robot", "auditor").await;
            assert_eq!(
                (status, bad["errors"][0]["field"].as_str()),
                (StatusCode::UNPROCESSABLE_ENTITY, Some("actor_kind"))
            );
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
            let (status, bad) =
                get(&app, "/api/v1/audit?actor_user_id=not-a-uuid", "auditor").await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(bad["errors"][0]["field"], "actor_user_id");
        })
        .await
        .unwrap();
    }

    /// Module `auth` giả: chỉ đăng ký loại `auth.user` vào sổ đối tượng (tên của Ada).
    struct FakeAuth(Uuid);

    struct Names(Uuid);

    #[async_trait::async_trait]
    impl autoxide_kernel::resource::ResourceResolver for Names {
        async fn resolve(
            &self,
            _: &TenantContext,
            id: &str,
        ) -> Result<Option<autoxide_kernel::resource::Resource>, Error> {
            Ok((id == self.0.to_string()).then(|| autoxide_kernel::resource::Resource::new("Ada")))
        }
    }

    impl autoxide_kernel::module::Module for FakeAuth {
        fn name(&self) -> &'static str {
            "auth"
        }

        fn start(
            &mut self,
            context: &mut autoxide_kernel::module::StartContext<'_>,
        ) -> Result<(), Error> {
            context.resource_type("auth.user", Arc::new(Names(self.0)))
        }
    }

    /// Đề bài giao diện B2: `include=actor` trả tên người làm trong cùng câu trả lời.
    #[tokio::test]
    async fn kem_ten_nguoi_lam() {
        TestDatabase::run(async |test| {
            let ada = Uuid::now_v7();
            let app = TestApp::builder(test)
                .user("auditor", 0, &["audit.read"])
                .module(FakeAuth(ada))
                .module(crate::module())
                .build()
                .await
                .unwrap();
            let recorder = crate::recorder::Recorder;
            let gone = Uuid::now_v7();
            recorder
                .handle(&event(app.database(), "auth.user_updated", ada, "auth.user:1"))
                .await
                .unwrap();
            recorder
                .handle(&event(app.database(), "auth.user_updated", gone, "auth.user:2"))
                .await
                .unwrap();

            let (status, page) =
                get(&app, "/api/v1/audit?include=actor&order=asc", "auditor").await;
            assert_eq!(status, StatusCode::OK, "{page}");
            assert_eq!(page["data"][0]["actor_name"], "Ada");
            assert!(page["data"][1]["actor_name"].is_null(), "{page}");
            let (_, plain) = get(&app, "/api/v1/audit", "auditor").await;
            assert!(plain["data"][0].get("actor_name").is_none());
        })
        .await
        .unwrap();
    }

    /// Bước 33 (D-276): mỗi khách một chuỗi băm (khách trên database chung và khách có database
    /// riêng), `doctor` kiểm từng chuỗi; người của khách này không đọc được audit của khách kia.
    #[tokio::test]
    async fn chuoi_rieng_moi_khach_va_doctor_kiem_tung_khach() {
        TestDatabase::run(async |test| {
            use autoxide_kernel::doctor::{Probe as _, Status};
            let app = TestApp::builder(test)
                .tenant("a")
                .tenant_dedicated("b")
                .user_in("a", "auditor-a", 1, &["audit.read"])
                .user_in("b", "auditor-b", 2, &["audit.read"])
                .module(crate::module())
                .build()
                .await
                .unwrap();
            let (a, b) = (app.tenant("a").await.unwrap(), app.tenant("b").await.unwrap());
            let marker = "MARKER-a-7f3";
            let mut seeded = Vec::new();
            for (tenant, subject) in [
                (&a, format!("auth.user:{marker}")),
                (&a, "x:1".to_owned()),
                (&b, "x:2".to_owned()),
            ] {
                let event = Delivered::new(
                    tenant.clone(),
                    "auth",
                    "auth.user_updated".to_owned(),
                    json!({}),
                )
                .actor(Actor::user(Uuid::now_v7()))
                .subject(Some(subject))
                .occurred_at(Utc::now().trunc_subsecs(6));
                if tenant.id() == a.id() {
                    seeded.push(("id", event.id.to_string()));
                }
                crate::recorder::Recorder.handle(&event).await.unwrap();
            }
            for (tenant, expected) in [(&a, 2), (&b, 1)] {
                let rows = crate::entity::Entity::find()
                    .filter(crate::entity::Column::TenantId.eq(tenant.id().as_uuid()))
                    .all(tenant.database().read())
                    .await
                    .unwrap();
                let mut sequences: Vec<_> = rows.iter().filter_map(|row| row.sequence).collect();
                sequences.sort_unstable();
                assert_eq!(
                    sequences,
                    (1..=expected).collect::<Vec<i64>>(),
                    "each tenant counts from 1"
                );
            }
            let probe = crate::chain::ChainProbe { databases: app.registry().clone() };
            let finding = probe.probe().await;
            assert_eq!(finding.status, Status::Ok, "{finding:?}");
            assert!(finding.message.contains("3 entries"), "{finding:?}");

            let (status, page) = get(&app, "/api/v1/audit", "auditor-b").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(page["data"].as_array().unwrap().len(), 1);
            let violations =
                autoxide_testkit::leak_check(&app, "auditor-b", marker, &seeded).await.unwrap();
            assert_eq!(violations, Vec::<String>::new());

            // Sửa một dòng của `a`: doctor báo đúng khách.
            crate::entity::Entity::update_many()
                .col_expr(
                    crate::entity::Column::Action,
                    sea_orm::sea_query::Expr::value("auth.forged"),
                )
                .filter(crate::entity::Column::TenantId.eq(a.id().as_uuid()))
                .exec(a.database().write())
                .await
                .unwrap();
            let finding = probe.probe().await;
            assert_eq!(finding.status, Status::Failed);
            assert!(finding.message.contains(&a.id().as_uuid().to_string()), "{finding:?}");
        })
        .await
        .unwrap();
    }

    const SHOP_READ: autoxide_kernel::access::Permission =
        autoxide_kernel::access::Permission::new("shop.read", "See the shop.");

    /// Module giả khai event `shop.item_added` có câu hoạt động.
    struct Shop;

    impl autoxide_kernel::module::Module for Shop {
        fn name(&self) -> &'static str {
            "shop"
        }

        fn permissions(&self) -> Vec<autoxide_kernel::access::Permission> {
            vec![SHOP_READ]
        }

        fn events(&self) -> Vec<autoxide_kernel::event::EventInfo> {
            vec![
                autoxide_kernel::event::EventInfo::new("shop.item_added", "An item was added.")
                    .activity("{actor} added “{name}”", SHOP_READ),
                autoxide_kernel::event::EventInfo::new("shop.item_viewed", "An item was viewed."),
            ]
        }
    }

    /// Bước 51 P5: hoạt động gần đây chỉ có event khai câu hoạt động, chỉ cho người có permission
    /// đọc của event, câu điền tên người làm và trường của dữ liệu.
    #[tokio::test]
    async fn hoat_dong_gan_day_theo_cau_cua_event() {
        TestDatabase::run(async |test| {
            let app = TestApp::builder(test)
                .user("clerk", 0, &["shop.read"])
                .user("nobody", 0, &[])
                .module(Shop)
                .module(crate::module())
                .build()
                .await
                .unwrap();
            let recorder = crate::recorder::Recorder;
            let actor = Uuid::now_v7();
            for name in ["shop.item_added", "shop.item_viewed"] {
                let delivered = Delivered::new(
                    autoxide_kernel::tenancy::DatabaseRegistry::single(app.database().clone())
                        .default_tenant(),
                    "shop",
                    name.to_owned(),
                    json!({"after": {"name": "Ấm trà"}}),
                )
                .actor(Actor::user(actor))
                .subject(Some("shop.item:1".to_owned()))
                .occurred_at(Utc::now().trunc_subsecs(6));
                recorder.handle(&delivered).await.unwrap();
            }
            let (status, body) = get(&app, "/api/v1/audit/activity", "clerk").await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let lines = body["data"].as_array().unwrap();
            assert_eq!(lines.len(), 1, "chỉ event có câu hoạt động: {body}");
            assert_eq!(
                lines[0]["sentence"], "— added “Ấm trà”",
                "không có module tài khoản: không có tên"
            );
            let (_, none) = get(&app, "/api/v1/audit/activity", "nobody").await;
            assert_eq!(none["data"], json!([]), "không có permission đọc của event");
        })
        .await
        .unwrap();
    }

    /// Lịch sử theo bản ghi (bước 51 P13): chỉ event có đúng chủ thể, mới trước, kèm người làm.
    #[tokio::test]
    async fn lich_su_theo_ban_ghi() {
        TestDatabase::run(async |test| {
            let (app, recorder) = setup(test).await;
            let ada = Uuid::now_v7();
            for (name, subject) in [
                ("auth.user_updated", "auth.user:1"),
                ("auth.role_created", "auth.role:1"),
                ("auth.user_disabled", "auth.user:1"),
            ] {
                recorder.handle(&event(app.database(), name, ada, subject)).await.unwrap();
            }
            let history = app
                .modules()
                .api::<dyn autoxide_kernel::resource::RecordHistory>()
                .expect("audit công bố RecordHistory");
            let tenant = autoxide_kernel::tenancy::DatabaseRegistry::single(app.database().clone())
                .default_tenant();
            let lines = history
                .history(&tenant, "auth.user:1", 10, autoxide_kernel::i18n::Language::En)
                .await
                .unwrap();
            let actions: Vec<&str> = lines.iter().map(|line| line.action.as_str()).collect();
            assert_eq!(actions, ["auth.user_disabled", "auth.user_updated"]);
            assert_eq!(lines[0].actor_user_id, Some(ada));
        })
        .await
        .unwrap();
    }
}
