//! OAuth 2.1 protected resource (step 71 G4, D-20261126-mcp-oauth): the discovery documents of RFC 9728 and
//! RFC 8414, the `resource_metadata` parameter of `WWW-Authenticate` (MCP authorization 2026-07-28), and the
//! audience check of OAuth access tokens (RFC 8707).
//!
//! The authorization server itself lives in a module (usually `auth`), which publishes
//! [`AuthorizationServer`](crate::access::AuthorizationServer). Without it, or without
//! `AUTOXIDE_HTTP_PUBLIC_URL`, the documents answer 404 and challenges stay a bare `Bearer`.
//!
//! Resources (canonical, no trailing slash): `{origin}` (everything), `{origin}/mcp` (every MCP endpoint),
//! `{origin}/mcp/<module>` and `{origin}/api/v1` (REST). `origin` is the public URL, or `https://<domain>` when
//! the request arrived on a tenant's verified domain.

use std::sync::Arc;

use axum::extract::{Path, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::API_PREFIX;
use super::mcp::MCP_PATH;
use crate::access::{AuthorizationServer, Principal};
use crate::error::Error;
use crate::tenancy::{DatabaseRegistry, TenantHint};

/// Where the protected resource metadata documents live (RFC 9728 §3).
pub const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource";
/// Where the authorization server metadata lives (RFC 8414 §3).
pub const AUTHORIZATION_SERVER_PATH: &str = "/.well-known/oauth-authorization-server";

/// What the resource server needs to know, shared by the documents, the challenge layer and the audience check.
pub(crate) struct ProtectedResources {
    public_url: Option<String>,
    registry: Option<DatabaseRegistry>,
    server: Option<Arc<dyn AuthorizationServer>>,
    /// Modules with an MCP endpoint (`/mcp/<module>`).
    mcp_modules: Vec<String>,
}

/// Placed in every request's extensions: the audience check in authentication reads it.
#[derive(Clone)]
pub(crate) struct Resources(pub(crate) Arc<ProtectedResources>);

impl ProtectedResources {
    pub(crate) fn new(
        public_url: Option<String>,
        registry: Option<DatabaseRegistry>,
        server: Option<Arc<dyn AuthorizationServer>>,
        mcp_modules: Vec<String>,
    ) -> Self {
        ProtectedResources { public_url, registry, server, mcp_modules }
    }

    /// OAuth is on: a module issues tokens and the public address is known.
    fn enabled(&self) -> bool {
        self.server.is_some() && self.public_url.is_some()
    }

    /// The origin (issuer and resource prefix) for a request to `host`: the tenant's verified domain, otherwise
    /// the public URL. An unknown `Host` never becomes an issuer (no reflection of attacker-chosen hosts).
    async fn origin(&self, host: Option<&str>) -> Result<Option<String>, Error> {
        let Some(public) = &self.public_url else { return Ok(None) };
        if let (Some(host), Some(registry)) = (host, &self.registry)
            && !same_host(public, host)
            && registry.tenant_by_domain(host).await?.is_some()
        {
            return Ok(Some(format!("https://{host}")));
        }
        Ok(Some(public.clone()))
    }

    /// The resource path (after the origin) a request path belongs to: `/mcp`, `/mcp/<module>`, `/api/v1`, or
    /// `""` (the whole server) for anything else.
    fn resource_path<'a>(&self, path: &'a str) -> &'a str {
        if let Some(rest) = path.strip_prefix(MCP_PATH) {
            if rest.is_empty() || rest == "/" {
                return MCP_PATH;
            }
            if let Some(module) = rest.strip_prefix('/')
                && self.mcp_modules.iter().any(|known| known == module.trim_end_matches('/'))
            {
                return &path[..MCP_PATH.len() + 1 + module.trim_end_matches('/').len()];
            }
            return MCP_PATH;
        }
        if path == API_PREFIX || path.starts_with(&format!("{API_PREFIX}/")) {
            return API_PREFIX;
        }
        ""
    }
}

