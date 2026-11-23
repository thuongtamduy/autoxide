//! HTTP layer of the MCP endpoint (Streamable HTTP, spec 2026-07-28 and older versions that still have
//! `initialize`): checks `Origin`, method, `Accept`, `Content-Type`, reads one JSON-RPC message, checks
//! headers per protocol generation, calls [`McpServer::dispatch`], then sets the HTTP status. Always returns
//! JSON (no SSE): no tool sends intermediate messages, and the server keeps no session (like the earlier
//! `rmcp` configuration).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};

use super::McpServer;
use super::protocol::{self, Incoming, RpcError, code};
use crate::access::Principal;
use crate::config::CorsOrigins;
use crate::i18n::Language;
use crate::tenancy::TenantContext;

/// Largest POST body (4 MiB, like the `rmcp` default): MCP commands are small JSON; files go through REST.
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

const PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";
const METHOD_HEADER: &str = "mcp-method";
const NAME_HEADER: &str = "mcp-name";

/// Who calls and with what: what [`McpServer::dispatch`] needs from the HTTP request.
pub(super) struct Caller {
    pub(super) tenant: TenantContext,
    pub(super) principal: Principal,
    pub(super) language: Language,
    pub(super) request_id: Option<String>,
    /// The request belongs to the new generation (`2026-07-28`): there is no `ping`, and results carry
    /// `resultType`.
    pub(super) modern: bool,
    /// The request called a tool that returns a secret: the response carries `Cache-Control: no-store` (D-149).
    // `AtomicBool` thay vì `Cell<bool>`: tham chiếu tới nó sống qua `await`, nên phải dùng chung được giữa
    // các thread (`Sync`) — future của handler axum phải `Send`.
    pub(super) secret: AtomicBool,
}

