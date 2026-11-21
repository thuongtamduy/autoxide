//! Đăng nhập qua nhà cung cấp danh tính (bước 40 P3): luồng đăng nhập (`/oidc/…`), liên kết của
//! chính chủ (`/me/identities…`) và quản trị nhà cung cấp của khách (`/oidc-providers…`).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::{Principal, Realm};
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{OptionalTenant, TenantContext};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use super::session::{MfaChallengeResponse, TokenResponse};
use crate::oidc::provider::{NewProvider, Provider, ProviderChanges};
use crate::oidc::{LinkedIdentity, Started};
use crate::permissions::{OIDC_MANAGE, OIDC_READ};
use crate::session::LoginOutcome;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(list_public_providers))
        .routes(routes!(start_oidc))
        .routes(routes!(finish_oidc))
        .routes(routes!(list_my_identities))
        .routes(routes!(start_identity_link))
        .routes(routes!(finish_identity_link))
        .routes(routes!(delete_my_identity))
        .routes(routes!(list_oidc_providers, create_oidc_provider))
        .routes(routes!(get_oidc_provider, update_oidc_provider, delete_oidc_provider))
}

/// A sign-in provider people of this tenant can use.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicProviderResponse {
    id: Uuid,
    /// Name in the sign-in paths, e.g. `google` in `/api/v1/auth/oidc/google/start`.
    slug: String,
    /// `google`, `microsoft`, `github` or `oidc`.
    kind: String,
    /// Label for the "Sign in with …" button.
    name: String,
}

/// Where the interface wants the provider to send the browser back.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartRequest {
    /// A page of the interface, exactly as listed in the provider's `redirect_uris`.
    #[validate(length(min = 1, max = 500))]
    #[schema(example = "https://app.example.com/sign-in/callback")]
    redirect_uri: String,
}

/// Where to send the browser.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct StartResponse {
    /// Open this address in the browser. The provider comes back to `redirect_uri` with `code`
    /// and `state` in the query.
    authorization_url: String,
    /// Seconds left to finish; after that, start again.
    expires_in: u64,
}

impl From<Started> for StartResponse {
    fn from(started: Started) -> Self {
        StartResponse {
            authorization_url: started.authorization_url,
            expires_in: started.expires_in,
        }
    }
}

/// What the provider sent back to `redirect_uri`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallbackRequest {
    /// The `code` query parameter.
    #[validate(length(min = 1, max = 2048))]
    code: String,
    /// The `state` query parameter, unchanged.
    #[validate(length(min = 1, max = 512))]
    state: String,
}

/// Start linking a provider to your account; proves it is you first.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinkStartRequest {
    /// A page of the interface, exactly as listed in the provider's `redirect_uris`.
    #[validate(length(min = 1, max = 500))]
    redirect_uri: String,
    /// Your current password.
    #[serde(default)]
    current_password: String,
    /// A confirmation token from `POST /api/v1/auth/me/reauth`, instead of `current_password`
    /// (accounts without a password use this). Exactly one of the two.
    reauth_token: Option<String>,
}

/// A sign-in with an outside provider linked to your account.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct IdentityResponse {
    id: Uuid,
    provider_id: Uuid,
    /// Slug of the provider; empty if the provider was removed meanwhile.
    provider: String,
    provider_name: String,
    /// `google`, `microsoft`, `github` or `oidc`.
    kind: String,
    /// The email the provider gave at the last sign-in.
    email: Option<String>,
    /// Whether the provider said it verified that email.
    email_verified: bool,
    created_at: Timestamp,
    last_login_at: Option<Timestamp>,
}

impl From<LinkedIdentity> for IdentityResponse {
    fn from(linked: LinkedIdentity) -> Self {
        let identity = linked.identity;
        IdentityResponse {
            id: identity.id,
            provider_id: identity.provider_id,
            provider: linked.provider_slug,
            provider_name: linked.provider_name,
            kind: linked.provider_kind,
            email: identity.email,
            email_verified: identity.email_verified,
            created_at: identity.created_at,
            last_login_at: identity.last_login_at,
        }
    }
}

