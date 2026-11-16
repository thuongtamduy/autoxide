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
use std::sync::atomic::{AtomicBool, Ordering};

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
use crate::tenancy::TenantContext;

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
    /// Quyền dùng theo khách (bước 35): tool của module gói không có bị ẩn và bị chặn.
    entitlements: crate::entitlement::EntitlementsHandle,
}

/// Router của endpoint MCP; `None` khi chưa module nào xác thực (không biết ai gọi thì
/// không có tool nào gọi được).
pub(super) fn router(
    tools: Vec<Tool>,
    translations: Arc<Translations>,
    authenticator: Option<Arc<dyn Authenticator>>,
    origins: &CorsOrigins,
    entitlements: crate::entitlement::EntitlementsHandle,
) -> Option<Router> {
    let authenticator = authenticator?;
    let server = McpServer { tools: Arc::new(tools), translations, entitlements };
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
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();
    // Mọi lệnh MCP là `POST`: khách chỉ đọc không bị chặn ở đây mà ở `call_tool`, theo cờ
    // `read_only` của từng tool.
    parts.extensions.insert(crate::tenancy::AllowedWhileReadOnly);
    let authenticated =
        Box::pin(crate::tenancy::request::authenticate(&authenticator, &mut parts)).await;
    // Khách của request (đặt vào extensions của request): tool nhận nó cùng người gọi.
    let authenticated = match authenticated {
        Ok(principal) => Box::pin(crate::tenancy::request::tenant(&mut parts))
            .await
            .map(|tenant| (principal, tenant)),
        Err(error) => Err(error),
    };
    let mut request = Request::from_parts(parts, body);
    match authenticated {
        Ok((principal, tenant)) => {
            request.extensions_mut().insert(principal);
            request.extensions_mut().insert(tenant);
            let secret = SecretResult::default();
            request.extensions_mut().insert(secret.clone());
            let mut response = next.run(request).await;
            if secret.0.load(Ordering::Relaxed) {
                // Kết quả có secret chỉ trả một lần: không cho ai lưu lại response (D-149).
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            }
            response
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

/// Cờ chung giữa lớp xác thực và `call_tool`: request đã gọi một tool trả secret. `Arc` vì
/// rmcp chép phần đầu request (kèm extensions) sang ngữ cảnh của tool — bản chép vẫn trỏ
/// cùng một cờ.
#[derive(Clone, Default)]
struct SecretResult(Arc<AtomicBool>);

/// Khách, người gọi và ngôn ngữ của request đang xử lý.
fn caller(
    context: &RequestContext<RoleServer>,
) -> Result<(TenantContext, Principal, Language), ErrorData> {
    let parts = context.extensions.get::<axum::http::request::Parts>();
    let principal = parts.and_then(|parts| parts.extensions.get::<Principal>()).cloned();
    let tenant = parts.and_then(|parts| parts.extensions.get::<TenantContext>()).cloned();
    let (Some(principal), Some(tenant)) = (principal, tenant) else {
        return Err(ErrorData::internal_error(
            "the MCP request reached the server without a caller",
            None,
        ));
    };
    let accept = parts
        .and_then(|parts| parts.headers.get(header::ACCEPT_LANGUAGE))
        .and_then(|value| value.to_str().ok());
    Ok((tenant, principal, Language::negotiate(accept)))
}

fn describe(tool: &Tool) -> rmcp::model::Tool {
    let mut description = tool.description().to_owned();
    match (tool.permission(), tool.alternative()) {
        (Some(permission), Some(alternative)) => description.push_str(&format!(
            " Requires permission `{}` or `{}`.",
            permission.name(),
            alternative.name()
        )),
        (Some(permission), None) => {
            description.push_str(&format!(" Requires permission `{}`.", permission.name()));
        }
        _ => {}
    }
    if let Some(deprecation) = tool.deprecation() {
        description.push(' ');
        description.push_str(&deprecation.sentence(tool.name()));
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
    /// Module của tool (`<module>.<tên>`) có được dùng ở khách `tenant` không, và khách có chỉ
    /// đọc vì thuê bao không (bước 35). `(true, false)` khi không có gói nào áp dụng.
    async fn plan_allows(
        &self,
        tool: &Tool,
        tenant: crate::tenancy::TenantId,
    ) -> Result<(bool, bool), Error> {
        let module = tool.name().split('.').next().unwrap_or_default();
        if !self.entitlements.is_enforced() || self.entitlements.catalog().is_always_enabled(module)
        {
            return Ok((true, false));
        }
        let snapshot = self.entitlements.snapshot(tenant).await?;
        Ok((
            snapshot.has_module(module),
            snapshot.access == crate::entitlement::TenantAccess::ReadOnly,
        ))
    }

    /// Lỗi của tool thành kết quả lỗi mà agent đọc được.
    fn failure(&self, tool: &str, error: &Error, language: Language) -> CallToolResult {
        if error.kind() == ErrorKind::Internal {
            tracing::error!(tool, error = ?error, "MCP tool failed");
        }
        let mut body = json!({
            "code": error.code(),
            "message": self.translations.message(error, language),
        });
        if let Some(reason) = error.reason() {
            body["reason"] = Value::from(reason);
        }
        if !error.fields().is_empty() {
            // Cùng hình dạng với `errors` của Problem trong REST.
            let fields: Vec<Value> = error
                .fields()
                .iter()
                .map(|field| {
                    let detail = self.translations.field_message(field, language);
                    json!({"field": field.field, "code": field.code, "detail": detail})
                })
                .collect();
            body["errors"] = Value::Array(fields);
        }
        CallToolResult::structured_error(body)
    }
}

/// Tool deprecated: kết quả giữ nguyên envelope `{"data": …}`, thêm `_meta`
/// `autoxide/deprecation` cho máy và một đoạn văn bản cảnh báo cho mô hình đọc (D-240, như
/// header `Deprecation` / `Sunset` của endpoint — D-059). Tên tool là tập cố định do code khai, nên
/// dùng làm nhãn số đo được.
fn warn_deprecated(
    result: &mut CallToolResult,
    tool: &str,
    deprecation: &crate::mcp::ToolDeprecation,
) {
    metrics::counter!("mcp_deprecated_tool_calls_total", "tool" => tool.to_owned()).increment(1);
    let mut meta = result.meta.take().unwrap_or_default();
    meta.0.insert(
        "autoxide/deprecation".to_owned(),
        json!({"since": deprecation.since, "sunset": deprecation.sunset, "successor": deprecation.successor}),
    );
    result.meta = Some(meta);
    result
        .content
        .push(rmcp::model::ContentBlock::text(format!("Warning: {}", deprecation.sentence(tool))));
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
        let (tenant, principal, _) = caller(&context)?;
        let mut listed = Vec::new();
        for tool in self.tools.iter().filter(|tool| tool.allows(&principal)) {
            // Tool của module gói không có: không liệt kê (lỗi tra quyền dùng: cũng không).
            if matches!(self.plan_allows(tool, tenant.id()).await, Ok((true, _))) {
                listed.push(describe(tool));
            }
        }
        Ok(ListToolsResult::with_all_items(listed))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let (tenant, principal, language) = caller(&context)?;
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
        if tool.is_secret()
            && let Some(secret) = context
                .extensions
                .get::<axum::http::request::Parts>()
                .and_then(|parts| parts.extensions.get::<SecretResult>())
        {
            secret.0.store(true, Ordering::Relaxed);
        }
        let arguments = request.arguments.map_or(Value::Null, Value::Object);
        metrics::counter!("mcp_tool_calls_total").increment(1);
        // Đếm cho hạn mức `kernel.api_requests` (REST và MCP), như lớp HTTP.
        self.entitlements.count_request(tenant.id());
        // Gói của khách không có module của tool: cùng lỗi như REST.
        let (enabled, plan_read_only) = match self.plan_allows(tool, tenant.id()).await {
            Ok(found) => found,
            Err(error) => return Ok(self.failure(tool.name(), &error, language).into()),
        };
        if !enabled {
            let module = tool.name().split('.').next().unwrap_or_default();
            metrics::counter!("entitlement_denials_total", "kind" => "module").increment(1);
            let error = crate::entitlement::module_not_enabled(module);
            return Ok(self.failure(tool.name(), &error, language).into());
        }
        // Khách chỉ đọc (trạng thái khách hay thuê bao): tool đọc vẫn chạy, tool ghi nhận cùng lỗi
        // như REST.
        if (tenant.read_only() || plan_read_only) && !tool.is_read_only() {
            let error = crate::tenancy::errors::tenant_read_only();
            return Ok(self.failure(tool.name(), &error, language).into());
        }
        let mut result = match tool.call(tenant, principal, arguments).await {
            Ok(value) => CallToolResult::structured(envelope(value)),
            Err(error) => self.failure(tool.name(), &error, language),
        };
        if let Some(deprecation) = tool.deprecation() {
            warn_deprecated(&mut result, tool.name(), deprecation);
        }
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
