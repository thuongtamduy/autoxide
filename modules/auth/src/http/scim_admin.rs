//! Quản trị SCIM của khách (envelope chung của Autoxide): token SCIM cho hệ thống danh tính.
//! Cần `auth.scim.manage`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use crate::permissions::SCIM_MANAGE;
use crate::scim::{BASE_PATH, CreatedScimToken, ScimToken};

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(list_scim_tokens, create_scim_token))
        .routes(routes!(revoke_scim_token))
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
    created_at: DateTime<Utc>,
    /// Last time the identity provider used it (to within 5 minutes); `null`: never.
    last_used_at: Option<DateTime<Utc>>,
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