/// A sign-in provider of the tenant, as administrators see it. The client secret is never shown.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct ProviderResponse {
    id: Uuid,
    slug: String,
    /// `google`, `microsoft`, `github` or `oidc`.
    kind: String,
    name: String,
    /// Issuer address; `null` for github.com.
    issuer: Option<String>,
    client_id: String,
    /// Whether a client secret is stored. The secret itself is never returned.
    has_secret: bool,
    /// Space-separated scopes asked for.
    scopes: String,
    /// Pages of the interface the provider may send the browser back to.
    redirect_uris: Vec<String>,
    /// Only emails at these domains may sign in; `null`: any.
    allowed_email_domains: Option<Vec<String>>,
    /// Whether someone without an account may get one by signing in.
    allow_signup: bool,
    /// `staff` or `customer`: which accounts sign in with it.
    realm: Realm,
    /// Unit where new accounts get `default_role_id`; `null`: the root unit.
    default_org_id: Option<Uuid>,
    /// Role new accounts get; `null`: none.
    default_role_id: Option<Uuid>,
    enabled: bool,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<Provider> for ProviderResponse {
    fn from(row: Provider) -> Self {
        ProviderResponse {
            has_secret: row.has_secret(),
            redirect_uris: row.redirect_uris(),
            allowed_email_domains: row.allowed_email_domains(),
            realm: row.realm(),
            id: row.id,
            slug: row.slug,
            kind: row.kind,
            name: row.name,
            issuer: row.issuer,
            client_id: row.client_id,
            scopes: row.scopes,
            allow_signup: row.allow_signup,
            default_org_id: row.default_org_id,
            default_role_id: row.default_role_id,
            enabled: row.enabled,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A new sign-in provider.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderRequest {
    /// Unique in the tenant: lowercase letters, digits and `-`, starting with a letter.
    #[schema(example = "google")]
    slug: String,
    /// `google`, `microsoft`, `github` or `oidc`.
    #[schema(example = "google")]
    kind: String,
    /// Label for the "Sign in with …" button, 1 to 100 characters.
    #[schema(example = "Google")]
    name: String,
    /// `oidc`: the issuer (required). `microsoft`: omit for any work or school account
    /// (`https://login.microsoftonline.com/organizations/v2.0`) or give
    /// `https://login.microsoftonline.com/<tenant id>/v2.0`. `github`: omit for github.com, or the
    /// address of a GitHub Enterprise Server. `google`: omit.
    issuer: Option<String>,
    /// The client (application) id registered at the provider.
    client_id: String,
    /// The client secret registered at the provider; write-only. Required except for `oidc`
    /// public clients.
    client_secret: Option<String>,
    /// Space-separated scopes; default `openid email profile` (`read:user user:email` for GitHub).
    scopes: Option<String>,
    /// Pages of the interface the provider may send the browser back to (1 to 20, exact match).
    redirect_uris: Vec<String>,
    /// Only emails at these domains may sign in; omit for any.
    allowed_email_domains: Option<Vec<String>>,
    /// Let someone without an account get one by signing in (still needs registration open, or
    /// `default_role_id`). Default `false`.
    #[serde(default)]
    allow_signup: bool,
    /// `staff` (default) or `customer`.
    #[serde(default)]
    #[schema(inline)]
    realm: Realm,
    /// Unit where new accounts get `default_role_id`; omit for the root unit.
    default_org_id: Option<Uuid>,
    /// Role new accounts get (staff only). You must hold every permission of it.
    default_role_id: Option<Uuid>,
    /// Default `true`.
    #[serde(default = "yes")]
    enabled: bool,
}

fn yes() -> bool {
    true
}

// Trường JSON có ba trạng thái: không gửi (`None`), `null` (`Some(None)`), có giá trị.
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Changes to a sign-in provider; fields left out stay as they are, `null` clears.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderPatch {
    name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    issuer: Option<Option<String>>,
    client_id: Option<String>,
    /// A new client secret (write-only).
    client_secret: Option<String>,
    scopes: Option<String>,
    redirect_uris: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Vec<String>>, nullable)]
    allowed_email_domains: Option<Option<Vec<String>>>,
    allow_signup: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    default_org_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    default_role_id: Option<Option<Uuid>>,
    enabled: Option<bool>,
}

impl ProviderRequest {
    pub(crate) fn into_new(self) -> NewProvider {
        NewProvider {
            slug: self.slug,
            kind: self.kind,
            name: self.name,
            issuer: self.issuer,
            client_id: self.client_id,
            client_secret: self.client_secret,
            scopes: self.scopes,
            redirect_uris: self.redirect_uris,
            allowed_email_domains: self.allowed_email_domains,
            allow_signup: self.allow_signup,
            realm: self.realm,
            default_org_id: self.default_org_id,
            default_role_id: self.default_role_id,
            enabled: self.enabled,
        }
    }
}

impl ProviderPatch {
    pub(crate) fn into_changes(self) -> ProviderChanges {
        ProviderChanges {
            name: self.name,
            issuer: self.issuer,
            client_id: self.client_id,
            client_secret: self.client_secret,
            scopes: self.scopes,
            redirect_uris: self.redirect_uris,
            allowed_email_domains: self.allowed_email_domains,
            allow_signup: self.allow_signup,
            default_org_id: self.default_org_id,
            default_role_id: self.default_role_id,
            enabled: self.enabled,
        }
    }
}

/// List the sign-in providers of this tenant.
///
/// No sign-in needed. Lists the enabled providers for "Sign in with …" buttons; start with
/// `POST /api/v1/auth/oidc/{slug}/start`. Off unless the server turns it on (`AUTOXIDE_AUTH_OIDC`).
#[utoipa::path(
    get,
    path = "/oidc/providers",
    extensions(("x-public" = json!(true))),
    operation_id = "list_public_oidc_providers",
    responses(
        (status = 200, description = "The enabled providers.", body = Data<Vec<PublicProviderResponse>>),
        (status = 404, description = "Outside sign-in is off on this server; code `auth.oidc_disabled`.", body = Problem),
    )
)]
async fn list_public_providers(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
) -> Result<Data<Vec<PublicProviderResponse>>, Error> {
    let providers = state.oidc.public_providers(&tenant).await?;
    let data = providers
        .into_iter()
        .map(|row| PublicProviderResponse {
            id: row.id,
            slug: row.slug,
            kind: row.kind,
            name: row.name,
        })
        .collect();
    Ok(Data { data })
}