/// `https://api.example.com:8443` and `api.example.com` (normalized `Host`, no port) are the same host.
fn same_host(public: &str, host: &str) -> bool {
    let authority = public.split_once("://").map_or(public, |(_, rest)| rest);
    let name = match authority.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default(),
        None => authority.rsplit_once(':').map_or(authority, |(name, _)| name),
    };
    name.eq_ignore_ascii_case(host)
}

fn host_of(parts: &Parts) -> Option<String> {
    TenantHint::of_request(&parts.headers, &parts.extensions).host().map(str::to_owned)
}

/// Rejects an OAuth access token used outside the resource it was issued for (RFC 8707, MCP: servers MUST
/// validate the audience). Credentials without an audience (sessions, API keys) pass.
pub(crate) async fn admit_audience(parts: &Parts, principal: &Principal) -> Result<(), Error> {
    let Some(access) = principal.oauth() else { return Ok(()) };
    let resources = parts.extensions.get::<Resources>().map(|resources| resources.0.clone());
    let origin = match &resources {
        Some(resources) if resources.enabled() => {
            resources.origin(host_of(parts).as_deref()).await?
        }
        _ => None,
    };
    let (Some(resources), Some(origin)) = (resources, origin) else {
        return Err(wrong_audience());
    };
    // Router lồng (`nest`) cắt tiền tố khỏi `uri`; `OriginalUri` giữ đường dẫn đầy đủ.
    let path = parts
        .extensions
        .get::<axum::extract::OriginalUri>()
        .map_or_else(|| parts.uri.path(), |original| original.0.path());
    let resource = format!("{origin}{}", resources.resource_path(path));
    if access.covers(&resource) { Ok(()) } else { Err(wrong_audience()) }
}

fn wrong_audience() -> Error {
    Error::unauthenticated(
        "oauth_token_audience",
        "This access token was issued for another resource. Ask the authorization server for a token for \
         this address (the `resource` parameter), then retry.",
    )
    .with_challenge(
        "Bearer error=\"invalid_token\", error_description=\"The token was issued for another resource\"",
    )
}

/// Layer on every request: makes the resources known to authentication, and adds `resource_metadata` to the
/// `Bearer` challenge of 401 / 403 answers under `/mcp` and `/api/v1` (RFC 9728 §5.1).
pub(crate) async fn challenge(
    resources: Arc<ProtectedResources>,
    mut request: Request,
    next: Next,
) -> Response {
    request.extensions_mut().insert(Resources(resources.clone()));
    if !resources.enabled() {
        return next.run(request).await;
    }
    let path = request.uri().path().to_owned();
    let (parts, body) = request.into_parts();
    let host = host_of(&parts);
    let mut response = next.run(Request::from_parts(parts, body)).await;
    let protected = path == MCP_PATH
        || path.starts_with(&format!("{MCP_PATH}/"))
        || path.starts_with(&format!("{API_PREFIX}/"));
    if !protected || !matches!(response.status().as_u16(), 401 | 403) {
        return response;
    }
    let Some(current) = response
        .headers()
        .get(header::WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.starts_with("Bearer") && !value.contains("resource_metadata="))
        .map(str::to_owned)
    else {
        return response;
    };
    let Ok(Some(origin)) = resources.origin(host.as_deref()).await else { return response };
    let metadata = format!(
        "resource_metadata=\"{origin}{PROTECTED_RESOURCE_PATH}{}\"",
        resources.resource_path(&path)
    );
    let value = match current.strip_prefix("Bearer").map(str::trim) {
        Some("") | None => format!("Bearer {metadata}"),
        Some(rest) => format!("Bearer {rest}, {metadata}"),
    };
    if let Ok(value) = HeaderValue::from_str(&value) {
        response.headers_mut().insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

pub(super) fn routes(resources: Arc<ProtectedResources>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(server_resource))
        .routes(routes!(mcp_resource))
        .routes(routes!(mcp_module_resource))
        .routes(routes!(rest_resource))
        .routes(routes!(authorization_server))
        .with_state(resources)
}

/// Protected resource metadata of the whole server.
///
/// RFC 9728 document for the resource `{origin}` (every endpoint): which authorization server issues tokens for
/// it. MCP clients find this through the `resource_metadata` parameter of a 401 answer.
#[utoipa::path(
    get,
    path = "/.well-known/oauth-protected-resource",
    extensions(("x-public" = json!(true))),
    tag = "health",
    operation_id = "oauth_protected_resource",
    responses(
        (status = 200, description = "The RFC 9728 metadata document.", body = Object),
        (status = 404, description = "OAuth sign-in is not set up on this server (code `oauth_unavailable`): \
         use an API key.", body = crate::response::Problem),
    )
)]
async fn server_resource(
    State(resources): State<Arc<ProtectedResources>>,
    parts: Parts,
) -> Result<Response, Error> {
    resource_document(&resources, &parts, "").await
}

