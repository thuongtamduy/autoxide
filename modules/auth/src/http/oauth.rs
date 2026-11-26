//! Endpoint của máy chủ uỷ quyền OAuth 2.1 cho client MCP (bước 71 G4, D-20261126-mcp-oauth).
//!
//! `/oauth/authorize`, `/oauth/token`, `/oauth/revoke`, `/oauth/register` theo đúng wire format của RFC (query,
//! form-urlencoded, JSON trần, lỗi `{error, error_description}`) — ngoại lệ có chủ đích với envelope và Problem
//! (D-20261126-mcp-oauth mục 10). API cho trang consent và quản lý grant theo quy ước chung của Autoxide.

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::HttpState;
use crate::oauth::{
    AuthorizeQuery, Decision, DecisionResult, GrantView, OAuthError, OAuthService, Registration,
    RegistrationResponse, RequestView, RevokeForm, TokenForm, TokenResponse,
};
use crate::permissions::OAUTH_GRANTS_MANAGE;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(authorize))
        .routes(routes!(token))
        .routes(routes!(revoke))
        .routes(routes!(register))
        .routes(routes!(describe_request))
        .routes(routes!(approve_request))
        .routes(routes!(deny_request))
        .routes(routes!(list_my_grants))
        .routes(routes!(revoke_my_grant))
        .routes(routes!(list_grants))
        .routes(routes!(revoke_grant))
}

fn service(state: &HttpState) -> Result<&Arc<OAuthService>, Error> {
    state.oauth.as_ref().ok_or_else(crate::oauth::disabled)
}

fn oauth_service(state: &HttpState) -> Result<&Arc<OAuthService>, OAuthError> {
    state.oauth.as_ref().ok_or_else(OAuthError::unavailable)
}

/// Start connecting an MCP / OAuth client (OAuth 2.1 authorization endpoint).
///
/// Opened in the browser by the client. Needs no sign-in. Checks the client (a client metadata document URL or a
/// registered `client_id`) and `redirect_uri` first: when either is wrong the answer is a 400 here, never a
/// redirect. Then checks the rest (`response_type=code`, PKCE `S256`, `resource`, `scope`): when wrong, redirects
/// back to the client with `error`, `state` and `iss`. When valid, redirects to the interface's consent page
/// (`AUTOXIDE_AUTH_OAUTH_CONSENT_URL`) with the request token.
#[utoipa::path(
    get,
    path = "/oauth/authorize",
    extensions(("x-public" = json!(true))),
    operation_id = "oauth_authorize",
    params(AuthorizeQuery),
    responses(
        (status = 302, description = "Redirect to the consent page, or back to the client with `error`, `state` and `iss`."),
        (status = 400, description = "The client is unknown or invalid (code `auth.oauth_client_invalid`), or `redirect_uri` is not one it registered (code `auth.oauth_redirect_invalid`).", body = Problem),
        (status = 404, description = "OAuth sign-in is off on this server; code `auth.oauth_disabled`.", body = Problem),
        (status = 429, description = "Too many requests from this address; code `auth.oauth_rate_limited`.", body = Problem),
    )
)]
async fn authorize(
    State(state): State<Arc<HttpState>>,
    client: ClientInfo,
    Query(query): Query<AuthorizeQuery>,
) -> Result<Response, Error> {
    let location = service(&state)?.authorize(query, client.ip).await?;
    let mut response = StatusCode::FOUND.into_response();
    let location = HeaderValue::from_str(&location).map_err(Error::internal)?;
    response.headers_mut().insert(header::LOCATION, location);
    Ok(response)
}