/// Start signing in with a provider.
///
/// No sign-in needed. Returns the provider's address to open in the browser (with PKCE, `state`
/// and `nonce` kept on the server for 10 minutes). The provider comes back to `redirect_uri`
/// with `code` and `state`; finish with `POST /api/v1/auth/oidc/{slug}/callback`.
#[utoipa::path(
    post,
    path = "/oidc/{slug}/start",
    extensions(("x-public" = json!(true))),
    operation_id = "start_oidc_sign_in",
    params(("slug" = String, Path, description = "Slug of the provider.")),
    request_body = StartRequest,
    responses(
        (status = 200, description = "Open `authorization_url` in the browser.", body = Data<StartResponse>),
        (status = 404, description = "Outside sign-in is off (code `auth.oidc_disabled`), or no enabled provider has this slug (code `auth.oidc_provider_not_found`).", body = Problem),
        (status = 422, description = "`redirect_uri` is not in the provider's list (code `auth.oidc_redirect_not_allowed`), or the body is invalid.", body = Problem),
        (status = 503, description = "The provider could not be reached; code `auth.oidc_provider_unavailable`.", body = Problem),
    )
)]
async fn start_oidc(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    Path(slug): Path<String>,
    ValidJson(input): ValidJson<StartRequest>,
) -> Result<Data<StartResponse>, Error> {
    let started = state.oidc.start(&tenant, &slug, &input.redirect_uri, None).await?;
    Ok(Data { data: started.into() })
}

