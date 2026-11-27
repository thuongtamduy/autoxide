//! Lời mời người dùng qua email (D-312).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{OptionalTenant, TenantContext};
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use super::session::TokenResponse;
use crate::invitation::{Invitation, NewInvitation};
use crate::permissions::INVITATIONS_MANAGE;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(list_invitations, create_invitation))
        .routes(routes!(revoke_invitation))
        .routes(routes!(resend_invitation))
        .routes(routes!(accept_invitation))
}

/// An invitation to work in this tenant.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct InvitationResponse {
    id: Uuid,
    #[schema(example = "new.colleague@example.com")]
    email: String,
    name: String,
    /// The unit the roles apply at.
    org_id: Uuid,
    /// The roles given on acceptance.
    role_ids: Vec<Uuid>,
    /// Your own reference, e.g. an HR ticket.
    reference: Option<String>,
    /// `pending`, `accepted` or `revoked`. A pending invitation past `expires_at` can no longer be
    /// accepted; resend it.
    #[schema(example = "pending")]
    status: String,
    expires_at: Timestamp,
    /// The account that accepted it.
    accepted_user_id: Option<Uuid>,
    invited_by: Option<Uuid>,
    created_at: Timestamp,
}

impl From<Invitation> for InvitationResponse {
    fn from(row: Invitation) -> Self {
        InvitationResponse {
            role_ids: row.roles(),
            id: row.id,
            email: row.email,
            name: row.name,
            org_id: row.org_id,
            reference: row.reference,
            status: row.status,
            expires_at: row.expires_at,
            accepted_user_id: row.accepted_user_id,
            invited_by: row.invited_by,
            created_at: row.created_at,
        }
    }
}

/// A new invitation.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct InvitationRequest {
    #[schema(example = "new.colleague@example.com")]
    email: String,
    /// Display name for the new account (they can change it).
    name: Option<String>,
    /// The unit they will work at; omit for the root unit. It must be in your scope.
    org_id: Option<Uuid>,
    /// Roles given at that unit when they accept; you must have every permission of each role.
    #[validate(length(max = 100))]
    role_ids: Vec<Uuid>,
    /// Your own reference, up to 200 characters.
    reference: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct InvitationQuery {
    /// Only `pending`, `accepted` or `revoked` invitations.
    #[param(example = "pending")]
    status: Option<String>,
}

/// Accept an invitation.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AcceptInvitationRequest {
    /// The token from the invitation mail.
    #[validate(length(min = 1, max = 512))]
    token: String,
    /// For a new account: display name (default: the one in the invitation).
    name: Option<String>,
    /// For a new account: a password (15 to 128 characters, not a common or breached one).
    /// Omit to sign in later with a link or add a passkey with the tokens returned here.
    password: Option<String>,
}

/// The result of accepting.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct AcceptedResponse {
    /// The account that now has the roles.
    user_id: Uuid,
    /// Tokens of a new session when a new account was created; `null` when you accepted with
    /// your existing account.
    tokens: Option<TokenResponse>,
}