/// Protected resource metadata of the MCP endpoint.
///
/// RFC 9728 document for the resource `{origin}/mcp`; a token for it is accepted at `/mcp` and every
/// `/mcp/{module}`.
#[utoipa::path(
    get,
    path = "/.well-known/oauth-protected-resource/mcp",
    extensions(("x-public" = json!(true))),
    tag = "health",
    operation_id = "oauth_protected_resource_mcp",
    responses(
        (status = 200, description = "The RFC 9728 metadata document.", body = Object),
        (status = 404, description = "OAuth sign-in is not set up on this server (code `oauth_unavailable`).",
         body = crate::response::Problem),
    )
)]
async fn mcp_resource(
    State(resources): State<Arc<ProtectedResources>>,
    parts: Parts,
) -> Result<Response, Error> {
    resource_document(&resources, &parts, MCP_PATH).await
}

/// Protected resource metadata of one module's MCP endpoint.
///
/// RFC 9728 document for the resource `{origin}/mcp/{module}`.
#[utoipa::path(
    get,
    path = "/.well-known/oauth-protected-resource/mcp/{module}",
    params(("module" = String, Path, description = "Name of a module that has an MCP endpoint.")),
    extensions(("x-public" = json!(true))),
    tag = "health",
    operation_id = "oauth_protected_resource_mcp_module",
    responses(
        (status = 200, description = "The RFC 9728 metadata document.", body = Object),
        (status = 404, description = "No such MCP endpoint, or OAuth sign-in is not set up (code \
         `oauth_unavailable`).", body = crate::response::Problem),
    )
)]
async fn mcp_module_resource(
    State(resources): State<Arc<ProtectedResources>>,
    Path(module): Path<String>,
    parts: Parts,
) -> Result<Response, Error> {
    if !resources.mcp_modules.contains(&module) {
        return Err(unavailable());
    }
    resource_document(&resources, &parts, &format!("{MCP_PATH}/{module}")).await
}

/// Protected resource metadata of the REST API.
///
/// RFC 9728 document for the resource `{origin}/api/v1`.
#[utoipa::path(
    get,
    path = "/.well-known/oauth-protected-resource/api/v1",
    extensions(("x-public" = json!(true))),
    tag = "health",
    operation_id = "oauth_protected_resource_rest",
    responses(
        (status = 200, description = "The RFC 9728 metadata document.", body = Object),
        (status = 404, description = "OAuth sign-in is not set up on this server (code `oauth_unavailable`).",
         body = crate::response::Problem),
    )
)]
async fn rest_resource(
    State(resources): State<Arc<ProtectedResources>>,
    parts: Parts,
) -> Result<Response, Error> {
    resource_document(&resources, &parts, API_PREFIX).await
}

