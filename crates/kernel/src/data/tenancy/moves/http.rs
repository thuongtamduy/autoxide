//! REST (`/api/v1/kernel/tenants/{id}/move…`) và tool MCP của việc dời khách — gọi đúng các hàm
//! của [`TenantMoves`]. Chỉ nhân sự khách nền tảng (`require_platform`).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::{TENANTS_MOVE, TenantMove, TenantMovePlan, TenantMoves};
use crate::access::Principal;
use crate::error::Error;
use crate::event::Actor;
use crate::i18n::Language;
use crate::mcp::Tool;
use crate::response::{Created, Data, Problem};
use crate::tenancy::{TENANTS_READ, TenantContext, require_platform};

fn yes() -> bool {
    true
}

/// Where to move the tenant's data.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = TenantMoveRequest)]
pub(crate) struct MoveRequest {
    /// Name of the placement (database) to move the data to.
    #[schema(example = "acme-db")]
    #[validate(length(min = 1, max = 63))]
    to: String,
    /// Only report what the move would do, changing nothing. Defaults to `true`: send `false` to
    /// move.
    #[serde(default = "yes")]
    #[schema(default = true)]
    dry_run: bool,
    /// Keep the tenant's data at the old placement as a backup copy (its queued jobs and events
    /// are removed there so nothing runs twice). Defaults to `false`: the old copy is deleted.
    #[serde(default)]
    keep_source: bool,
}

/// Move a tenant to another placement.
///
/// Requires `kernel.tenants.move` and an account of the platform tenant (not one entered with
/// `Autoxide-Tenant`). With `dry_run` (the default) nothing changes: the answer lists what blocks
/// the move and the rows of the tenant that would be copied. With `"dry_run": false` the tenant
/// answers `503 tenant_maintenance` while a background job copies its data, switches it to the new
/// placement and deletes the old copy; follow it at the `Location` of the answer. Sending the same
/// move again resumes one that stopped half-way.
#[utoipa::path(
    post,
    path = "/tenants/{id}/move",
    extensions(("x-permissions" = json!(["kernel.tenants.move"]))),
    operation_id = "move_tenant",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant.")),
    request_body = MoveRequest,
    responses(
        (status = 200, description = "Dry run: what the move would do; nothing changed.", body = Data<TenantMovePlan>),
        (status = 201, description = "The move started (or a stopped one resumed) in the background; `Location` is its status.", body = Data<TenantMove>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.move` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant or placement; code `tenant_not_found` or `tenant_placement_not_found`.", body = Problem),
        (status = 409, description = "The move cannot run now; code `tenant_move_platform`, `tenant_move_same_placement`, `tenant_move_not_active`, `tenant_move_running`, `tenant_move_unfinished`, `tenant_move_schema_differs`, `tenant_move_same_database` or `tenant_placement_behind`. A dry run lists every reason.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
        (status = 503, description = "A database cannot be reached; code `tenant_move_unreachable`.", body = Problem),
    )
)]
async fn move_tenant(
    State(moves): State<Arc<TenantMoves>>,
    principal: Principal,
    language: Language,
    Path(id): Path<String>,
    crate::http::ValidJson(input): crate::http::ValidJson<MoveRequest>,
) -> Result<Response, Error> {
    principal.require(&TENANTS_MOVE)?;
    require_platform(&principal)?;
    if input.dry_run {
        let plan = moves.plan(&id, &input.to, input.keep_source, language).await?;
        return Ok(Data { data: plan }.into_response());
    }
    let moving =
        moves.start(Actor::from(&principal), &id, &input.to, input.keep_source, true).await?;
    let path = format!("/kernel/tenants/{}/moves/{}", moving.tenant_id, moving.id);
    Ok(Created::new(path, moving).into_response())
}

/// Get a move of a tenant.
///
/// Requires `kernel.tenants.read` and an account of the platform tenant. `state` goes `copying` →
/// `copied` → `switched` → `done` (or `aborted`); `error` tells why a run stopped.
#[utoipa::path(
    get,
    path = "/tenants/{id}/moves/{move_id}",
    extensions(("x-permissions" = json!(["kernel.tenants.read"]))),
    operation_id = "get_tenant_move",
    security(("bearer" = [])),
    params(
        ("id" = String, Path, description = "Id or slug of the tenant."),
        ("move_id" = Uuid, Path, description = "Id of the move."),
    ),
    responses(
        (status = 200, description = "The move.", body = Data<TenantMove>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.read` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant or move; code `tenant_not_found` or `tenant_move_not_found`.", body = Problem),
    )
)]
async fn get_tenant_move(
    State(moves): State<Arc<TenantMoves>>,
    principal: Principal,
    Path((id, move_id)): Path<(String, Uuid)>,
) -> Result<Data<TenantMove>, Error> {
    principal.require(&TENANTS_READ)?;
    require_platform(&principal)?;
    Ok(Data { data: moves.get(&id, move_id).await? })
}

