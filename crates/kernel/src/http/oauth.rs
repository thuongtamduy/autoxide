//! OAuth 2.1 protected resource (step 71 G4, D-20261126-mcp-oauth): the discovery documents of RFC 9728 and
//! RFC 8414, the `resource_metadata` parameter of `WWW-Authenticate` (MCP authorization 2026-07-28), and the
//! audience check of OAuth access tokens (RFC 8707).
//!
//! The authorization server itself lives in a module (usually `auth`), which publishes
//! [`AuthorizationServer`](crate::access::AuthorizationServer). Without it, or without
//! `AUTOXIDE_HTTP_PUBLIC_URL`, the documents answer 404 and challenges stay a bare `Bearer`.
//!
//! Resources (canonical, no trailing slash): `{issuer}/mcp` (every MCP endpoint), `{issuer}/mcp/<module>` and
//! `{issuer}/api/v1` (REST). The issuer is the public URL (one for the whole platform; a public URL with a path
//! turns OAuth off). Any other path never accepts an OAuth token.

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

/// Where the protected resource metadata documents live (RFC 9728 §3).
pub const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource";
/// Where the authorization server metadata lives (RFC 8414 §3).
pub const AUTHORIZATION_SERVER_PATH: &str = "/.well-known/oauth-authorization-server";

/// What the resource server needs to know, shared by the documents, the challenge layer and the audience check.
pub(crate) struct ProtectedResources {
    /// The issuer and resource prefix: the public URL, lowercase, without path. `None` turns OAuth off.
    origin: Option<String>,
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
        server: Option<Arc<dyn AuthorizationServer>>,
        mcp_modules: Vec<String>,
    ) -> Self {
        // Một issuer cho cả nền tảng (D-20261126-mcp-oauth): public URL. Public URL có path (API sau một tiền tố của
        // proxy) thì tài liệu khám phá phải chèn path theo RFC 8414 / 9728 mà proxy chưa chắc chuyển tới đây: tắt
        // OAuth và báo, thay vì phát địa chỉ sai.
        let origin = public_url.and_then(|url| {
            let origin = crate::access::canonical_resource(&url)?;
            let path = origin.split_once("://").map_or("", |(_, rest)| rest.split_once('/').map_or("", |(_, path)| path));
            if path.is_empty() {
                Some(origin)
            } else {
                if server.is_some() {
                    tracing::warn!(
                        "AUTOXIDE_HTTP_PUBLIC_URL has a path: OAuth sign-in for MCP is off. Serve the API at the \
                         root of its own host name to turn it on."
                    );
                }
                None
            }
        });
        ProtectedResources { origin, server, mcp_modules }
    }

    /// OAuth is on: a module issues tokens and the public address is known: the issuer.
    fn issuer(&self) -> Option<&str> {
        self.server.as_ref().and(self.origin.as_deref())
    }

    /// The resource path (after the origin) a request path belongs to: `/mcp`, `/mcp/<module>`, `/api/v1`, or
    /// `""` (no resource: OAuth tokens are never accepted there).
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
            return "";
        }
        if path == API_PREFIX || path.starts_with(&format!("{API_PREFIX}/")) {
            return API_PREFIX;
        }
        ""
    }

    /// `challenge` (a `Bearer …` value) with `resource_metadata` pointing at the document of the resource that
    /// `path` belongs to. Unchanged when OAuth is off, the path is no resource, or it already has one.
    pub(crate) fn complete(&self, path: &str, challenge: &str) -> String {
        let resource = self.resource_path(path);
        let Some(origin) = self.issuer() else { return challenge.to_owned() };
        if resource.is_empty() || !challenge.starts_with("Bearer") {
            return challenge.to_owned();
        }
        let metadata = format!("resource_metadata=\"{origin}{PROTECTED_RESOURCE_PATH}{resource}\"");
        if challenge.contains(&metadata) {
            return challenge.to_owned();
        }
        // Một `resource_metadata` khác (không phải của resource này) không được giữ: bỏ nó rồi thêm của mình. Tách
        // tham số theo dấu phẩy NGOÀI chuỗi trong ngoặc kép (`error_description` có thể chứa dấu phẩy).
        let params = challenge.strip_prefix("Bearer").unwrap_or_default();
        let kept: Vec<&str> = split_params(params)
            .into_iter()
            .filter(|part| !part.is_empty() && !part.starts_with("resource_metadata="))
            .collect();
        if kept.is_empty() {
            format!("Bearer {metadata}")
        } else {
            format!("Bearer {}, {metadata}", kept.join(", "))
        }
    }
}

