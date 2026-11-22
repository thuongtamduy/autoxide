//! AI tạo và sửa trang (bước 67): yêu cầu (202, chạy nền) và lịch sử trò chuyện. Logic ở `ai/pages.rs`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::page_request;
use crate::ai::pages::{AiTurn, TurnKind};
use crate::service::ContentService;

/// One request to the AI page builder and its outcome.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AiTurnResponse {
    id: Uuid,
    entry_id: Uuid,
    /// `compose` (a whole page) or `edit` (a change to the current layout).
    kind: String,
    /// What the person asked.
    instruction: String,
    /// Blocks the change was limited to (`edit`).
    selection: Option<Vec<String>>,
    /// `queued`, `running`, `succeeded` or `failed`.
    status: String,
    /// Calls made to the model (a refused answer is retried at most twice).
    tries: i32,
    /// Why it failed, e.g. `content.ai_result_invalid`, `content.ai_conflict`, `ai.disabled`.
    error_code: Option<String>,
    /// The draft revision holding the result; publish the page to show it.
    revision_id: Option<Uuid>,
    /// The revision before the change: restore it (`POST /entries/{id}/revisions/{revision_id}/restore`)
    /// to undo.
    previous_revision_id: Option<Uuid>,
    requested_by: Uuid,
    #[schema(value_type = String, format = DateTime)]
    created_at: Timestamp,
    #[schema(value_type = Option<String>, format = DateTime)]
    finished_at: Option<Timestamp>,
}

fn response(turn: AiTurn) -> AiTurnResponse {
    AiTurnResponse {
        selection: turn.selection.as_deref().and_then(|text| serde_json::from_str(text).ok()),
        id: turn.id,
        entry_id: turn.entry_id,
        kind: turn.kind,
        instruction: turn.instruction,
        status: turn.status,
        tries: turn.tries,
        error_code: turn.error_code,
        revision_id: turn.revision_id,
        previous_revision_id: turn.previous_revision_id,
        requested_by: turn.requested_by,
        created_at: turn.created_at,
        finished_at: turn.finished_at,
    }
}

/// A request to the AI page builder.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiPageRequest {
    /// What the page should be, or what to change, in plain words (1 to 2000 characters), e.g.
    /// "a summer sale landing page with a hero, three reasons to buy and an FAQ".
    instruction: String,
    /// `edit` only: block ids to limit the change to.
    selection: Option<Vec<String>>,
}

async fn request(
    service: &ContentService,
    tenant: &TenantContext,
    principal: &Principal,
    id: Uuid,
    kind: TurnKind,
    input: AiPageRequest,
) -> Result<(StatusCode, Data<AiTurnResponse>), Error> {
    let turn = service
        .request_ai_page(tenant, principal, id, kind, &input.instruction, input.selection)
        .await?;
    Ok((StatusCode::ACCEPTED, Data { data: response(turn) }))
}

/// Ask the AI to build the page's block layout.
///
/// Requires `content.write` (or `content.write_own` on your page) and the plan feature
/// `content.ai_suggestions`. Runs in the background: follow it with `GET /entries/{id}/ai/thread`.
/// The AI only assembles blocks of `GET /api/v1/kernel/blocks`; its answer goes through the same
/// layout checks as a person's and becomes a **draft** marked `ai_assisted` — never published by
/// itself. A refused answer is retried at most twice.
#[utoipa::path(
    post,
    path = "/entries/{id}/ai/compose",
    extensions(("x-permissions" = json!(["content.write", "content.write_own"]))),
    operation_id = "compose_content_page_with_ai",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the page.")),
    request_body = AiPageRequest,
    responses(
        (status = 202, description = "Queued.", body = Data<AiTurnResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission (`permission_denied`, `content.not_owner`), or the plan lacks `content.ai_suggestions`.", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 422, description = "The instruction is empty or too long, or the entry is a post; see `errors`.", body = Problem),
        (status = 503, description = "AI is off on this server; code `ai.disabled`.", body = Problem),
    )
)]
pub(crate) async fn compose_page(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AiPageRequest>,
) -> Result<(StatusCode, Data<AiTurnResponse>), Error> {
    request(&service, &tenant, &principal, id, TurnKind::Compose, input).await
}

/// Ask the AI to change the page's block layout.
///
/// Requires `content.write` (or `content.write_own` on your page) and the plan feature
/// `content.ai_suggestions`. Like `compose`, on the current layout: say what to change ("make the
/// hero red", "add a buy now button"); `selection` limits it to some blocks. The result is a new
/// draft; the turn keeps the revision before it to undo.
#[utoipa::path(
    post,
    path = "/entries/{id}/ai/edit",
    extensions(("x-permissions" = json!(["content.write", "content.write_own"]))),
    operation_id = "edit_content_page_with_ai",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the page.")),
    request_body = AiPageRequest,
    responses(
        (status = 202, description = "Queued.", body = Data<AiTurnResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission, or the plan lacks `content.ai_suggestions`.", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 422, description = "The instruction is invalid, or the page has no block layout yet; see `errors`.", body = Problem),
        (status = 503, description = "AI is off on this server; code `ai.disabled`.", body = Problem),
    )
)]
pub(crate) async fn edit_page(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AiPageRequest>,
) -> Result<(StatusCode, Data<AiTurnResponse>), Error> {
    request(&service, &tenant, &principal, id, TurnKind::Edit, input).await
}

/// List the AI conversation of a page.
///
/// Requires `content.read`. Every request with who asked, its outcome and the revisions to compare
/// or undo, newest first.
#[utoipa::path(
    get,
    path = "/entries/{id}/ai/thread",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "list_content_page_ai_thread",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the page."), PageQuery),
    responses(
        (status = 200, description = "A page of turns.", body = Page<AiTurnResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 422, description = "A paging value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn ai_thread(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<AiTurnResponse>, Error> {
    let page = page_request(paging, Vec::new())?;
    Ok(service.ai_thread(&tenant, &principal, id, &page).await?.map(response))
}