async fn resource_document(
    resources: &ProtectedResources,
    parts: &Parts,
    path: &str,
) -> Result<Response, Error> {
    if !resources.enabled() {
        return Err(unavailable());
    }
    let origin = resources.origin(host_of(parts).as_deref()).await?.ok_or_else(unavailable)?;
    let name = match path {
        "" => "Autoxide".to_owned(),
        MCP_PATH => "Autoxide MCP".to_owned(),
        API_PREFIX => "Autoxide REST API".to_owned(),
        other => format!("Autoxide MCP ({})", other.trim_start_matches("/mcp/")),
    };
    // `scopes_supported` cố ý không khai (D-20261126-mcp-oauth mục 7): người duyệt chọn quyền ở trang consent.
    let document = json!({
        "resource": format!("{origin}{path}"),
        "authorization_servers": [origin],
        "bearer_methods_supported": ["header"],
        "resource_name": name,
        "resource_documentation": format!("{origin}{}", super::LLMS_PATH),
    });
    Ok(public_json(document))
}

/// Authorization server metadata.
///
/// RFC 8414 document of the authorization server built into this server: endpoints, PKCE methods, client
/// registration (Client ID Metadata Documents, dynamic registration) and supported scopes.
#[utoipa::path(
    get,
    path = "/.well-known/oauth-authorization-server",
    extensions(("x-public" = json!(true))),
    tag = "health",
    operation_id = "oauth_authorization_server",
    responses(
        (status = 200, description = "The RFC 8414 metadata document.", body = Object),
        (status = 404, description = "OAuth sign-in is not set up on this server (code `oauth_unavailable`).",
         body = crate::response::Problem),
    )
)]
async fn authorization_server(
    State(resources): State<Arc<ProtectedResources>>,
    parts: Parts,
) -> Result<Response, Error> {
    let (Some(server), true) = (&resources.server, resources.enabled()) else {
        return Err(unavailable());
    };
    let issuer = resources.origin(host_of(&parts).as_deref()).await?.ok_or_else(unavailable)?;
    let mut document = server.metadata(&issuer);
    // Kernel tự đặt `issuer`: client PHẢI so nó với địa chỉ đã tải (RFC 8414 §3.3).
    document.insert("issuer".to_owned(), Value::String(issuer));
    Ok(public_json(Value::Object(document)))
}

/// A public JSON document: cacheable for five minutes, readable from any web page (no credentials involved).
fn public_json(document: Value) -> Response {
    let mut response = (StatusCode::OK, axum::Json(document)).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=300"));
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    response
}

fn unavailable() -> Error {
    Error::not_found(
        "oauth_unavailable",
        "OAuth sign-in is not set up on this server. Connect with an API key instead, or ask the operator to \
         set AUTOXIDE_HTTP_PUBLIC_URL and the authorization server settings.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resources() -> ProtectedResources {
        ProtectedResources::new(
            Some("https://api.example.com".to_owned()),
            None,
            None,
            vec!["notes".to_owned()],
        )
    }

    #[test]
    fn duong_dan_ra_resource() {
        let resources = resources();
        assert_eq!(resources.resource_path("/mcp"), "/mcp");
        assert_eq!(resources.resource_path("/mcp/notes"), "/mcp/notes");
        assert_eq!(resources.resource_path("/mcp/notes/"), "/mcp/notes");
        // Module lạ: thuộc `/mcp` chung (token cho `/mcp/<lạ>` không bao giờ được cấp).
        assert_eq!(resources.resource_path("/mcp/unknown"), "/mcp");
        assert_eq!(resources.resource_path("/api/v1/notes/1"), "/api/v1");
        assert_eq!(resources.resource_path("/api/v1x"), "");
        assert_eq!(resources.resource_path("/health/live"), "");
    }

    #[test]
    fn cung_host_bo_qua_port_va_hoa_thuong() {
        assert!(same_host("https://API.example.com:8443", "api.example.com"));
        assert!(same_host("http://[::1]:3000", "::1"));
        assert!(!same_host("https://api.example.com", "shop.example.com"));
    }
}
