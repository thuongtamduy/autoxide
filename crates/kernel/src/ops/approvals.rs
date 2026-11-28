//! Commands that need approval when an agent calls them (step 51 P12, D-201, D-213, D-224).
//!
//! A module marks a risky command with the OpenAPI extension `x-approval: "agent"` on `#[utoipa::path]`. When
//! the caller is an agent (an API key of kind `agent`), the [`gate`] layer does not let the command run: it
//! records an **approval request** (`kernel_approval_requests`, tenant level) and returns `202` with that
//! request. A person holding one of the command's permissions approves or rejects it
//! (`POST /api/v1/kernel/approvals/{id}/approve` | `/reject`, event `kernel.approval_decided`). Once
//! approved, the agent resends **exactly** the same request (same method, path, body, and the same
//! `Idempotency-Key`) with the header `Autoxide-Approval: <id>`; each approval lets the command run once,
//! within the deadline (24 hours by default).
//!
//! The idempotency layer does not keep this `202` response (see [`Pending`]): resending the same key after
//! approval runs the command for real. The corresponding MCP tool declares `Tool::requires_approval_for_agent`:
//! an agent calling that tool is refused and pointed to the REST path to ask for approval (MCP cannot do what
//! REST forbids).

use crate::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{FromRequestParts, MatchedPath, Path, Query, Request, State};
use axum::http::{HeaderName, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, Order, Query as SqlQuery};
use serde::{Deserialize, Serialize};
use utoipa::openapi::OpenApi;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::{ActorKind, Principal};
use crate::error::{Error, FieldError};
use crate::event::{Actor, EventInfo, Outbox};
use crate::http::ValidJson;
use crate::migration::prelude::*;
use crate::response::{Data, Problem};
use crate::tenancy::TenantContext;

const TABLE: &str = "kernel_approval_requests";
/// An approval request lives this many hours.
const LIFETIME_HOURS: i64 = 24;
/// A request body larger than this cannot ask for approval (a risky command does not need a large body).
const MAX_BODY: usize = 1024 * 1024;
/// Header the agent sends back with an approved command.
pub const APPROVAL_HEADER: HeaderName = HeaderName::from_static("autoxide-approval");

/// Event: an approval request was approved or rejected (the agent learns the outcome without polling forever).
pub(crate) const APPROVAL_DECIDED: EventInfo = EventInfo::new(
    "kernel.approval_decided",
    "A person approved or rejected a command an agent asked to run.",
);

#[derive(Debug, Serialize)]
struct ApprovalDecided {
    approval_id: Uuid,
    status: String,
    requested_by: Uuid,
}

impl crate::event::Event for ApprovalDecided {
    const NAME: &'static str = "kernel.approval_decided";
}

/// Marker on the 202 response: the idempotency layer does not keep it, so the agent can resend the same key
/// after approval.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pending(pub(crate) Uuid);

/// A module's command that needs approval when an agent calls it: (method, full path template) -> permission.
#[derive(Debug, Clone, Default)]
pub(crate) struct Rules {
    commands: Vec<(Method, String, Vec<String>)>,
}

impl Rules {
    pub(crate) fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    fn find(&self, method: &Method, path: &str) -> Option<&Vec<String>> {
        self.commands
            .iter()
            .find(|(known, template, _)| known == method && template == path)
            .map(|(_, _, permissions)| permissions)
    }
}

/// Commands with `x-approval: "agent"` in the documentation of module `module` (full path
/// `/api/v1/<module>...`), and writes `202` into their documentation.
pub(crate) fn rules(module: &str, openapi: &mut OpenApi) -> Rules {
    let mut commands = Vec::new();
    for (path, item) in &mut openapi.paths.paths {
        let methods = [
            (Method::POST, item.post.as_mut()),
            (Method::PUT, item.put.as_mut()),
            (Method::PATCH, item.patch.as_mut()),
            (Method::DELETE, item.delete.as_mut()),
        ];
        for (method, operation) in methods {
            let Some(operation) = operation else { continue };
            let marked = operation
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.get("x-approval"))
                .is_some_and(|value| value == "agent");
            if !marked {
                continue;
            }
            let permissions = operation
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.get("x-permissions"))
                .and_then(|value| value.as_array().cloned())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect();
            let response = utoipa::openapi::ResponseBuilder::new()
                .description(
                    "Called by an AI agent (API key of kind `agent`): nothing was done yet. The \
                     body is the approval request (`data`); once a person approves it, send the \
                     same request again, with the same `Idempotency-Key`, and the header \
                     `Autoxide-Approval: <id>`.",
                )
                .content(
                    "application/json",
                    utoipa::openapi::ContentBuilder::new()
                        .schema(Some(utoipa::openapi::schema::Object::new()))
                        .build(),
                )
                .build();
            operation.responses.responses.insert("202".to_owned(), response.into());
            commands.push((
                method,
                format!("{}/{module}{path}", crate::http::API_PREFIX),
                permissions,
            ));
        }
    }
    Rules { commands }
}