// Tham số của một challenge tách theo dấu phẩy nằm ngoài ngoặc kép (RFC 9110 §11.2), đã bỏ khoảng
// trắng. Dấu ngoặc kép và gạch chéo ngược viết bằng mã Unicode để công cụ quét mã nguồn không nhầm là mở chuỗi.
fn split_params(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut start, mut quoted, mut escaped) = (0, false, false);
    for (index, character) in text.char_indices() {
        match character {
            _ if escaped => escaped = false,
            '\u{5c}' if quoted => escaped = true,
            '\u{22}' => quoted = !quoted,
            ',' if !quoted => {
                parts.push(text[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(text[start..].trim());
    parts
}

/// Rejects an OAuth access token used outside the resource it was issued for (RFC 8707, MCP: servers MUST
/// validate the audience). Fail-closed: a request path that is no resource (`/mcp`, `/mcp/<module>`, `/api/v1`)
/// never accepts an OAuth token. Credentials without an audience (sessions, API keys) pass.
pub(crate) fn admit_audience(parts: &Parts, principal: &Principal) -> Result<(), Error> {
    if principal.oauth().is_none() {
        return Ok(());
    }
    let Some(resources) = parts.extensions.get::<Resources>() else { return Err(wrong_audience()) };
    // Router lồng (`nest`) cắt tiền tố khỏi `uri`; `OriginalUri` giữ đường dẫn đầy đủ.
    let path = parts
        .extensions
        .get::<axum::extract::OriginalUri>()
        .map_or_else(|| parts.uri.path(), |original| original.0.path());
    if resources.0.admits(path, principal) { Ok(()) } else { Err(wrong_audience()) }
}

impl ProtectedResources {
    /// Whether `principal` may call `path` (full request path): always for credentials without an audience; for
    /// an OAuth token only when its audience covers the resource `path` belongs to.
    pub(crate) fn admits(&self, path: &str, principal: &Principal) -> bool {
        let Some(access) = principal.oauth() else { return true };
        let Some(origin) = self.issuer() else { return false };
        let resource = self.resource_path(path);
        !resource.is_empty() && access.covers(&format!("{origin}{resource}"))
    }
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
    if resources.issuer().is_none() {
        return next.run(request).await;
    }
    let path = request.uri().path().to_owned();
    let mut response = next.run(request).await;
    if !matches!(response.status().as_u16(), 401 | 403) {
        return response;
    }
    let Some(current) =
        response.headers().get(header::WWW_AUTHENTICATE).and_then(|value| value.to_str().ok())
    else {
        return response;
    };
    let completed = resources.complete(&path, current);
    if let Ok(value) = HeaderValue::from_str(&completed) {
        response.headers_mut().insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

pub(super) fn routes(resources: Arc<ProtectedResources>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(mcp_resource))
        .routes(routes!(mcp_module_resource))
        .routes(routes!(rest_resource))
        .routes(routes!(authorization_server))
        .with_state(resources)
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
async fn mcp_resource(State(resources): State<Arc<ProtectedResources>>) -> Result<Response, Error> {
    resource_document(&resources, MCP_PATH)
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
) -> Result<Response, Error> {
    if !resources.mcp_modules.contains(&module) {
        return Err(unavailable());
    }
    resource_document(&resources, &format!("{MCP_PATH}/{module}"))
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
) -> Result<Response, Error> {
    resource_document(&resources, API_PREFIX)
}

fn resource_document(resources: &ProtectedResources, path: &str) -> Result<Response, Error> {
    let origin = resources.issuer().ok_or_else(unavailable)?;
    let name = match path {
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
) -> Result<Response, Error> {
    let (Some(server), Some(issuer)) = (&resources.server, resources.issuer()) else {
        return Err(unavailable());
    };
    let mut document = server.metadata(issuer);
    // Kernel tự đặt `issuer`: client PHẢI so nó với địa chỉ đã tải (RFC 8414 §3.3).
    document.insert("issuer".to_owned(), Value::String(issuer.to_owned()));
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

    fn resources(public: &str) -> ProtectedResources {
        ProtectedResources::new(Some(public.to_owned()), None, vec!["notes".to_owned()])
    }

    #[test]
    fn duong_dan_ra_resource() {
        let resources = resources("https://api.example.com");
        assert_eq!(resources.resource_path("/mcp"), "/mcp");
        assert_eq!(resources.resource_path("/mcp/notes"), "/mcp/notes");
        assert_eq!(resources.resource_path("/mcp/notes/"), "/mcp/notes");
        // Module lạ, đường dẫn ngoài `/mcp` và `/api/v1`: không phải resource, token OAuth không bao giờ được nhận.
        assert_eq!(resources.resource_path("/mcp/unknown"), "");
        assert_eq!(resources.resource_path("/api/v1/notes/1"), "/api/v1");
        assert_eq!(resources.resource_path("/api/v1x"), "");
        assert_eq!(resources.resource_path("/health/live"), "");
    }

    #[test]
    fn challenge_giu_mo_ta_co_dau_phay_va_thay_metadata_la() {
        let resources = ProtectedResources::new(
            Some("https://api.example.com".to_owned()),
            Some(std::sync::Arc::new(Fake)),
            vec![],
        );
        let ours = "resource_metadata=\"https://api.example.com/.well-known/oauth-protected-resource/mcp\"";
        let completed = resources.complete(
            "/mcp",
            "Bearer error=\"invalid_token\", error_description=\"a, b\", resource_metadata=\"https://evil/x\"",
        );
        assert_eq!(
            completed,
            format!("Bearer error=\"invalid_token\", error_description=\"a, b\", {ours}")
        );
        assert_eq!(resources.complete("/mcp", "Bearer"), format!("Bearer {ours}"));
    }

    struct Fake;

    impl AuthorizationServer for Fake {
        fn metadata(&self, _: &str) -> serde_json::Map<String, Value> {
            serde_json::Map::new()
        }
    }

    #[test]
    fn public_url_co_path_thi_tat_oauth() {
        assert_eq!(
            resources("https://API.example.com/").origin.as_deref(),
            Some("https://api.example.com")
        );
        assert_eq!(resources("https://example.com/api").origin, None);
    }
}
