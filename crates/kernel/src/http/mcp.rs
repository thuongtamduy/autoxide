//! Endpoint MCP (Streamable HTTP) ở `/mcp`, dựng bằng `rmcp` (D-068).
//!
//! - Không giữ phiên (stateless) và trả JSON: mỗi request độc lập, chạy được trên nhiều
//!   máy chủ như REST.
//! - Xác thực như REST (`Authorization: Bearer <access token hoặc API key>`), ở một lớp
//!   chạy trước `rmcp`: thiếu hay sai token thì 401 kiểu Problem. Người gọi
//!   ([`Principal`]) được đặt vào request; handler đọc lại từ đó.
//! - `tools/list` chỉ liệt kê tool người gọi có permission; `tools/call` tool khác coi
//!   như không tồn tại.
//! - Lỗi của tool trả về như kết quả lỗi của tool (`isError`), kèm `code` ổn định và
//!   `message` dịch theo `Accept-Language` — agent đọc được và tự sửa.

use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, ServerHandler};
use serde_json::{Value, json};

use crate::access::{Authenticator, Principal};
use crate::config::CorsOrigins;
use crate::error::{Error, ErrorKind};
use crate::i18n::{Language, Translations};
use crate::mcp::Tool;

/// Đường dẫn của endpoint MCP.
pub const MCP_PATH: &str = "/mcp";

/// Hướng dẫn gửi cho client lúc bắt tay.
const INSTRUCTIONS: &str = "Tools of this Autoxide server. Each tool calls the same service as \
the REST API (`/api/v1/openapi.json`) with the caller's permissions; only the tools the caller \
may use are listed. Results have the same shape as REST: the value under `data`, and lists also \
carry `pagination` (follow `next_cursor` with the `cursor` argument). Failed calls return \
`isError` with a stable `code` and a `message` that says what to fix.";

#[derive(Clone)]
struct McpServer {
    tools: Arc<Vec<Tool>>,
    translations: Arc<Translations>,
}

/// Router của endpoint MCP; `None` khi chưa module nào xác thực (không biết ai gọi thì
/// không có tool nào gọi được).
pub(super) fn router(
    tools: Vec<Tool>,
    translations: Arc<Translations>,
    authenticator: Option<Arc<dyn Authenticator>>,
    origins: &CorsOrigins,
) -> Option<Router> {
    let authenticator = authenticator?;
    let server = McpServer { tools: Arc::new(tools), translations };
    let mut config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        // Mỗi request mang token riêng, nên kiểm `Host` (chống DNS rebinding cho máy chủ
        // không xác thực) không cần; tên miền do người cài chọn.
        .disable_allowed_hosts();
    config = match origins {
        // Như CORS: không khai origin nào thì trình duyệt không gọi được.
        CorsOrigins::None => config.enforce_origin_validation(),
        CorsOrigins::Any => config.disable_allowed_origins(),
        CorsOrigins::List(list) => config.with_allowed_origins(list.clone()),
    };
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let router = Router::new().route_service(MCP_PATH, service).route_layer(middleware::from_fn(
        move |request: Request, next: Next| authenticate(authenticator.clone(), request, next),
    ));
    Some(router)
}

/// Lớp xác thực: đặt [`Principal`] vào request, hoặc trả 401.
async fn authenticate(
    authenticator: Arc<dyn Authenticator>,
    mut request: Request,
    next: Next,
) -> Response {
    let authorization =
        request.headers().get(header::AUTHORIZATION).and_then(|value| value.to_str().ok());
    match authenticator.authenticate(authorization).await {
        Ok(principal) => {
            request.extensions_mut().insert(principal);
            next.run(request).await
        }
        Err(error) => {
            let mut response = error.into_response();
            if response.status().as_u16() == 401 {
                response
                    .headers_mut()
                    .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            }
            response
        }
    }
}