/// Layer in front of commands that need approval (see the top of the file). Not an agent, or a command that is not marked: passes through.
pub(crate) async fn gate(rules: Arc<Rules>, request: Request, next: Next) -> Response {
    let Some(permissions) = request
        .extensions()
        .get::<MatchedPath>()
        .and_then(|matched| rules.find(request.method(), matched.as_str()))
        .cloned()
    else {
        return next.run(request).await;
    };
    match checked(permissions, request, next).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn checked(
    permissions: Vec<String>,
    request: Request,
    next: Next,
) -> Result<Response, Error> {
    let (mut parts, body) = request.into_parts();
    // Không xác thực được thì để handler trả 401 như thường.
    let Ok(principal) = Principal::from_request_parts(&mut parts, &()).await else {
        return Ok(next.run(Request::from_parts(parts, body)).await);
    };
    if principal.actor_kind() != ActorKind::Agent {
        return Ok(next.run(Request::from_parts(parts, body)).await);
    }
    let tenant = TenantContext::from_request_parts(&mut parts, &()).await?;
    let bytes = axum::body::to_bytes(body, MAX_BODY).await.map_err(|_| {
        Error::validation(vec![FieldError::new(
            "body",
            "invalid",
            "is too large for a command that needs approval",
        )])
    })?;
    let target = parts.uri.path_and_query().map_or(parts.uri.path(), |path| path.as_str());
    let hash = crate::secret::sha256(&format!(
        "{}\n{target}\n{}",
        parts.method,
        String::from_utf8_lossy(&bytes)
    ));
    // Credential uỷ quyền (API key hay grant OAuth): lệnh đã duyệt chỉ dùng lại được bằng đúng credential đó.
    let api_key = principal.credential_id();
    if let Some(approval) = parts.headers.get(&APPROVAL_HEADER) {
        let id = approval.to_str().ok().and_then(|raw| raw.parse::<Uuid>().ok());
        let used = match id {
            Some(id) => consume(&tenant, id, principal.user_id(), &hash).await?,
            None => false,
        };
        if !used {
            return Err(Error::conflict(
                "kernel.approval_invalid",
                "The `Autoxide-Approval` header names no approved request for exactly this \
                 command (same method, address and body) by you: it may be pending, rejected, \
                 expired, already used, or for another command.",
            ));
        }
        return Ok(next.run(Request::from_parts(parts, Body::from(bytes))).await);
    }
    let id = Uuid::now_v7();
    let now = now();
    let summary = format!("{} {}", parts.method, parts.uri.path());
    let insert = SqlQuery::insert()
        .into_table(Alias::new(TABLE))
        .columns([
            "id",
            "tenant_id",
            "requested_by",
            "api_key_id",
            "summary",
            "body_hash",
            "permissions",
            "status",
            "expires_at",
            "created_at",
        ])
        .values_panic([
            id.into(),
            tenant.id().as_uuid().into(),
            principal.user_id().into(),
            api_key.into(),
            summary.into(),
            hash.into(),
            serde_json::to_string(&permissions).map_err(Error::internal)?.into(),
            "pending".into(),
            (now + SignedDuration::from_hours(LIFETIME_HOURS)).into(),
            now.into(),
        ])
        .to_owned();
    tenant.database().write().execute(&insert).await?;
    tracing::info!(approval_id = %id, "command of an agent waits for approval");
    let view = find_view(&tenant, id).await?;
    let mut response = (StatusCode::ACCEPTED, axum::Json(Data { data: view })).into_response();
    response.extensions_mut().insert(Pending(id));
    Ok(response)
}

/// Uses approval `id` for the command with hash `hash` of `user_id`: once, within the deadline. `true` when it
/// could be used (a conditional update: of two simultaneous sends only one gets through).
async fn consume(
    tenant: &TenantContext,
    id: Uuid,
    user_id: Uuid,
    hash: &str,
) -> Result<bool, Error> {
    let update = SqlQuery::update()
        .table(Alias::new(TABLE))
        .value(Alias::new("status"), "used")
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .and_where(Expr::col(Alias::new("requested_by")).eq(user_id))
        .and_where(Expr::col(Alias::new("body_hash")).eq(hash))
        .and_where(Expr::col(Alias::new("status")).eq("approved"))
        .and_where(Expr::col(Alias::new("expires_at")).gt(now()))
        .to_owned();
    Ok(tenant.database().write().execute(&update).await?.rows_affected() == 1)
}

fn now() -> Timestamp {
    Timestamp::now()
}

/// A command an AI agent asked to run, waiting for a person.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ApprovalRequest)]
pub(crate) struct ApprovalView {
    pub id: Uuid,
    /// The command, e.g. `DELETE /api/v1/content/entries/0190…`.
    pub summary: String,
    /// The user whose agent key asked.
    pub requested_by: Uuid,
    pub api_key_id: Option<Uuid>,
    /// `pending`, `approved`, `rejected`, `expired` or `used` (approved, then run).
    #[schema(example = "pending")]
    pub status: String,
    /// Any of these permissions may approve it.
    pub permissions: Vec<String>,
    pub decided_by: Option<Uuid>,
    pub decided_at: Option<Timestamp>,
    /// Why it was rejected.
    pub reason: Option<String>,
    pub expires_at: Timestamp,
    pub created_at: Timestamp,
    /// Where to look at it and decide.
    pub url: String,
}

const COLUMNS: [&str; 11] = [
    "id",
    "summary",
    "requested_by",
    "api_key_id",
    "status",
    "permissions",
    "decided_by",
    "decided_at",
    "reason",
    "expires_at",
    "created_at",
];

fn view(row: &sea_orm::QueryResult) -> Result<ApprovalView, Error> {
    let id: Uuid = row.try_get("", "id")?;
    let permissions: String = row.try_get("", "permissions")?;
    let expires_at: Timestamp = row.try_get("", "expires_at")?;
    let mut status: String = row.try_get("", "status")?;
    if status == "pending" && expires_at <= now() {
        status = "expired".to_owned();
    }
    Ok(ApprovalView {
        id,
        summary: row.try_get("", "summary")?,
        requested_by: row.try_get("", "requested_by")?,
        api_key_id: row.try_get("", "api_key_id")?,
        status,
        permissions: serde_json::from_str(&permissions).unwrap_or_default(),
        decided_by: row.try_get("", "decided_by")?,
        decided_at: row.try_get("", "decided_at")?,
        reason: row.try_get("", "reason")?,
        expires_at,
        created_at: row.try_get("", "created_at")?,
        url: format!("{}/kernel/approvals/{id}", crate::http::API_PREFIX),
    })
}

async fn find_view(tenant: &TenantContext, id: Uuid) -> Result<ApprovalView, Error> {
    let select = SqlQuery::select()
        .columns(COLUMNS)
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .to_owned();
    let row = tenant.database().read().query_one(&select).await?;
    row.as_ref().map(view).transpose()?.ok_or_else(|| not_found(id))
}

fn not_found(id: Uuid) -> Error {
    Error::not_found("kernel.approval_not_found", "There is no approval request `{id}` for you.")
        .with_param("id", id)
}

/// [`may_see`] in SQL, so the database skips what the caller may not see: the caller's own requests, and — for
/// a person — requests naming a permission the caller holds (`permissions` is a JSON array of names; a name is
/// matched with its quotes, `_` escaped). `may_see` still checks every row read.
fn visible_to(principal: &Principal) -> sea_orm::sea_query::Condition {
    use sea_orm::sea_query::{Condition, LikeExpr};
    let mine = Expr::col(Alias::new("requested_by")).eq(principal.user_id());
    let mut visible = Condition::any().add(mine);
    // Yêu cầu của người khác chỉ hiện cho người quyết được: phiên đăng nhập (D-20261126-mcp-oauth).
    if principal.is_session()
        && matches!(principal.actor_kind(), ActorKind::Human | ActorKind::Support)
    {
        if principal.has_all_permissions() {
            return Condition::all();
        }
        for permission in principal.permissions() {
            let escaped = permission.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            let pattern = LikeExpr::new(format!("%\"{escaped}\"%")).escape('\\');
            visible = visible.add(Expr::col(Alias::new("permissions")).like(pattern));
        }
    }
    visible
}

/// The caller sees the request: their own, or one they can decide with a permission (and is not an agent).
fn may_see(principal: &Principal, approval: &ApprovalView) -> bool {
    approval.requested_by == principal.user_id() || may_decide(principal, approval)
}

fn may_decide(principal: &Principal, approval: &ApprovalView) -> bool {
    // Duyệt là việc của người trong phiên đăng nhập: không API key, không client OAuth (D-20261126-mcp-oauth).
    if !principal.is_session()
        || !matches!(principal.actor_kind(), ActorKind::Human | ActorKind::Support)
    {
        return false;
    }
    // Lệnh không đòi permission nào (việc của chính người gọi: huỷ đơn của mình, trả tiền đơn của mình…): người
    // duyệt là chính chủ của agent, trong phiên đăng nhập của họ (D-20261128-agent-approval-audit).
    if approval.permissions.is_empty() {
        return approval.requested_by == principal.user_id();
    }
    approval.permissions.iter().any(|permission| principal.allows(permission))
}

/// Which approval requests.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ApprovalsQuery {
    /// Only `pending`, `approved`, `rejected`, `expired` or `used` requests.
    status: Option<String>,
}