/// Exchange a code or a refresh token for tokens (OAuth 2.1 token endpoint).
///
/// Form-encoded body (`application/x-www-form-urlencoded`). `grant_type=authorization_code` with `code`,
/// `code_verifier`, `redirect_uri`, `client_id` (and `resource`); `grant_type=refresh_token` with `refresh_token`
/// and `client_id`. Every refresh returns a new refresh token; using an old one again revokes the approval. Errors
/// follow RFC 6749 §5.2: `{"error": …, "error_description": …}`.
#[utoipa::path(
    post,
    path = "/oauth/token",
    extensions(("x-public" = json!(true))),
    operation_id = "oauth_token",
    request_body(content = TokenForm, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, description = "The new access token and refresh token.", body = TokenResponse),
        (status = 400, description = "RFC 6749 error: `invalid_request`, `invalid_grant` (code or refresh token unknown, expired, used; PKCE mismatch), `invalid_target`, `unsupported_grant_type`.", body = crate::oauth::ErrorBody),
        (status = 404, description = "OAuth sign-in is off on this server (`invalid_request`).", body = crate::oauth::ErrorBody),
    )
)]
async fn token(State(state): State<Arc<HttpState>>, body: Bytes) -> Response {
    let outcome = async {
        let service = oauth_service(&state)?;
        let form = form_of::<TokenForm>(&body)?;
        service.token(form).await
    }
    .await;
    match outcome {
        Ok(tokens) => {
            let mut response = axum::Json(tokens).into_response();
            response.headers_mut().insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
            response
        }
        Err(error) => error.into_response(),
    }
}

/// Revoke a token (RFC 7009).
///
/// Form-encoded `token` (access or refresh token) and `client_id`. Revokes the whole approval the token belongs
/// to. Answers 200 even for an unknown token.
#[utoipa::path(
    post,
    path = "/oauth/revoke",
    extensions(("x-public" = json!(true))),
    operation_id = "oauth_revoke",
    request_body(content = RevokeForm, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, description = "Revoked (or nothing to revoke)."),
        (status = 400, description = "RFC 6749 error: `invalid_request` (no `token`).", body = crate::oauth::ErrorBody),
    )
)]
async fn revoke(State(state): State<Arc<HttpState>>, body: Bytes) -> Response {
    let outcome = async {
        let service = oauth_service(&state)?;
        let form = form_of::<RevokeForm>(&body)?;
        let token = form.token.ok_or_else(|| OAuthError::invalid_request("token is missing."))?;
        service.revoke_token(&token, form.client_id.as_deref()).await
    }
    .await;
    match outcome {
        Ok(()) => StatusCode::OK.into_response(),
        Err(error) => error.into_response(),
    }
}

/// Register an MCP / OAuth client (RFC 7591, deprecated).
///
/// For clients that cannot use a client metadata document (MCP 2026-07-28 deprecates dynamic registration). Only
/// public clients (`token_endpoint_auth_method` `none`, PKCE). Unknown fields are ignored. Errors follow RFC 7591
/// §3.2.2.
#[utoipa::path(
    post,
    path = "/oauth/register",
    extensions(("x-public" = json!(true))),
    operation_id = "oauth_register",
    request_body = Registration,
    responses(
        (status = 201, description = "The registered client; use `client_id` in the authorization request.", body = RegistrationResponse),
        (status = 400, description = "RFC 7591 error: `invalid_redirect_uri`, `invalid_client_metadata`.", body = crate::oauth::ErrorBody),
        (status = 404, description = "OAuth sign-in or dynamic registration is off on this server.", body = crate::oauth::ErrorBody),
        (status = 429, description = "Too many registrations from this address (`slow_down`).", body = crate::oauth::ErrorBody),
    )
)]
async fn register(
    State(state): State<Arc<HttpState>>,
    client: ClientInfo,
    body: Bytes,
) -> Response {
    let outcome = async {
        let service = oauth_service(&state)?;
        if !service.config.dynamic_registration {
            return Err(OAuthError::unavailable());
        }
        let input: Registration = serde_json::from_slice(&body).map_err(|_| {
            OAuthError::new(
                StatusCode::BAD_REQUEST,
                "invalid_client_metadata",
                "The body is not a JSON object.",
            )
        })?;
        service.register(input, client.ip).await
    }
    .await;
    match outcome {
        Ok(client) => {
            let body = RegistrationResponse::from(client);
            (StatusCode::CREATED, axum::Json(body)).into_response()
        }
        Err(error) => error.into_response(),
    }
}