/// Người gọi và ngôn ngữ của request đang xử lý.
fn caller(context: &RequestContext<RoleServer>) -> Result<(Principal, Language), ErrorData> {
    let parts = context.extensions.get::<axum::http::request::Parts>();
    let principal = parts.and_then(|parts| parts.extensions.get::<Principal>()).cloned();
    let principal = principal.ok_or_else(|| {
        ErrorData::internal_error("the MCP request reached the server without a caller", None)
    })?;
    let accept = parts
        .and_then(|parts| parts.headers.get(header::ACCEPT_LANGUAGE))
        .and_then(|value| value.to_str().ok());
    Ok((principal, Language::negotiate(accept)))
}

fn describe(tool: &Tool) -> rmcp::model::Tool {
    let mut description = tool.description().to_owned();
    if let Some(permission) = tool.permission() {
        description.push_str(&format!(" Requires permission `{}`.", permission.name()));
    }
    let schema = match tool.input_schema() {
        Value::Object(object) => object.clone(),
        _ => serde_json::Map::new(),
    };
    let annotations =
        ToolAnnotations::new().read_only(tool.is_read_only()).destructive(tool.is_destructive());
    rmcp::model::Tool::new(tool.name().to_owned(), description, Arc::new(schema))
        .annotate(annotations)
}

impl McpServer {
    /// Lỗi của tool thành kết quả lỗi mà agent đọc được.
    fn failure(&self, tool: &str, error: &Error, language: Language) -> CallToolResult {
        if error.kind() == ErrorKind::Internal {
            tracing::error!(tool, error = ?error, "MCP tool failed");
        }
        let mut body = json!({
            "code": error.code(),
            "message": self.translations.message(error, language),
        });
        if !error.fields().is_empty() {
            // Cùng hình dạng với `errors` của Problem trong REST.
            let fields: Vec<Value> = error
                .fields()
                .iter()
                .map(|field| {
                    json!({"field": field.field, "code": field.code, "detail": field.message})
                })
                .collect();
            body["errors"] = Value::Array(fields);
        }
        CallToolResult::structured_error(body)
    }
}

/// Kết quả tool theo đúng envelope của REST: một trang (`Page`: `data` + `pagination`)
/// giữ nguyên; giá trị khác bọc thành `{"data": …}` (như `Data`).
fn envelope(value: Value) -> Value {
    let is_page = value.as_object().is_some_and(|object| {
        object.len() == 2 && object.contains_key("data") && object.contains_key("pagination")
    });
    if is_page { value } else { json!({ "data": value }) }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("autoxide", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let (principal, _) = caller(&context)?;
        let tools = self.tools.iter().filter(|tool| tool.allows(&principal)).map(describe);
        Ok(ListToolsResult::with_all_items(tools.collect()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let (principal, language) = caller(&context)?;
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name() == request.name && tool.allows(&principal))
            .ok_or_else(|| {
                // Tool không có và tool không được dùng trả cùng một lỗi.
                ErrorData::invalid_params(
                    format!(
                        "Unknown tool `{}`. Call `tools/list` for the tools you may use.",
                        request.name
                    ),
                    None,
                )
            })?;
        let arguments = request.arguments.map_or(Value::Null, Value::Object);
        metrics::counter!("mcp_tool_calls_total").increment(1);
        let result = match tool.call(principal, arguments).await {
            Ok(value) => CallToolResult::structured(envelope(value)),
            Err(error) => self.failure(tool.name(), &error, language),
        };
        Ok(result.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ket_qua_cung_envelope_voi_rest() {
        let page = json!({"data": [1, 2], "pagination": {"limit": 2, "next_cursor": null}});
        assert_eq!(envelope(page.clone()), page);
        assert_eq!(envelope(json!({"id": 1})), json!({"data": {"id": 1}}));
        assert_eq!(envelope(json!([1])), json!({"data": [1]}));
        // Trùng tên trường mà không phải trang: vẫn bọc.
        let lookalike = json!({"data": 1, "pagination": 2, "extra": 3});
        assert_eq!(envelope(lookalike.clone()), json!({"data": lookalike}));
    }
}