/// Finish signing in with a provider.
///
/// No sign-in needed; the `state` names the tenant. The `state` works once. Answers like
/// `POST /api/v1/auth/login`: tokens, or 202 with an `mfa_token` when two-step sign-in is on. An
/// existing account is linked only when both it and the provider have verified the email.
#[utoipa::path(
    post,
    path = "/oidc/{slug}/callback",
    extensions(("x-public" = json!(true))),
    operation_id = "finish_oidc_sign_in",
    params(("slug" = String, Path, description = "Slug of the provider.")),
    request_body = CallbackRequest,
    responses(
        (status = 200, description = "Signed in.", body = Data<TokenResponse>),
        (status = 202, description = "A code is needed; continue with `POST /api/v1/auth/login/mfa`.", body = Data<MfaChallengeResponse>),
        (status = 400, description = "The `state` is unknown, used or expired; code `auth.oidc_state_invalid`. Start again.", body = Problem),
        (status = 401, description = "The provider's answer is not valid (signature, issuer, audience, expiry, nonce); code `auth.oidc_token_invalid`, `reason` says which.", body = Problem),
        (status = 403, description = "The provider has not verified the email (code `auth.oidc_email_unverified`), its domain is not allowed (code `auth.oidc_email_domain_not_allowed`), nobody may create an account this way (code `auth.oidc_signup_closed`), or the account is disabled (code `auth.user_disabled`).", body = Problem),
        (status = 404, description = "Outside sign-in is off (code `auth.oidc_disabled`), or no enabled provider has this slug (code `auth.oidc_provider_not_found`).", body = Problem),
        (status = 409, description = "An account has this email but one side has not verified it; code `auth.oidc_link_required`. Sign in the usual way and link the provider in the account.", body = Problem),
        (status = 422, description = "The body is missing a field.", body = Problem),
        (status = 503, description = "The provider could not be reached; code `auth.oidc_provider_unavailable`.", body = Problem),
    )
)]
async fn finish_oidc(
    State(state): State<Arc<HttpState>>,
    OptionalTenant(tenant): OptionalTenant,
    Path(slug): Path<String>,
    headers: HeaderMap,
    ValidJson(input): ValidJson<CallbackRequest>,
) -> Result<Response, Error> {
    let user_agent = super::user_agent(&headers);
    let (tenant, outcome) =
        state.oidc.callback(tenant.as_ref(), &slug, &input.code, &input.state, user_agent).await?;
    Ok(match outcome {
        LoginOutcome::Tokens(tokens) => {
            super::note_country(&state, &tenant, &headers, tokens.session_id).await;
            Data { data: TokenResponse::from(tokens) }.into_response()
        }
        LoginOutcome::MfaRequired { mfa_token, expires_in } => {
            let data = MfaChallengeResponse::new(mfa_token, expires_in);
            (StatusCode::ACCEPTED, Data { data }).into_response()
        }
    })
}