/// Read an authorization request waiting for your decision.
///
/// For the interface's consent page. Signed in with a password session only. Show `client_host` and
/// `redirect_host` (the client chose its name itself), warn when `localhost_redirect` is true, and let the person
/// choose permissions from `permissions`.
#[utoipa::path(
    get,
    path = "/oauth/requests/{token}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "describe_oauth_request",
    security(("bearer" = [])),
    params(("token" = String, Path, description = "The request token the consent page received.")),
    responses(
        (status = 200, description = "The request.", body = Data<RequestView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not a password session (code `auth.session_required`) or not a staff account (code `auth.oauth_staff_only`).", body = Problem),
        (status = 404, description = "No such request, or it expired (code `auth.oauth_request_not_found`); or OAuth is off (code `auth.oauth_disabled`).", body = Problem),
    )
)]
async fn describe_request(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(token): Path<String>,
) -> Result<Data<RequestView>, Error> {
    Ok(Data { data: service(&state)?.describe(&tenant, &principal, &token).await? })
}

/// Approve a client: grant it the permissions you choose.
///
/// Signed in with a password session only. Permissions that can delete data, touch money or read personal data
/// need `reauth_token` from `POST /api/v1/auth/me/reauth`. Answers where to send the browser: the client's
/// `redirect_uri` with the code. The client then never has more than you currently have.
#[utoipa::path(
    post,
    path = "/oauth/requests/{token}/approve",
    extensions(("x-permissions" = json!([]))),
    operation_id = "approve_oauth_request",
    security(("bearer" = [])),
    params(("token" = String, Path, description = "The request token the consent page received.")),
    request_body = Decision,
    responses(
        (status = 200, description = "Approved; open `redirect_to` in the browser.", body = Data<DecisionResult>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not a password session (code `auth.session_required`) or not a staff account (code `auth.oauth_staff_only`).", body = Problem),
        (status = 404, description = "No such request, it expired or was already decided (code `auth.oauth_request_not_found`).", body = Problem),
        (status = 422, description = "A permission is not one you can grant, `days` is out of range, or `reauth_token` is missing; see `errors`.", body = Problem),
    )
)]
async fn approve_request(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(token): Path<String>,
    ValidJson(decision): ValidJson<Decision>,
) -> Result<Data<DecisionResult>, Error> {
    let redirect_to = service(&state)?.approve(&tenant, &principal, &token, decision).await?;
    Ok(Data { data: DecisionResult { redirect_to } })
}

/// Decline a client.
///
/// Signed in with a password session only. Answers where to send the browser: the client's `redirect_uri` with
/// `error=access_denied`.
#[utoipa::path(
    post,
    path = "/oauth/requests/{token}/deny",
    extensions(("x-permissions" = json!([]))),
    operation_id = "deny_oauth_request",
    security(("bearer" = [])),
    params(("token" = String, Path, description = "The request token the consent page received.")),
    responses(
        (status = 200, description = "Declined; open `redirect_to` in the browser.", body = Data<DecisionResult>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not a password session (code `auth.session_required`) or not a staff account (code `auth.oauth_staff_only`).", body = Problem),
        (status = 404, description = "No such request, it expired or was already decided (code `auth.oauth_request_not_found`).", body = Problem),
    )
)]
async fn deny_request(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Path(token): Path<String>,
) -> Result<Data<DecisionResult>, Error> {
    let redirect_to = service(&state)?.deny(&principal, &token).await?;
    Ok(Data { data: DecisionResult { redirect_to } })
}

/// List the MCP / OAuth clients you approved.
///
/// Newest first, including revoked and expired approvals.
#[utoipa::path(
    get,
    path = "/me/oauth-grants",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_oauth_grants",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of your approvals, newest first.", body = Page<GrantView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "OAuth sign-in is off on this server; code `auth.oauth_disabled`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_my_grants(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<GrantView>, Error> {
    let page = PageRequest::try_from(page)?;
    service(&state)?.list(&tenant, Some(principal.user_id()), &page).await
}

