//! Quản trị SCIM của khách (envelope chung của Autoxide): token SCIM cho hệ thống danh tính, nhóm
//! nó đẩy sang và luật ánh xạ nhóm → role ở nút.
//! Cần `auth.scim.manage`.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use crate::permissions::SCIM_MANAGE;
use crate::scim::{BASE_PATH, CreatedScimToken, GroupMapping, GroupSummary, ScimToken};

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(list_scim_tokens, create_scim_token))
        .routes(routes!(revoke_scim_token))
        .routes(routes!(list_scim_groups))
        .routes(routes!(list_scim_group_mappings, create_scim_group_mapping))
        .routes(routes!(delete_scim_group_mapping))
}

/// A SCIM token of the tenant, without its secret.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct ScimTokenResponse {
    id: Uuid,
    /// Name to recognise the token, e.g. the identity provider it is set up in.
    #[schema(example = "Microsoft Entra ID")]
    name: String,
    /// Start and end of the token, to recognise it; never the whole token.
    #[schema(example = "axscim1_…k3Fq")]
    prefix: String,
    /// The user who created it.
    created_by: Uuid,
    created_at: Timestamp,
    /// Last time the identity provider used it (to within 5 minutes); `null`: never.
    last_used_at: Option<Timestamp>,
}

impl From<ScimToken> for ScimTokenResponse {
    fn from(token: ScimToken) -> Self {
        ScimTokenResponse {
            id: token.id,
            name: token.name,
            prefix: token.prefix,
            created_by: token.created_by,
            created_at: token.created_at,
            last_used_at: token.last_used_at,
        }
    }
}

/// A SCIM token just created: the only time its secret is shown.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct CreatedScimTokenResponse {
    #[serde(flatten)]
    token: ScimTokenResponse,
    /// The token itself (`axscim1_…`). Paste it into the identity provider as the secret token /
    /// API token; it is not shown again.
    token_secret: String,
    /// Path of the SCIM endpoint on this server; put the server's public address in front of it
    /// for the identity provider's "tenant URL" / "base URL".
    #[schema(example = "/api/v1/auth/scim/v2")]
    scim_base_path: String,
}

impl From<CreatedScimToken> for CreatedScimTokenResponse {
    fn from(created: CreatedScimToken) -> Self {
        CreatedScimTokenResponse {
            token: created.token.into(),
            token_secret: created.secret,
            scim_base_path: BASE_PATH.to_owned(),
        }
    }
}

/// A new SCIM token.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct CreateScimTokenRequest {
    /// 1–100 characters, e.g. the identity provider it is for.
    #[schema(example = "Microsoft Entra ID")]
    name: String,
}

/// List the SCIM tokens of the tenant.
///
/// Requires `auth.scim.manage`. Newest first; secrets are never listed.
#[utoipa::path(
    get,
    path = "/scim/tokens",
    extensions(("x-permissions" = json!(["auth.scim.manage"]))),
    operation_id = "list_scim_tokens",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every SCIM token of the tenant.", body = Data<Vec<ScimTokenResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.scim.manage`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_scim_tokens(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<ScimTokenResponse>>, Error> {
    principal.require(&SCIM_MANAGE)?;
    let tokens = state.scim.tokens(&tenant).await?;
    Ok(Data { data: tokens.into_iter().map(ScimTokenResponse::from).collect() })
}