/// List the outside sign-ins linked to your account.
///
/// Any signed-in user, for their own account.
#[utoipa::path(
    get,
    path = "/me/identities",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_identities",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Your linked sign-ins, oldest first.", body = Data<Vec<IdentityResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn list_my_identities(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<IdentityResponse>>, Error> {
    let linked = state.oidc.identities(&tenant, principal.user_id()).await?;
    Ok(Data { data: linked.into_iter().map(Into::into).collect() })
}

/// Start linking a provider to your account.
///
/// Needs a session (not an API key) and your current password or a `reauth_token`. Then open
/// `authorization_url` and finish with `POST /api/v1/auth/me/identities/{slug}/callback`.
#[utoipa::path(
    post,
    path = "/me/identities/{slug}/start",
    extensions(("x-permissions" = json!([]))),
    operation_id = "start_identity_link",
    security(("bearer" = [])),
    params(("slug" = String, Path, description = "Slug of the provider.")),
    request_body = LinkStartRequest,
    responses(
        (status = 200, description = "Open `authorization_url` in the browser.", body = Data<StartResponse>),
        (status = 400, description = "The current password is wrong; code `auth.wrong_password`.", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Called with an API key (code `auth.session_required`), or the `reauth_token` is not valid (code `auth.reauth_token_invalid`).", body = Problem),
        (status = 404, description = "Outside sign-in is off (code `auth.oidc_disabled`), or no enabled provider has this slug (code `auth.oidc_provider_not_found`).", body = Problem),
        (status = 422, description = "`redirect_uri` is not in the provider's list (code `auth.oidc_redirect_not_allowed`), or the body is invalid.", body = Problem),
        (status = 429, description = "Too many wrong passwords; code `auth.login_throttled`.", body = Problem),
        (status = 503, description = "The provider could not be reached; code `auth.oidc_provider_unavailable`.", body = Problem),
    )
)]
async fn start_identity_link(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(slug): Path<String>,
    ValidJson(input): ValidJson<LinkStartRequest>,
) -> Result<Data<StartResponse>, Error> {
    super::require_session(&principal)?;
    state.oidc.require_enabled()?;
    let confirm = super::confirm(&input.current_password, input.reauth_token.as_deref())?;
    confirm.check(&state.sessions, &tenant, principal.user_id()).await?;
    let user = Some(principal.user_id());
    let started = state.oidc.start(&tenant, &slug, &input.redirect_uri, user).await?;
    Ok(Data { data: started.into() })
}

/// Finish linking a provider to your account.
///
/// Needs the same session that started it. The `state` works once.
#[utoipa::path(
    post,
    path = "/me/identities/{slug}/callback",
    extensions(("x-permissions" = json!([]))),
    operation_id = "finish_identity_link",
    security(("bearer" = [])),
    params(("slug" = String, Path, description = "Slug of the provider.")),
    request_body = CallbackRequest,
    responses(
        (status = 201, description = "Linked; you can now sign in with this provider.", body = Data<IdentityResponse>),
        (status = 400, description = "The `state` is unknown, used, expired or not yours; code `auth.oidc_state_invalid`.", body = Problem),
        (status = 401, description = "Not signed in, or the provider's answer is not valid (code `auth.oidc_token_invalid`).", body = Problem),
        (status = 403, description = "Called with an API key (code `auth.session_required`), or the email domain is not allowed (code `auth.oidc_email_domain_not_allowed`).", body = Problem),
        (status = 404, description = "Outside sign-in is off (code `auth.oidc_disabled`), or no enabled provider has this slug (code `auth.oidc_provider_not_found`).", body = Problem),
        (status = 409, description = "This sign-in is linked to another account; code `auth.identity_taken`.", body = Problem),
        (status = 422, description = "The body is missing a field.", body = Problem),
        (status = 503, description = "The provider could not be reached; code `auth.oidc_provider_unavailable`.", body = Problem),
    )
)]
async fn finish_identity_link(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(slug): Path<String>,
    ValidJson(input): ValidJson<CallbackRequest>,
) -> Result<Created<IdentityResponse>, Error> {
    super::require_session(&principal)?;
    let linked = state
        .oidc
        .link_callback(&tenant, principal.user_id(), &slug, &input.code, &input.state)
        .await?;
    Ok(Created::new("/auth/me/identities", IdentityResponse::from(linked)))
}