/// Why a request is rejected.
#[derive(Debug, Clone, Deserialize, ToSchema, validator::Validate)]
#[serde(deny_unknown_fields)]
pub(crate) struct RejectRequest {
    /// 1 to 500 characters, shown to the agent.
    pub reason: String,
}

#[derive(Clone)]
pub(crate) struct ApprovalsState {
    pub outbox: Outbox,
}

/// List approval requests.
///
/// Any signed-in user: the requests of your own agents, and those you may decide (a person holding
/// one of the command's permissions). Newest first unless `order=asc`; paged with `limit` /
/// `cursor`. The rows read per request are bounded: a page may hold fewer than `limit` items (even
/// none) while `pagination.next_cursor` is set — keep following it until it is `null`. With
/// `count=true`, `total` is left out when the list is too long to count in one request.
#[utoipa::path(
    get,
    path = "/approvals",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_approvals",
    security(("bearer" = [])),
    params(ApprovalsQuery, crate::pagination::PageQuery),
    responses(
        (status = 200, description = "One page of requests, newest first.", body = crate::pagination::Page<ApprovalView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "`status` or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_approvals(
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ApprovalsQuery>,
    Query(page): Query<crate::pagination::PageQuery>,
) -> Result<crate::pagination::Page<ApprovalView>, Error> {
    let page = crate::pagination::PageRequest::try_from(page);
    page_for(&tenant, &principal, query.status.as_deref(), page).await
}

const STATUSES: [&str; 5] = ["pending", "approved", "rejected", "expired", "used"];

/// One page of the requests the caller may see, by ID (newest first unless `order=asc`); `status` filters by
/// status. Status and paging problems come back in one 422. Who may see a request is filtered in SQL
/// ([`visible_to`], keyset on the indexes `(tenant_id, id)` / `(tenant_id, status, id)`) and checked again per
/// row; the scan per request is bounded, so a page may be shorter than `limit` (even empty) while
/// `next_cursor` is set (see `fetch_statement_page_filtered`).
pub(crate) async fn page_for(
    tenant: &TenantContext,
    principal: &Principal,
    status: Option<&str>,
    page: Result<crate::pagination::PageRequest, Error>,
) -> Result<crate::pagination::Page<ApprovalView>, Error> {
    use crate::pagination::{fetch_statement_page_filtered, try_map_page, with_filter_problems};
    use sea_orm::sea_query::IntoColumnRef as _;
    let mut problems = Vec::new();
    if status.is_some_and(|status| !STATUSES.contains(&status)) {
        problems.push(FieldError::new(
            "status",
            "invalid",
            "must be `pending`, `approved`, `rejected`, `expired` or `used`",
        ));
    }
    let page = with_filter_problems(problems, page)?;
    let mut select = SqlQuery::select()
        .columns(COLUMNS)
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .to_owned();
    // `expired` không lưu: là `pending` đã quá hạn (xem `view`).
    match status {
        Some("pending") => {
            select
                .and_where(Expr::col(Alias::new("status")).eq("pending"))
                .and_where(Expr::col(Alias::new("expires_at")).gt(now()));
        }
        Some("expired") => {
            select
                .and_where(Expr::col(Alias::new("status")).eq("pending"))
                .and_where(Expr::col(Alias::new("expires_at")).lte(now()));
        }
        Some(status) => {
            select.and_where(Expr::col(Alias::new("status")).eq(status));
        }
        None => {}
    }
    select.cond_where(visible_to(principal));
    let rows = fetch_statement_page_filtered(
        select,
        None,
        Alias::new("id").into_column_ref(),
        |row| Ok(row.try_get("", "id")?),
        |row| Ok(may_see(principal, &view(row)?)),
        &page,
        tenant.database().read(),
    )
    .await?;
    try_map_page(rows, view)
}

/// Requests the caller may see, newest first, at most 200 (the work inbox's own count); `status` filters by
/// status.
pub(crate) async fn list_for(
    tenant: &TenantContext,
    principal: &Principal,
    status: Option<&str>,
) -> Result<Vec<ApprovalView>, Error> {
    if status.is_some_and(|status| !STATUSES.contains(&status)) {
        return Err(Error::validation(vec![FieldError::new(
            "status",
            "invalid",
            "must be `pending`, `approved`, `rejected`, `expired` or `used`",
        )]));
    }
    let mut select = SqlQuery::select()
        .columns(COLUMNS)
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .order_by(Alias::new("id"), Order::Desc)
        .limit(500)
        .to_owned();
    if status == Some("pending") {
        // Hộp việc đếm thường xuyên: chỉ đọc dòng còn chờ và còn hạn.
        select
            .and_where(Expr::col(Alias::new("status")).eq("pending"))
            .and_where(Expr::col(Alias::new("expires_at")).gt(now()));
    }
    let rows = tenant.database().read().query_all(&select).await?;
    let mut found = Vec::new();
    for row in &rows {
        let approval = view(row)?;
        if may_see(principal, &approval) && status.is_none_or(|status| status == approval.status) {
            found.push(approval);
        }
        if found.len() == 200 {
            break;
        }
    }
    Ok(found)
}

/// Work inbox (P2): pending requests the caller can decide.
pub(crate) struct ApprovalsWork;

#[async_trait::async_trait]
impl crate::ops::work::WorkSource for ApprovalsWork {
    fn id(&self) -> &'static str {
        "kernel.approvals"
    }

    // Không dùng (nguồn cho mọi người đã đăng nhập, tự lọc theo quyền duyệt).
    fn permission(&self) -> crate::access::Permission {
        crate::ops::module_catalog::MODULES_READ
    }

    fn for_any_signed_in(&self) -> bool {
        true
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _target: crate::access::OrgTarget,
    ) -> Result<u64, Error> {
        let mine = decidable(tenant, principal).await?;
        Ok(<u64 as TryFrom<usize>>::try_from(mine.len()).unwrap_or(u64::MAX))
    }

    async fn items(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _target: crate::access::OrgTarget,
        limit: u16,
    ) -> Result<Vec<crate::ops::work::WorkItem>, Error> {
        let mine = decidable(tenant, principal).await?;
        Ok(mine
            .into_iter()
            .take(usize::from(limit))
            .map(|approval| {
                crate::ops::work::WorkItem::new(
                    approval.id.to_string(),
                    crate::ops::work::Severity::Attention,
                    "An AI agent asks to run {command}",
                    "home.work",
                )
                .param("command", approval.summary)
                .due_at(Some(approval.expires_at))
            })
            .collect())
    }
}