/// List invitations.
///
/// Requires `auth.invitations.manage`; only invitations at units in your scope. Newest first
/// unless `order=asc`.
#[utoipa::path(
    get,
    path = "/invitations",
    extensions(("x-permissions" = json!(["auth.invitations.manage"]))),
    operation_id = "list_invitations",
    security(("bearer" = [])),
    params(InvitationQuery, PageQuery),
    responses(
        (status = 200, description = "One page of invitations.", body = Page<InvitationResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.invitations.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_invitations(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<InvitationQuery>,
    Query(query): Query<PageQuery>,
) -> Result<Page<InvitationResponse>, Error> {
    principal.require(&INVITATIONS_MANAGE)?;
    let page = PageRequest::try_from(query)?;
    let found =
        state.invitations.list(&tenant, &principal, filter.status.as_deref(), &page).await?;
    Ok(found.map(InvitationResponse::from))
}

/// Invite someone by email.
///
/// Requires `auth.invitations.manage` at the unit. The mail (in the language of
/// `Accept-Language`) goes out shortly; the invitation can be accepted for
/// `AUTOXIDE_AUTH_INVITATION_DAYS` days.
#[utoipa::path(
    post,
    path = "/invitations",
    extensions(("x-permissions" = json!(["auth.invitations.manage"]))),
    operation_id = "create_invitation",
    security(("bearer" = [])),
    request_body = InvitationRequest,
    responses(
        (status = 201, description = "The invitation; its mail is on its way.", body = Data<InvitationResponse>, headers(("Location" = String, description = "Address of the invitation."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission (`permission_denied`), or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 422, description = "The email is invalid, a role does not exist, or a role has permissions you lack; see `errors`.", body = Problem),
    )
)]
async fn create_invitation(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    ValidJson(input): ValidJson<InvitationRequest>,
) -> Result<Created<InvitationResponse>, Error> {
    principal.require(&INVITATIONS_MANAGE)?;
    let input = NewInvitation {
        email: input.email,
        name: input.name,
        org_id: input.org_id,
        role_ids: input.role_ids,
        reference: input.reference,
    };
    let row = state.invitations.create(&tenant, Some(&principal), input, language).await?;
    let path = format!("/auth/invitations/{}", row.id);
    Ok(Created::new(path, row.into()))
}

/// Revoke an invitation.
///
/// Requires `auth.invitations.manage` at its unit. Its link stops working. Revoking again
/// changes nothing; an accepted invitation cannot be revoked.
#[utoipa::path(
    delete,
    path = "/invitations/{id}",
    extensions(("x-permissions" = json!(["auth.invitations.manage"]))),
    operation_id = "revoke_invitation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the invitation.")),
    responses(
        (status = 200, description = "The invitation, revoked.", body = Data<InvitationResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.invitations.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No invitation with this id in your scope; code `auth.invitation_not_found`.", body = Problem),
        (status = 409, description = "It was already accepted; code `auth.invitation_not_pending`.", body = Problem),
    )
)]
async fn revoke_invitation(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<InvitationResponse>, Error> {
    principal.require(&INVITATIONS_MANAGE)?;
    let row = state.invitations.revoke(&tenant, &principal, id).await?;
    Ok(Data { data: row.into() })
}

/// Send an invitation again.
///
/// Requires `auth.invitations.manage` at its unit. The old link stops working; the new one is
/// valid for `AUTOXIDE_AUTH_INVITATION_DAYS` days from now.
#[utoipa::path(
    post,
    path = "/invitations/{id}/resend",
    extensions(("x-permissions" = json!(["auth.invitations.manage"]))),
    operation_id = "resend_invitation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the invitation.")),
    responses(
        (status = 200, description = "The invitation; a new mail is on its way.", body = Data<InvitationResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.invitations.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No invitation with this id in your scope; code `auth.invitation_not_found`.", body = Problem),
        (status = 409, description = "It was already accepted or revoked; code `auth.invitation_not_pending`.", body = Problem),
    )
)]
async fn resend_invitation(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    Path(id): Path<Uuid>,
) -> Result<Data<InvitationResponse>, Error> {
    principal.require(&INVITATIONS_MANAGE)?;
    let row = state.invitations.resend(&tenant, &principal, id, language).await?;
    Ok(Data { data: row.into() })
}

/// Accept an invitation with the token from its mail.
///
/// No sign-in needed for a new account: it is created (its email counts as verified) and signed
/// in. When the email already has an account, sign in with it first and call this with your
/// access token: the roles are added to it. The token works once.
#[utoipa::path(
    post,
    path = "/invitations/accept",
    extensions(("x-public" = json!(true))),
    operation_id = "accept_invitation",
    security((), ("bearer" = [])),
    request_body = AcceptInvitationRequest,
    responses(
        (status = 200, description = "Accepted.", body = Data<AcceptedResponse>),
        (status = 400, description = "The invitation is wrong, used, revoked or expired; code `auth.invitation_invalid`.", body = Problem),
        (status = 403, description = "You are signed in with another account than the invited email; code `auth.invitation_email_mismatch`.", body = Problem),
        (status = 409, description = "The email already has an account: sign in, then accept; code `auth.invitation_sign_in_required`.", body = Problem),
        (status = 422, description = "The password breaks the rules: it must be 15 to 128 characters (more when the tenant raises `auth.password_min_length`) and not on the list of common or breached passwords (field code `common_password`); see `errors`.", body = Problem),
    )
)]
async fn accept_invitation(
    State(state): State<Arc<HttpState>>,
    OptionalTenant(tenant): OptionalTenant,
    principal: Option<Principal>,
    headers: HeaderMap,
    ValidJson(input): ValidJson<AcceptInvitationRequest>,
) -> Result<Data<AcceptedResponse>, Error> {
    let user_agent = super::user_agent(&headers);
    let accepted = state
        .invitations
        .accept(
            tenant.as_ref(),
            &input.token,
            input.name,
            input.password,
            principal.as_ref(),
            user_agent,
        )
        .await?;
    if let Some(tokens) = &accepted.tokens {
        super::note_country(&state, &accepted.tenant, &headers, tokens.session_id).await;
    }
    Ok(Data {
        data: AcceptedResponse {
            user_id: accepted.user_id,
            tokens: accepted.tokens.map(TokenResponse::from),
        },
    })
}
