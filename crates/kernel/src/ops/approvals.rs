//! Lệnh cần duyệt khi agent gọi (bước 51 P12, D-201, D-213, D-224).
//!
//! Module đánh dấu lệnh rủi ro bằng phần mở rộng OpenAPI `x-approval: "agent"` trên `#[utoipa::path]`.
//! Khi người gọi là agent (API key loại `agent`), lớp [`gate`] không cho lệnh chạy: nó ghi một **yêu
//! cầu duyệt** (`kernel_approval_requests`, cấp khách hàng) và trả `202` kèm yêu cầu đó. Người có
//! một trong các permission của lệnh duyệt hay từ chối (`POST /api/v1/kernel/approvals/{id}/approve`
//! | `/reject`, event `kernel.approval_decided`). Được duyệt thì agent gửi lại **đúng** request đó
//! (cùng method, đường dẫn, thân — và cùng `Idempotency-Key`) kèm header `Autoxide-Approval: <id>`;
//! mỗi lần duyệt chạy được một lần, trong hạn (mặc định 24 giờ).
//!
//! Lớp idempotency không giữ câu trả lời `202` này (xem [`Pending`]): gửi lại cùng key sau khi duyệt
//! thì lệnh chạy thật. Tool MCP tương ứng khai `Tool::requires_approval_for_agent`: agent gọi tool đó
//! thì bị từ chối và được chỉ sang đường REST để xin duyệt (MCP không làm được điều REST cấm).

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
/// Yêu cầu duyệt sống chừng này giờ.
const LIFETIME_HOURS: i64 = 24;
/// Thân request lớn hơn thế không xin duyệt được (lệnh rủi ro không cần thân lớn).
const MAX_BODY: usize = 1024 * 1024;
/// Header agent gửi lại kèm lệnh đã được duyệt.
pub const APPROVAL_HEADER: HeaderName = HeaderName::from_static("autoxide-approval");

/// Event: một yêu cầu duyệt được duyệt hay bị từ chối (agent biết kết quả mà không hỏi lại mãi).
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

/// Đánh dấu trên câu trả lời 202: lớp idempotency không giữ nó, để agent gửi lại cùng key sau khi
/// được duyệt.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pending(pub(crate) Uuid);

/// Lệnh của một module cần duyệt khi agent gọi: (method, đường dẫn mẫu đầy đủ) → permission.
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

/// Lệnh có `x-approval: "agent"` trong tài liệu của module `module` (đường dẫn đầy đủ
/// `/api/v1/<module>…`), và ghi `202` vào tài liệu của chúng.
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

/// Lớp trước lệnh cần duyệt (xem đầu file). Không phải agent, hay lệnh không đánh dấu: cho qua.
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
    let api_key = principal.api_key_id();
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

/// Dùng lần duyệt `id` cho lệnh có băm `hash` của `user_id`: một lần, trong hạn. `true` khi dùng
/// được (cập nhật có điều kiện: hai lần gửi đồng thời chỉ một lần qua).
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

/// Người gọi thấy yêu cầu: của mình, hay có một permission duyệt được nó (và không phải agent).
fn may_see(principal: &Principal, approval: &ApprovalView) -> bool {
    approval.requested_by == principal.user_id() || may_decide(principal, approval)
}

fn may_decide(principal: &Principal, approval: &ApprovalView) -> bool {
    matches!(principal.actor_kind(), ActorKind::Human | ActorKind::Support)
        && approval.permissions.iter().any(|permission| principal.allows(permission))
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
/// one of the command's permissions). Newest first, at most 200.
#[utoipa::path(
    get,
    path = "/approvals",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_approvals",
    security(("bearer" = [])),
    params(ApprovalsQuery),
    responses(
        (status = 200, description = "The requests.", body = Data<Vec<ApprovalView>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "`status` is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_approvals(
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ApprovalsQuery>,
) -> Result<Data<Vec<ApprovalView>>, Error> {
    Ok(Data { data: list_for(&tenant, &principal, query.status.as_deref()).await? })
}

/// Yêu cầu người gọi được thấy, mới trước, tối đa 200; `status` lọc theo trạng thái.
pub(crate) async fn list_for(
    tenant: &TenantContext,
    principal: &Principal,
    status: Option<&str>,
) -> Result<Vec<ApprovalView>, Error> {
    const STATUSES: [&str; 5] = ["pending", "approved", "rejected", "expired", "used"];
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

/// Hộp việc (P2): yêu cầu đang chờ mà người gọi quyết được.
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
             agent sees here whether a person approved its command yet.",
            |tenant: TenantContext, principal: Principal, input: ListApprovalsInput| async move {
                list_for(&tenant, &principal, input.status.as_deref()).await
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

/// Migration tạo bảng `kernel_approval_requests` (cấp khách hàng, mọi database dữ liệu).
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