/// Route dưới `/api/v1/kernel`.
pub(crate) fn routes(moves: Arc<TenantMoves>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(move_tenant))
        .routes(routes!(get_tenant_move))
        .with_state(moves)
}

// Đầu vào của tool `move_tenant`.
/// Which tenant to move, and where.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MoveTenantInput {
    /// Id or slug of the tenant.
    #[validate(length(min = 1, max = 63))]
    tenant: String,
    /// Name of the placement (database) to move the data to.
    #[validate(length(min = 1, max = 63))]
    to: String,
    /// Only report what the move would do, changing nothing. Defaults to `true`.
    #[serde(default = "yes")]
    #[schema(default = true)]
    dry_run: bool,
    /// Keep the old copy of the data as a backup. Defaults to `false`.
    #[serde(default)]
    keep_source: bool,
}

// Đầu vào của tool `get_tenant_move`.
/// Which move to get.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TenantMoveRef {
    /// Id or slug of the tenant.
    #[validate(length(min = 1, max = 63))]
    tenant: String,
    /// Id of the move, from `kernel.move_tenant`.
    move_id: Uuid,
}

/// Kết quả của tool `move_tenant`: kế hoạch (dry run) hay lượt dời đã bắt đầu.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum MoveOutcome {
    Plan(TenantMovePlan),
    Started(TenantMove),
}

/// Tool MCP — gọi đúng các hàm REST dùng.
pub(crate) fn tools(moves: Arc<TenantMoves>) -> Vec<Tool> {
    let (start, get) = (moves.clone(), moves);
    vec![
        Tool::new(
            "move_tenant",
            "Move a tenant's data to another placement (database) (platform staff only). Dry run by \
             default: returns what blocks the move and the rows that would be copied, changing \
             nothing. With `dry_run: false` the tenant is unavailable (503) while a background job \
             copies its data, switches it and deletes the old copy (unless `keep_source`); follow \
             it with `kernel.get_tenant_move`. Calling again with the same `to` resumes a move that \
             stopped.",
            move |_tenant: TenantContext, principal: Principal, input: MoveTenantInput| {
                let start = start.clone();
                async move {
                    require_platform(&principal)?;
                    if input.dry_run {
                        let plan = start
                            .plan(&input.tenant, &input.to, input.keep_source, Language::En)
                            .await?;
                        return Ok(MoveOutcome::Plan(plan));
                    }
                    let actor = Actor::from(&principal);
                    let moving = start
                        .start(actor, &input.tenant, &input.to, input.keep_source, true)
                        .await?;
                    Ok(MoveOutcome::Started(moving))
                }
            },
        )
        .requires(TENANTS_MOVE)
        .destructive(),
        Tool::new(
            "get_tenant_move",
            "Get a move of a tenant to another placement (platform staff only): its `state` \
             (`copying`, `copied`, `switched`, `done`, `aborted`), rows copied per table and the \
             `error` that stopped the last run, if any.",
            move |_tenant: TenantContext, principal: Principal, input: TenantMoveRef| {
                let get = get.clone();
                async move {
                    require_platform(&principal)?;
                    get.get(&input.tenant, input.move_id).await
                }
            },
        )
        .requires(TENANTS_READ)
        .read_only(),
    ]
}