async fn decidable(
    tenant: &TenantContext,
    principal: &Principal,
) -> Result<Vec<ApprovalView>, Error> {
    Ok(list_for(tenant, principal, Some("pending"))
        .await?
        .into_iter()
        .filter(|approval| may_decide(principal, approval))
        .collect())
}

/// Get an approval request.
///
/// Your own agent's request, or one you may decide.
#[utoipa::path(
    get,
    path = "/approvals/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_approval",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the request.")),
    responses(
        (status = 200, description = "The request.", body = Data<ApprovalView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such request for you; code `kernel.approval_not_found`.", body = Problem),
    )
)]
async fn get_approval(
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ApprovalView>, Error> {
    let approval = find_view(&tenant, id).await?;
    if !may_see(&principal, &approval) {
        return Err(not_found(id));
    }
    Ok(Data { data: approval })
}

async fn decide(
    state: &ApprovalsState,
    tenant: &TenantContext,
    principal: &Principal,
    id: Uuid,
    status: &str,
    reason: Option<String>,
) -> Result<ApprovalView, Error> {
    let approval = find_view(tenant, id).await?;
    if !may_see(principal, &approval) {
        return Err(not_found(id));
    }
    if !may_decide(principal, &approval) {
        return Err(Error::forbidden(
            "kernel.approval_forbidden",
            "Only a person holding one of the command's permissions decides: {permissions}.",
        )
        .with_param("permissions", approval.permissions.join(", ")));
    }
    let transaction = tenant.database().begin_write().await?;
    let update = SqlQuery::update()
        .table(Alias::new(TABLE))
        .value(Alias::new("status"), status)
        .value(Alias::new("decided_by"), principal.user_id())
        .value(Alias::new("decided_at"), now())
        .value(Alias::new("reason"), reason)
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .and_where(Expr::col(Alias::new("status")).eq("pending"))
        .and_where(Expr::col(Alias::new("expires_at")).gt(now()))
        .to_owned();
    if transaction.execute(&update).await?.rows_affected() == 0 {
        transaction.rollback().await?;
        return Err(Error::conflict(
            "kernel.approval_decided",
            "Approval request `{id}` is not pending any more (it is `{status}`).",
        )
        .with_param("id", id)
        .with_param("status", approval.status));
    }
    state
        .outbox
        .record(
            &transaction,
            tenant,
            Actor::from(principal),
            Some(format!("kernel.approval:{id}")),
            &ApprovalDecided {
                approval_id: id,
                status: status.to_owned(),
                requested_by: approval.requested_by,
            },
        )
        .await?;
    transaction.commit().await?;
    find_view(tenant, id).await
}

/// Approve an agent's command.
///
/// A person holding one of the command's permissions. The agent may then send the command again,
/// once, before `expires_at`. Emits `kernel.approval_decided`.
#[utoipa::path(
    post,
    path = "/approvals/{id}/approve",
    extensions(("x-permissions" = json!([]))),
    operation_id = "approve_request",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the request.")),
    responses(
        (status = 200, description = "Approved.", body = Data<ApprovalView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "You may not decide it (an agent, or none of its permissions); code `kernel.approval_forbidden`.", body = Problem),
        (status = 404, description = "No such request for you; code `kernel.approval_not_found`.", body = Problem),
        (status = 409, description = "Not pending (decided or expired); code `kernel.approval_decided`.", body = Problem),
    )
)]
async fn approve_request(
    State(state): State<Arc<ApprovalsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ApprovalView>, Error> {
    Ok(Data { data: decide(&state, &tenant, &principal, id, "approved", None).await? })
}

/// Reject an agent's command.
///
/// A person holding one of the command's permissions; `reason` is shown to the agent. Emits
/// `kernel.approval_decided`.
#[utoipa::path(
    post,
    path = "/approvals/{id}/reject",
    extensions(("x-permissions" = json!([]))),
    operation_id = "reject_request",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the request.")),
    request_body = RejectRequest,
    responses(
        (status = 200, description = "Rejected.", body = Data<ApprovalView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "You may not decide it; code `kernel.approval_forbidden`.", body = Problem),
        (status = 404, description = "No such request for you; code `kernel.approval_not_found`.", body = Problem),
        (status = 409, description = "Not pending; code `kernel.approval_decided`.", body = Problem),
        (status = 422, description = "`reason` is empty or too long; see `errors`.", body = Problem),
    )
)]
async fn reject_request(
    State(state): State<Arc<ApprovalsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<RejectRequest>,
) -> Result<Data<ApprovalView>, Error> {
    let reason = input.reason.trim().to_owned();
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err(Error::validation(vec![FieldError::new(
            "reason",
            "invalid",
            "must be 1 to 500 characters long",
        )]));
    }
    Ok(Data { data: decide(&state, &tenant, &principal, id, "rejected", Some(reason)).await? })
}