/// Remove an outside sign-in from your account.
///
/// Needs a session (not an API key). Your last way to sign in cannot be removed: set a password
/// or add a passkey first.
#[utoipa::path(
    delete,
    path = "/me/identities/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "delete_my_identity",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the linked sign-in.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Called with an API key; code `auth.session_required`.", body = Problem),
        (status = 404, description = "You have no linked sign-in with this id; code `auth.identity_not_found`.", body = Problem),
        (status = 409, description = "It is your last way to sign in; code `auth.identity_last_method`.", body = Problem),
    )
)]
async fn delete_my_identity(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    super::require_session(&principal)?;
    state.oidc.unlink(&tenant, principal.user_id(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List the sign-in providers of this tenant, for administrators.
///
/// Requires `auth.oidc.read`. Includes disabled providers; never the client secrets.
#[utoipa::path(
    get,
    path = "/oidc-providers",
    extensions(("x-permissions" = json!(["auth.oidc.read"]))),
    operation_id = "list_oidc_providers",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every provider, oldest first.", body = Data<Vec<ProviderResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.oidc.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_oidc_providers(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<ProviderResponse>>, Error> {
    principal.require(&OIDC_READ)?;
    let rows = state.oidc.providers.list(&tenant).await?;
    Ok(Data { data: rows.into_iter().map(Into::into).collect() })
}

/// Add a sign-in provider to this tenant.
///
/// Requires `auth.oidc.manage`. Register the application at the provider first and list the
/// interface's callback pages in `redirect_uris`. The client secret is stored encrypted and never
/// shown again.
#[utoipa::path(
    post,
    path = "/oidc-providers",
    extensions(("x-permissions" = json!(["auth.oidc.manage"]))),
    operation_id = "create_oidc_provider",
    security(("bearer" = [])),
    request_body = ProviderRequest,
    responses(
        (status = 201, description = "The provider was added.", body = Data<ProviderResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.oidc.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "Another provider has this slug; code `auth.oidc_slug_taken`.", body = Problem),
        (status = 422, description = "A value is invalid, e.g. an issuer on a private network; see `errors`.", body = Problem),
    )
)]
async fn create_oidc_provider(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ProviderRequest>,
) -> Result<Created<ProviderResponse>, Error> {
    principal.require(&OIDC_MANAGE)?;
    let oidc = &state.oidc;
    let row = oidc.providers.create(&tenant, &principal, &oidc.client, input.into_new()).await?;
    let path = format!("/auth/oidc-providers/{}", row.id);
    Ok(Created::new(path, ProviderResponse::from(row)))
}

/// Get a sign-in provider of this tenant.
///
/// Requires `auth.oidc.read`.
#[utoipa::path(
    get,
    path = "/oidc-providers/{id}",
    extensions(("x-permissions" = json!(["auth.oidc.read"]))),
    operation_id = "get_oidc_provider",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the provider.")),
    responses(
        (status = 200, description = "The provider.", body = Data<ProviderResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.oidc.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No provider has this id; code `auth.oidc_provider_not_found`.", body = Problem),
    )
)]
async fn get_oidc_provider(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ProviderResponse>, Error> {
    principal.require(&OIDC_READ)?;
    Ok(Data { data: state.oidc.providers.get(&tenant, id).await?.into() })
}

/// Change a sign-in provider of this tenant.
///
/// Requires `auth.oidc.manage`. Fields left out stay as they are; `null` clears `issuer`,
/// `allowed_email_domains`, `default_org_id` or `default_role_id`. `slug`, `kind` and `realm`
/// cannot change.
#[utoipa::path(
    patch,
    path = "/oidc-providers/{id}",
    extensions(("x-permissions" = json!(["auth.oidc.manage"]))),
    operation_id = "update_oidc_provider",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the provider.")),
    request_body = ProviderPatch,
    responses(
        (status = 200, description = "The changed provider.", body = Data<ProviderResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.oidc.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No provider has this id; code `auth.oidc_provider_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_oidc_provider(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ProviderPatch>,
) -> Result<Data<ProviderResponse>, Error> {
    principal.require(&OIDC_MANAGE)?;
    let oidc = &state.oidc;
    let row =
        oidc.providers.update(&tenant, &principal, &oidc.client, id, input.into_changes()).await?;
    Ok(Data { data: row.into() })
}

/// Remove a sign-in provider from this tenant.
///
/// Requires `auth.oidc.manage`. Every account link to it goes too; people who signed in only
/// with it reset their password by email.
#[utoipa::path(
    delete,
    path = "/oidc-providers/{id}",
    extensions(("x-permissions" = json!(["auth.oidc.manage"]))),
    operation_id = "delete_oidc_provider",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the provider.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.oidc.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No provider has this id; code `auth.oidc_provider_not_found`.", body = Problem),
    )
)]
async fn delete_oidc_provider(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&OIDC_MANAGE)?;
    state.oidc.providers.delete(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