/// Serves one HTTP request to `/mcp` or `/mcp/<module>` (already past the authentication layer).
pub(super) async fn handle(
    server: McpServer,
    origins: Arc<CorsOrigins>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    // Chống DNS rebinding (đặc tả: "Servers MUST validate the Origin header"): như CORS — không khai origin
    // nào thì trình duyệt không gọi được.
    if let Some(origin) = parts.headers.get(header::ORIGIN)
        && !origin_allowed(&origins, origin)
    {
        let error = RpcError::new(code::INVALID_REQUEST, "Forbidden: Origin is not allowed");
        return json_response(StatusCode::FORBIDDEN, &error.into_message(None));
    }
    // Không giữ phiên, không luồng GET: `GET` / `DELETE` (của bản cũ) trả 405.
    if parts.method != Method::POST {
        let mut response = plain(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed: use POST");
        response.headers_mut().insert(header::ALLOW, HeaderValue::from_static("POST"));
        return response;
    }
    if !accepts_json_and_sse(&parts.headers) {
        return plain(
            StatusCode::NOT_ACCEPTABLE,
            "Not Acceptable: Client must accept both application/json and text/event-stream",
        );
    }
    if !is_json(&parts.headers) {
        return plain(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Unsupported Media Type: Content-Type must be application/json",
        );
    }
    let Ok(bytes) = axum::body::to_bytes(body, MAX_BODY_BYTES).await else {
        return plain(StatusCode::PAYLOAD_TOO_LARGE, "Payload Too Large");
    };
    let (id, method, params) = match protocol::parse(&bytes) {
        Ok(Incoming::Request { id, method, params }) => (id, method, params),
        // Thông báo (`notifications/initialized`, `notifications/cancelled`): nhận, không trả lời.
        Ok(Incoming::Notification) => return StatusCode::ACCEPTED.into_response(),
        Ok(Incoming::Response) => {
            let error = RpcError::new(
                code::INVALID_REQUEST,
                "Invalid Request: this server keeps no session and never asks the client anything",
            );
            return json_response(StatusCode::BAD_REQUEST, &error.into_message(None));
        }
        Err(error) => return json_response(StatusCode::BAD_REQUEST, &error.into_message(None)),
    };

    let modern = match era(&parts.headers, &method, &params) {
        Ok(modern) => modern,
        Err(Rejection::Text(message)) => return plain(StatusCode::BAD_REQUEST, message),
        Err(Rejection::Rpc(error)) => {
            return json_response(StatusCode::BAD_REQUEST, &error.into_message(Some(id)));
        }
    };

    let principal = parts.extensions.get::<Principal>().cloned();
    let tenant = parts.extensions.get::<TenantContext>().cloned();
    let (Some(principal), Some(tenant)) = (principal, tenant) else {
        tracing::error!("the MCP request reached the server without a caller");
        return json_response(StatusCode::OK, &RpcError::internal().into_message(Some(id)));
    };
    let accept_language =
        parts.headers.get(header::ACCEPT_LANGUAGE).and_then(|value| value.to_str().ok());
    let caller = Caller {
        tenant,
        principal,
        language: Language::negotiate(accept_language),
        request_id: crate::ops::request_log::header_text(
            parts.headers.get(&super::super::request_id::REQUEST_ID_HEADER),
            128,
        ),
        modern,
        secret: AtomicBool::new(false),
    };

    let outcome = server.dispatch(&method, params, &caller).await;
    let (status, message) = match outcome {
        Ok(mut result) => {
            if modern {
                decorate(&method, &mut result);
            }
            (StatusCode::OK, protocol::result_message(id, Value::Object(result)))
        }
        Err(error) => (error_status(&error, modern), error.into_message(Some(id))),
    };
    let mut response = json_response(status, &message);
    if caller.secret.load(std::sync::atomic::Ordering::Relaxed) {
        // Kết quả có secret chỉ trả một lần: không cho ai lưu lại response (D-149).
        response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

/// Reason for rejecting before running: bare text (like `rmcp` with a wrong version header) or a JSON-RPC
/// error.
enum Rejection {
    Text(String),
    Rpc(RpcError),
}

/// The request's generation (`true`: new) after checking the headers of that generation.
///
/// New: `params._meta` carries `io.modelcontextprotocol/protocolVersion`, or the method is `server/discover`.
/// Otherwise: the old version, the version taken from the `MCP-Protocol-Version` header (`2025-03-26` if
/// missing).
fn era(headers: &HeaderMap, method: &str, params: &Map<String, Value>) -> Result<bool, Rejection> {
    let meta = protocol::meta(params);
    let meta_version = meta.and_then(|meta| meta.get(protocol::META_PROTOCOL_VERSION));
    let header_version = match headers.get(PROTOCOL_VERSION_HEADER) {
        None => None,
        Some(value) => Some(value.to_str().map_err(|_| {
            Rejection::Text("Bad Request: Invalid MCP-Protocol-Version header encoding".to_owned())
        })?),
    };

    if meta_version.is_none() && method != "server/discover" {
        // Bản cũ.
        if let Some(version) = header_version {
            if !protocol::is_supported(version) {
                return Err(Rejection::Text(format!(
                    "Bad Request: Unsupported MCP-Protocol-Version: {version}"
                )));
            }
            if protocol::is_modern(version) {
                return Err(Rejection::Rpc(RpcError::invalid_params(
                    "Invalid params: request requires protocolVersion in request _meta",
                )));
            }
            if method == "initialize" {
                let body =
                    params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
                if body != version {
                    return Err(Rejection::Rpc(RpcError::new(
                        code::INVALID_REQUEST,
                        format!(
                            "Invalid Request: MCP-Protocol-Version header ({version}) does not match \
                             initialize params.protocolVersion ({body})"
                        ),
                    )));
                }
            }
        }
        return Ok(false);
    }

    // Bản mới: `_meta` phải đủ trường bắt buộc (thiếu là request hỏng: -32602, HTTP 400).
    let mut missing = Vec::new();
    let version = meta_version.and_then(Value::as_str);
    if version.is_none() {
        missing.push(protocol::META_PROTOCOL_VERSION);
    }
    if !meta
        .and_then(|meta| meta.get(protocol::META_CLIENT_CAPABILITIES))
        .is_some_and(Value::is_object)
    {
        missing.push(protocol::META_CLIENT_CAPABILITIES);
    }
    let Some(version) = version.filter(|_| missing.is_empty()) else {
        return Err(Rejection::Rpc(RpcError::invalid_params(format!(
            "request _meta is missing or has malformed required fields: {}",
            missing.join(", ")
        ))));
    };
    if !protocol::is_supported(version) {
        return Err(Rejection::Rpc(RpcError::unsupported_version(version)));
    }
    if header_version != Some(version) {
        return Err(Rejection::Rpc(RpcError::header_mismatch(format!(
            "MCP-Protocol-Version header ({}) does not match the request _meta protocolVersion ({version})",
            header_version.unwrap_or("missing")
        ))));
    }
    if !protocol::is_modern(version) {
        // `_meta` mang bản cũ: phục vụ như bản cũ (không có header chuẩn để kiểm).
        return Ok(false);
    }
    // Header chuẩn (SEP-2243): bộ cân bằng tải có thể định tuyến theo header, nên header phải khớp thân.
    let header_method = headers.get(METHOD_HEADER).and_then(|value| value.to_str().ok());
    if header_method != Some(method) {
        return Err(Rejection::Rpc(RpcError::header_mismatch(format!(
            "Mcp-Method header ({}) does not match the request method ({method})",
            header_method.unwrap_or("missing")
        ))));
    }
    let name_field = match method {
        "tools/call" | "prompts/get" => Some("name"),
        "resources/read" => Some("uri"),
        _ => None,
    };
    if let Some(field) = name_field {
        let body = params.get(field).and_then(Value::as_str);
        let header = headers
            .get(NAME_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(protocol::decode_header_value);
        match header {
            Some(Some(header)) if Some(header.as_str()) == body => {}
            Some(None) => {
                return Err(Rejection::Rpc(RpcError::header_mismatch(
                    "Mcp-Name header has an invalid Base64 value",
                )));
            }
            other => {
                let shown = other.flatten().unwrap_or_else(|| "missing".to_owned());
                return Err(Rejection::Rpc(RpcError::header_mismatch(format!(
                    "Mcp-Name header ({shown}) does not match params.{field} ({})",
                    body.unwrap_or("missing")
                ))));
            }
        }
    }
    Ok(true)
}

/// The result of the new version: `resultType`, server information in `_meta`, and cache hints for lists /
/// resource content (results depend on the caller's permissions: `private`, not kept long: `0`).
fn decorate(method: &str, result: &mut Map<String, Value>) {
    result.insert("resultType".to_owned(), Value::from("complete"));
    let meta = result.entry("_meta").or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(meta) = meta {
        meta.insert(protocol::META_SERVER_INFO.to_owned(), protocol::server_info());
    }
    let cacheable = matches!(
        method,
        "tools/list"
            | "resources/list"
            | "resources/read"
            | "resources/templates/list"
            | "prompts/list"
    );
    if cacheable {
        result.insert("ttlMs".to_owned(), Value::from(0));
        result.insert("cacheScope".to_owned(), Value::from("private"));
    }
}

/// The HTTP status of a JSON-RPC error: the old version is always 200; the new version 400 for version /
/// header errors, 404 for a nonexistent method (spec 2026-07-28, "Protocol Version Header").
fn error_status(error: &RpcError, modern: bool) -> StatusCode {
    if !modern {
        return StatusCode::OK;
    }
    match error.code {
        code::UNSUPPORTED_PROTOCOL_VERSION | code::HEADER_MISMATCH => StatusCode::BAD_REQUEST,
        code::METHOD_NOT_FOUND => StatusCode::NOT_FOUND,
        _ => StatusCode::OK,
    }
}

fn origin_allowed(origins: &CorsOrigins, origin: &HeaderValue) -> bool {
    match origins {
        CorsOrigins::None => false,
        CorsOrigins::Any => true,
        CorsOrigins::List(list) => {
            origin.to_str().is_ok_and(|origin| list.iter().any(|allowed| allowed == origin))
        }
    }
}

/// `Accept` contains both `application/json` and `text/event-stream` (or `*/*`, `application/*`, `text/*`).
fn accepts_json_and_sse(headers: &HeaderMap) -> bool {
    let mut json = false;
    let mut sse = false;
    for value in headers.get_all(header::ACCEPT) {
        let Ok(value) = value.to_str() else { continue };
        for item in value.split(',') {
            let essence = item.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
            match essence.as_str() {
                "*/*" => {
                    json = true;
                    sse = true;
                }
                "application/json" | "application/*" => json = true,
                "text/event-stream" | "text/*" => sse = true,
                _ => {}
            }
        }
    }
    json && sse
}

fn is_json(headers: &HeaderMap) -> bool {
    headers.get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok()).is_some_and(|value| {
        value.split(';').next().unwrap_or_default().trim().eq_ignore_ascii_case("application/json")
    })
}

fn json_response(status: StatusCode, message: &Value) -> Response {
    let body = serde_json::to_vec(message).unwrap_or_else(|_| b"{}".to_vec());
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    // Lỗi JSON-RPC giữ nguyên thân qua lớp Problem của kernel: đặc tả MCP đòi lỗi 400 / 404 của bản mới mang
    // lỗi JSON-RPC — client hai thế hệ đọc nó để biết máy chủ nói bản mới (không có thì lùi về `initialize`).
    response.extensions_mut().insert(crate::http::StandardErrorBody);
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

fn plain(status: StatusCode, message: impl Into<String>) -> Response {
    (status, message.into()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    fn modern_params(extra: Value) -> Map<String, Value> {
        let mut params = json!({
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        });
        if let (Value::Object(params), Value::Object(extra)) = (&mut params, extra) {
            params.extend(extra);
        }
        params.as_object().cloned().unwrap()
    }

    fn rejected_code(result: Result<bool, Rejection>) -> Option<i64> {
        match result {
            Err(Rejection::Rpc(error)) => Some(error.code),
            _ => None,
        }
    }

    #[test]
    fn ban_cu_khong_header_hay_header_cu() {
        assert!(!era(&HeaderMap::new(), "tools/list", &Map::new()).ok().unwrap());
        let old = headers(&[("mcp-protocol-version", "2025-06-18")]);
        assert!(!era(&old, "tools/list", &Map::new()).ok().unwrap());
        let unknown = headers(&[("mcp-protocol-version", "1900-01-01")]);
        assert!(matches!(era(&unknown, "tools/list", &Map::new()), Err(Rejection::Text(_))));
        let modern_without_meta = headers(&[("mcp-protocol-version", "2026-07-28")]);
        assert_eq!(
            rejected_code(era(&modern_without_meta, "tools/list", &Map::new())),
            Some(-32602)
        );
        let mismatch = headers(&[("mcp-protocol-version", "2025-06-18")]);
        let init = json!({"protocolVersion": "2025-11-25"}).as_object().cloned().unwrap();
        assert_eq!(rejected_code(era(&mismatch, "initialize", &init)), Some(-32600));
    }

    #[test]
    fn ban_moi_kiem_meta_va_header() {
        let good = headers(&[
            ("mcp-protocol-version", "2026-07-28"),
            ("mcp-method", "tools/call"),
            ("mcp-name", "notes.create"),
        ]);
        let params = modern_params(json!({"name": "notes.create"}));
        assert!(era(&good, "tools/call", &params).ok().unwrap());

        // Tên mã hoá base64 cũng khớp.
        let encoded = headers(&[
            ("mcp-protocol-version", "2026-07-28"),
            ("mcp-method", "tools/call"),
            ("mcp-name", "=?base64?bm90ZXMuY3JlYXRl?="),
        ]);
        assert!(era(&encoded, "tools/call", &params).ok().unwrap());

        let wrong_name = headers(&[
            ("mcp-protocol-version", "2026-07-28"),
            ("mcp-method", "tools/call"),
            ("mcp-name", "notes.delete"),
        ]);
        assert_eq!(rejected_code(era(&wrong_name, "tools/call", &params)), Some(-32020));
        let no_method =
            headers(&[("mcp-protocol-version", "2026-07-28"), ("mcp-name", "notes.create")]);
        assert_eq!(rejected_code(era(&no_method, "tools/call", &params)), Some(-32020));
        let wrong_version =
            headers(&[("mcp-protocol-version", "2025-11-25"), ("mcp-method", "tools/call")]);
        assert_eq!(rejected_code(era(&wrong_version, "tools/call", &params)), Some(-32020));

        let unsupported = modern_params(json!({}));
        let mut unsupported = unsupported;
        unsupported["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("2099-01-01");
        assert_eq!(rejected_code(era(&good, "tools/list", &unsupported)), Some(-32022));

        let no_capabilities =
            json!({"_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}})
                .as_object()
                .cloned()
                .unwrap();
        assert_eq!(rejected_code(era(&good, "tools/list", &no_capabilities)), Some(-32602));
        // `server/discover` luôn là bản mới, kể cả khi thiếu `_meta`.
        assert_eq!(
            rejected_code(era(&HeaderMap::new(), "server/discover", &Map::new())),
            Some(-32602)
        );
    }

    #[test]
    fn accept_va_content_type() {
        assert!(accepts_json_and_sse(&headers(&[(
            "accept",
            "application/json, text/event-stream"
        )])));
        assert!(accepts_json_and_sse(&headers(&[("accept", "*/*")])));
        assert!(!accepts_json_and_sse(&headers(&[("accept", "application/json")])));
        assert!(is_json(&headers(&[("content-type", "application/json; charset=utf-8")])));
        assert!(!is_json(&headers(&[("content-type", "text/plain")])));
    }

    #[test]
    fn ma_http_cua_loi() {
        assert_eq!(error_status(&RpcError::method_not_found("x"), false), StatusCode::OK);
        assert_eq!(error_status(&RpcError::method_not_found("x"), true), StatusCode::NOT_FOUND);
        assert_eq!(error_status(&RpcError::invalid_params("x"), true), StatusCode::OK);
    }
}