/// Create a SCIM token for the tenant's identity provider.
///
/// Requires `auth.scim.manage` and the plan feature `auth.scim`. The identity provider (Entra ID,
/// Okta, Google Workspace) calls the SCIM endpoint (`scim_base_path`) with it to create, change
/// and disable staff accounts and groups. The secret is in the response only. Rotate by creating a
/// new token, switching the provider to it, then revoking the old one; a tenant has at most 5.
#[utoipa::path(
    post,
    path = "/scim/tokens",
    extensions(("x-permissions" = json!(["auth.scim.manage"]))),
    operation_id = "create_scim_token",
    security(("bearer" = [])),
    request_body = CreateScimTokenRequest,
    responses(
        (status = 201, description = "The token was created; `token_secret` holds it.", body = Data<CreatedScimTokenResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.scim.manage` (code `permission_denied`), or the plan does not include `auth.scim` (code `feature_not_enabled`).", body = Problem),
        (status = 409, description = "The tenant already has 5 SCIM tokens; code `auth.scim_token_limit`.", body = Problem),
        (status = 422, description = "`name` is empty or longer than 100 characters; code `validation_failed`.", body = Problem),
    )
)]
async fn create_scim_token(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateScimTokenRequest>,
) -> Result<Created<CreatedScimTokenResponse>, Error> {
    principal.require(&SCIM_MANAGE)?;
    let created = state.scim.create_token(&tenant, &principal, &input.name).await?;
    Ok(Created::new(format!("/auth/scim/tokens/{}", created.token.id), created.into()))
}

/// Revoke a SCIM token.
///
/// Requires `auth.scim.manage`. The identity provider using it is refused at once.
#[utoipa::path(
    delete,
    path = "/scim/tokens/{id}",
    extensions(("x-permissions" = json!(["auth.scim.manage"]))),
    operation_id = "revoke_scim_token",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the token.")),
    responses(
        (status = 204, description = "The token was revoked."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.scim.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No SCIM token of the tenant has this id; code `auth.scim_token_not_found`.", body = Problem),
    )
)]
async fn revoke_scim_token(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&SCIM_MANAGE)?;
    state.scim.revoke_token(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A group pushed by the identity provider, for administrators.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct ScimGroupSummaryResponse {
    id: Uuid,
    #[schema(example = "Sales")]
    display_name: String,
    external_id: Option<String>,
    /// How many people are in it.
    members: u64,
    /// How many mapping rules it has; `0`: its members get no roles from it (also listed in the
    /// work inbox as `auth.scim_unmapped_groups`).
    mappings: u64,
    updated_at: Timestamp,
}

impl From<GroupSummary> for ScimGroupSummaryResponse {
    fn from(summary: GroupSummary) -> Self {
        ScimGroupSummaryResponse {
            id: summary.group.id,
            display_name: summary.group.display_name,
            external_id: summary.group.external_id,
            members: summary.members,
            mappings: summary.mappings,
            updated_at: summary.group.updated_at,
        }
    }
}

/// A mapping rule: members of the group hold the role at the unit.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct ScimGroupMappingResponse {
    id: Uuid,
    group_id: Uuid,
    /// Organization unit; the tenant's id is the root unit.
    org_id: Uuid,
    role_id: Uuid,
    created_by: Uuid,
    created_at: Timestamp,
}

impl From<GroupMapping> for ScimGroupMappingResponse {
    fn from(row: GroupMapping) -> Self {
        ScimGroupMappingResponse {
            id: row.id,
            group_id: row.group_id,
            org_id: row.org_id,
            role_id: row.role_id,
            created_by: row.created_by,
            created_at: row.created_at,
        }
    }
}

/// A new mapping rule.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct CreateScimGroupMappingRequest {
    /// A SCIM group of the tenant (`GET /api/v1/auth/scim/groups`).
    group_id: Uuid,
    /// The unit where members get the role; the tenant's id for the root unit.
    org_id: Uuid,
    /// The role members get there.
    role_id: Uuid,
}

/// List the groups the identity provider pushed.
///
/// Requires `auth.scim.manage`. With member and mapping-rule counts, oldest first unless
/// `order=desc`.
#[utoipa::path(
    get,
    path = "/scim/groups",
    extensions(("x-permissions" = json!(["auth.scim.manage"]))),
    operation_id = "list_scim_groups",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of groups.", body = Page<ScimGroupSummaryResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.scim.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; code `validation_failed`.", body = Problem),
    )
)]
async fn list_scim_groups(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<ScimGroupSummaryResponse>, Error> {
    principal.require(&SCIM_MANAGE)?;
    let page = PageRequest::try_from(query)?;
    Ok(state.scim.group_summaries(&tenant, &page).await?.map(ScimGroupSummaryResponse::from))
}

/// List the mapping rules from SCIM groups to roles.
///
/// Requires `auth.scim.manage`. Oldest first.
#[utoipa::path(
    get,
    path = "/scim/group-mappings",
    extensions(("x-permissions" = json!(["auth.scim.manage"]))),
    operation_id = "list_scim_group_mappings",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every mapping rule of the tenant.", body = Data<Vec<ScimGroupMappingResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.scim.manage`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_scim_group_mappings(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<ScimGroupMappingResponse>>, Error> {
    principal.require(&SCIM_MANAGE)?;
    let rows = state.scim.mappings(&tenant).await?;
    Ok(Data { data: rows.into_iter().map(ScimGroupMappingResponse::from).collect() })
}

/// Map a SCIM group to a role at a unit.
///
/// Requires `auth.scim.manage`, and you must be able to give that role at that unit yourself (the
/// unit is in your scope, you hold every permission of the role; `admin` only at the root). Every
/// current member of the group gets the role at once; later members get it when they join.
#[utoipa::path(
    post,
    path = "/scim/group-mappings",
    extensions(("x-permissions" = json!(["auth.scim.manage"]))),
    operation_id = "create_scim_group_mapping",
    security(("bearer" = [])),
    request_body = CreateScimGroupMappingRequest,
    responses(
        (status = 201, description = "The rule was created and applied to the group's members.", body = Data<ScimGroupMappingResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.scim.manage` (code `permission_denied`), or the plan's staff limit is reached.", body = Problem),
        (status = 404, description = "No SCIM group has this id; code `auth.scim_group_not_found`.", body = Problem),
        (status = 409, description = "The same rule exists (code `auth.scim_mapping_taken`).", body = Problem),
        (status = 422, description = "The unit or role is unknown, out of your scope, or the role has permissions you do not have; see `errors`.", body = Problem),
    )
)]
async fn create_scim_group_mapping(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateScimGroupMappingRequest>,
) -> Result<Created<ScimGroupMappingResponse>, Error> {
    principal.require(&SCIM_MANAGE)?;
    let row = state
        .scim
        .create_mapping(&tenant, &principal, input.group_id, input.org_id, input.role_id)
        .await?;
    Ok(Created::new(format!("/auth/scim/group-mappings/{}", row.id), row.into()))
}

/// Remove a mapping rule.
///
/// Requires `auth.scim.manage`. Members of the group lose the role at once, unless another group or
/// a hand-made membership gives it too.
#[utoipa::path(
    delete,
    path = "/scim/group-mappings/{id}",
    extensions(("x-permissions" = json!(["auth.scim.manage"]))),
    operation_id = "delete_scim_group_mapping",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the rule.")),
    responses(
        (status = 204, description = "The rule was removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.scim.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No rule has this id; code `auth.scim_mapping_not_found`.", body = Problem),
        (status = 409, description = "Removing it would leave the tenant without an active administrator; code `auth.last_admin`.", body = Problem),
    )
)]
async fn delete_scim_group_mapping(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&SCIM_MANAGE)?;
    state.scim.delete_mapping(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