/// Which approval requests to list.
#[derive(Debug, Default, Deserialize, validator::Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListApprovalsInput {
    /// Only `pending`, `approved`, `rejected`, `expired` or `used` requests.
    status: Option<String>,
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc`.
    order: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// A decision on an approval request.
#[derive(Debug, Deserialize, validator::Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecideApprovalInput {
    /// Id of the request.
    id: Uuid,
    /// `approve` or `reject`.
    #[schema(example = "approve")]
    decision: String,
    /// Required to reject (1 to 500 characters, shown to the agent).
    reason: Option<String>,
}

pub(crate) fn tools(state: ApprovalsState) -> Vec<crate::mcp::Tool> {
    use crate::mcp::Tool;
    let state = Arc::new(state);
    vec![
        Tool::new(
            "list_approvals",
            "List approval requests: those of your own AI agents, and those you may decide. An \
             agent sees here whether a person approved its command yet. A page may hold fewer items \
             than `limit`, even none, and still have `pagination.next_cursor`: keep following it \
             until it is null.",
            |tenant: TenantContext, principal: Principal, input: ListApprovalsInput| async move {
                // Cùng kiểu tham số trang và cùng hàm với REST.
                let page = crate::mcp::PageInput::new(input.limit, input.cursor, input.order)
                    .with_count(input.count.unwrap_or(false))
                    .page();
                page_for(&tenant, &principal, input.status.as_deref(), page).await
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "decide_approval",
            "Approve or reject a command an AI agent asked to run. Only a person holding one of \
             the command's permissions decides; an agent cannot.",
            move |tenant: TenantContext, principal: Principal, input: DecideApprovalInput| {
                let state = state.clone();
                async move {
                    match input.decision.as_str() {
                        "approve" => {
                            decide(&state, &tenant, &principal, input.id, "approved", None).await
                        }
                        "reject" => {
                            let reason = input
                                .reason
                                .as_deref()
                                .map(str::trim)
                                .unwrap_or_default()
                                .to_owned();
                            if reason.is_empty() || reason.chars().count() > 500 {
                                return Err(Error::validation(vec![FieldError::new(
                                    "reason",
                                    "invalid",
                                    "must be 1 to 500 characters long",
                                )]));
                            }
                            decide(&state, &tenant, &principal, input.id, "rejected", Some(reason))
                                .await
                        }
                        _ => Err(Error::validation(vec![FieldError::new(
                            "decision",
                            "invalid",
                            "must be `approve` or `reject`",
                        )])),
                    }
                }
            },
        )
        .for_any_signed_in(),
    ]
}

pub(crate) fn routes(state: ApprovalsState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_approvals))
        .routes(routes!(get_approval))
        .routes(routes!(approve_request))
        .routes(routes!(reject_request))
        .with_state(Arc::new(state))
}

/// Migration that creates the table `kernel_approval_requests` (tenant level, every data database).
pub(crate) struct CreateApprovalRequests;

impl MigrationName for CreateApprovalRequests {
    fn name(&self) -> &str {
        "m0024_create_approval_requests"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateApprovalRequests {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("requested_by"))
                    .col(schema::uuid_null("api_key_id"))
                    .col(schema::string_len("summary", 600))
                    .col(schema::string_len("body_hash", 64))
                    .col(schema::text("permissions"))
                    .col(schema::string_len("status", 16))
                    .col(schema::uuid_null("decided_by"))
                    .col(schema::timestamp_with_time_zone_null("decided_at"))
                    .col(schema::string_len_null("reason", 2000))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_approval_requests_list_idx")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("id"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(TABLE)).to_owned()).await
    }
}