/// Revoke a client you approved.
///
/// Signed in with a password session only. Its tokens stop working at once. Safe to repeat.
#[utoipa::path(
    delete,
    path = "/me/oauth-grants/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "revoke_my_oauth_grant",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "ID of the approval.")),
    responses(
        (status = 200, description = "Revoked; the approval as it is now.", body = Data<GrantView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not a password session; code `auth.session_required`.", body = Problem),
        (status = 404, description = "You have no such approval; code `auth.oauth_grant_not_found`.", body = Problem),
    )
)]
async fn revoke_my_grant(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<GrantView>, Error> {
    Ok(Data { data: service(&state)?.revoke(&tenant, &principal, id, true).await? })
}

/// List every approval people gave to MCP / OAuth clients.
///
/// Requires `auth.oauth_grants.manage`. Newest first, including revoked and expired approvals.
#[utoipa::path(
    get,
    path = "/oauth-grants",
    extensions(("x-permissions" = json!(["auth.oauth_grants.manage"]))),
    operation_id = "list_oauth_grants",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of approvals, newest first.", body = Page<GrantView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.oauth_grants.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "OAuth sign-in is off on this server; code `auth.oauth_disabled`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_grants(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<GrantView>, Error> {
    principal.require(&OAUTH_GRANTS_MANAGE)?;
    let page = PageRequest::try_from(page)?;
    service(&state)?.list(&tenant, None, &page).await
}

/// Revoke any approval of an MCP / OAuth client.
///
/// Requires `auth.oauth_grants.manage`, signed in with a password session. Its tokens stop working at once. Safe
/// to repeat.
#[utoipa::path(
    delete,
    path = "/oauth-grants/{id}",
    extensions(("x-permissions" = json!(["auth.oauth_grants.manage"]))),
    operation_id = "revoke_oauth_grant",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "ID of the approval.")),
    responses(
        (status = 200, description = "Revoked; the approval as it is now.", body = Data<GrantView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.oauth_grants.manage` (code `permission_denied`), or not a password session (code `auth.session_required`).", body = Problem),
        (status = 404, description = "No such approval; code `auth.oauth_grant_not_found`.", body = Problem),
    )
)]
async fn revoke_grant(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<GrantView>, Error> {
    principal.require(&OAUTH_GRANTS_MANAGE)?;
    Ok(Data { data: service(&state)?.revoke(&tenant, &principal, id, false).await? })
}

/// Thân `application/x-www-form-urlencoded` → kiểu `T` (trường lặp lại: lỗi, OAuth 2.1 §3.1).
fn form_of<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, OAuthError> {
    let invalid =
        || OAuthError::invalid_request("The body must be application/x-www-form-urlencoded.");
    let text = std::str::from_utf8(body).map_err(|_| invalid())?;
    let mut fields = HashMap::new();
    for pair in text.split('&').filter(|pair| !pair.is_empty()) {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        let (name, value) = (decode(name).ok_or_else(invalid)?, decode(value).ok_or_else(invalid)?);
        if fields.insert(name.clone(), value).is_some() {
            return Err(OAuthError::invalid_request(format!("`{name}` is repeated.")));
        }
    }
    serde_json::from_value(json!(fields)).map_err(|_| invalid())
}

/// Giải mã một thành phần form: `+` là dấu cách, `%XX` là một byte; kết quả phải là UTF-8.
fn decode(raw: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(raw.len());
    let mut input = raw.bytes();
    while let Some(byte) = input.next() {
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let high = (input.next()? as char).to_digit(16)?;
                let low = (input.next()? as char).to_digit(16)?;
                bytes.push(u8::try_from(high * 16 + low).ok()?);
            }
            other => bytes.push(other),
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_form_urlencoded() {
        let form: TokenForm =
            form_of(b"grant_type=authorization_code&code=axc1_x&redirect_uri=http%3A%2F%2F127.0.0.1%3A1%2Fcb&scope=a+b")
                .unwrap();
        assert_eq!(form.grant_type.as_deref(), Some("authorization_code"));
        assert_eq!(form.redirect_uri.as_deref(), Some("http://127.0.0.1:1/cb"));
        assert_eq!(form.scope.as_deref(), Some("a b"));
        assert!(form_of::<TokenForm>(b"code=a&code=b").is_err());
        assert!(form_of::<TokenForm>(b"code=%zz").is_err());
    }
}
