//! Client MCP cho test (thay client của `rmcp`, D-20261121-mcp-server): gửi request JSON-RPC thẳng vào
//! router của app (không mở cổng TCP), theo đặc tả MCP mới (`2026-07-28`, mặc định) hay bản cũ
//! (`initialize`, [`McpClient::legacy`]).
//!
//! Tên hàm và tên trường giống client `rmcp` mà test đã quen (`call_tool`, `list_all_tools`,
//! `structured_content`, `is_error`…), nên đổi thư viện không phải sửa test của module. Client cũng tự kiểm
//! máy chủ trả đúng hình dạng đặc tả đòi (`resultType`, thông tin máy chủ trong `_meta` ở bản mới;
//! không có các trường đó ở bản cũ) — test nào gọi MCP cũng là test tương thích giao thức.

use std::borrow::Cow;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt as _;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tower::ServiceExt as _;

/// Phiên bản mới nhất của đặc tả: không bắt tay, mỗi request mang `_meta`.
pub const MODERN_VERSION: &str = "2026-07-28";
/// Bản cũ mới nhất (còn `initialize`).
pub const LEGACY_VERSION: &str = "2025-11-25";

/// Lỗi khi gọi MCP: HTTP không thành công, hay máy chủ trả lỗi JSON-RPC (in ra `message` của nó).
#[derive(Debug, Clone)]
pub struct McpClientError {
    /// Mã HTTP của response.
    pub status: StatusCode,
    /// Lỗi JSON-RPC (`code`, `message`, `data`), nếu có.
    pub error: Option<Value>,
    /// Thân response (chữ), để in khi test hỏng.
    pub body: String,
}

impl fmt::Display for McpClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.error {
            Some(error) => {
                write!(
                    f,
                    "MCP error {}: {} (HTTP {})",
                    error["code"],
                    error["message"].as_str().unwrap_or_default(),
                    self.status.as_u16()
                )?;
                // `data` mang chi tiết (vd `errors` theo từng ô, như Problem của REST).
                match error.get("data") {
                    Some(data) => write!(f, " data: {data}"),
                    None => Ok(()),
                }
            }
            None => write!(f, "MCP HTTP {}: {}", self.status.as_u16(), self.body),
        }
    }
}

impl std::error::Error for McpClientError {}

fn broken(status: StatusCode, body: impl Into<String>) -> McpClientError {
    McpClientError { status, error: None, body: body.into() }
}

/// Client MCP nối vào `path` (`/mcp`, `/mcp/<module>`) của một router.
#[derive(Clone)]
pub struct McpClient {
    router: Router,
    path: String,
    token: Option<String>,
    /// `None`: bản mới. `Some(v)`: bản cũ đã bắt tay ở phiên bản `v`.
    legacy: Option<String>,
    next_id: std::sync::Arc<AtomicU64>,
}

impl fmt::Debug for McpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpClient").field("path", &self.path).field("legacy", &self.legacy).finish()
    }
}

impl McpClient {
    /// Client bản mới (`2026-07-28`) tới `path` của `router`, xác thực bằng `Bearer <token>`.
    pub fn new(router: Router, path: &str, token: Option<&str>) -> Self {
        McpClient {
            router,
            path: path.to_owned(),
            token: token.map(str::to_owned),
            legacy: None,
            next_id: std::sync::Arc::new(AtomicU64::new(1)),
        }
    }

    /// Như [`McpClient::new`] nhưng nói bản cũ `version`: gửi `initialize` (+ `notifications/initialized`)
    /// rồi mọi request sau mang header `MCP-Protocol-Version: <phiên bản máy chủ chọn>`.
    pub async fn legacy(
        router: Router,
        path: &str,
        token: Option<&str>,
        version: &str,
    ) -> Result<Self, McpClientError> {
        let mut client = McpClient::new(router, path, token);
        let result = client
            .send(
                "initialize",
                json!({
                    "protocolVersion": version,
                    "capabilities": {},
                    "clientInfo": {"name": "autoxide-testkit", "version": "1"},
                }),
                Some(version),
            )
            .await?;
        let chosen = result["protocolVersion"].as_str().unwrap_or(version).to_owned();
        client.legacy = Some(chosen);
        client.notify("notifications/initialized").await?;
        Ok(client)
    }

    /// Phiên bản giao thức đang nói.
    pub fn protocol_version(&self) -> &str {
        self.legacy.as_deref().unwrap_or(MODERN_VERSION)
    }

    /// Gửi một request, trả `result` (lỗi HTTP hay JSON-RPC thành [`McpClientError`]).
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, McpClientError> {
        let version = self.legacy.clone();
        self.send(method, params, version.as_deref()).await
    }

    async fn send(
        &self,
        method: &str,
        params: Value,
        legacy_header: Option<&str>,
    ) -> Result<Value, McpClientError> {
        let mut params = match params {
            Value::Object(params) => params,
            _ => Map::new(),
        };
        let mut builder = self.builder();
        let modern = self.legacy.is_none() && method != "initialize";
        if modern {
            let meta = params.entry("_meta").or_insert_with(|| json!({}));
            meta["io.modelcontextprotocol/protocolVersion"] = json!(MODERN_VERSION);
            meta["io.modelcontextprotocol/clientCapabilities"] = json!({});
            meta["io.modelcontextprotocol/clientInfo"] =
                json!({"name": "autoxide-testkit", "version": "1"});
            builder =
                builder.header("mcp-protocol-version", MODERN_VERSION).header("mcp-method", method);
            let name = match method {
                "tools/call" | "prompts/get" => params.get("name"),
                "resources/read" => params.get("uri"),
                _ => None,
            };
            if let Some(name) = name.and_then(Value::as_str) {
                builder = builder.header("mcp-name", header_value(name));
            }
        } else if let Some(version) = legacy_header {
            builder = builder.header("mcp-protocol-version", version);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let (status, text) = self.exchange(builder, body).await?;
        let message: Value =
            serde_json::from_str(&text).map_err(|_| broken(status, format!("not JSON: {text}")))?;
        if let Some(error) = message.get("error") {
            return Err(McpClientError { status, error: Some(error.clone()), body: text });
        }
        if status != StatusCode::OK || message["id"] != json!(id) || message["jsonrpc"] != "2.0" {
            return Err(broken(status, text));
        }
        let mut result = message["result"].clone();
        check_shape(method, &mut result, modern).map_err(|problem| broken(status, problem))?;
        Ok(result)
    }

    async fn notify(&self, method: &str) -> Result<(), McpClientError> {
        let builder = match &self.legacy {
            Some(version) => self.builder().header("mcp-protocol-version", version.as_str()),
            None => self.builder(),
        };
        let (status, text) =
            self.exchange(builder, json!({"jsonrpc": "2.0", "method": method})).await?;
        if status == StatusCode::ACCEPTED { Ok(()) } else { Err(broken(status, text)) }
    }

    fn builder(&self) -> axum::http::request::Builder {
        let mut builder = Request::builder()
            .method("POST")
            .uri(&self.path)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream");
        if let Some(token) = &self.token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        builder
    }

    async fn exchange(
        &self,
        builder: axum::http::request::Builder,
        body: Value,
    ) -> Result<(StatusCode, String), McpClientError> {
        let request = builder
            .body(Body::from(body.to_string()))
            .map_err(|error| broken(StatusCode::BAD_REQUEST, error.to_string()))?;
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .map_err(|error| broken(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|error| broken(status, error.to_string()))?
            .to_bytes();
        Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// `server/discover` (bản mới).
    pub async fn discover(&self) -> Result<Value, McpClientError> {
        self.request("server/discover", json!({})).await
    }

    /// Gọi một tool.
    pub async fn call_tool(
        &self,
        params: CallToolRequestParams,
    ) -> Result<CallToolResult, McpClientError> {
        let result = self.request("tools/call", serialize(&params)).await?;
        parse(result)
    }

    /// Mọi tool người gọi được dùng.
    pub async fn list_all_tools(&self) -> Result<Vec<Tool>, McpClientError> {
        let result = self.request("tools/list", json!({})).await?;
        parse(result["tools"].clone())
    }

    /// Mọi resource người gọi được đọc.
    pub async fn list_all_resources(&self) -> Result<Vec<Resource>, McpClientError> {
        let result = self.request("resources/list", json!({})).await?;
        parse(result["resources"].clone())
    }

    /// Mọi prompt người gọi được dùng.
    pub async fn list_all_prompts(&self) -> Result<Vec<Prompt>, McpClientError> {
        let result = self.request("prompts/list", json!({})).await?;
        parse(result["prompts"].clone())
    }

    /// Đọc một resource.
    pub async fn read_resource(
        &self,
        params: ReadResourceRequestParams,
    ) -> Result<ReadResourceResult, McpClientError> {
        let result = self.request("resources/read", serialize(&params)).await?;
        parse(result)
    }

    /// Dựng một prompt.
    pub async fn get_prompt(
        &self,
        params: GetPromptRequestParams,
    ) -> Result<GetPromptResult, McpClientError> {
        let result = self.request("prompts/get", serialize(&params)).await?;
        parse(result)
    }

    /// Đóng client. Máy chủ không giữ phiên nên không có gì phải báo; giữ để test viết như với `rmcp`.
    pub async fn cancel(self) -> Result<(), McpClientError> {
        Ok(())
    }
}

/// Kiểm hình dạng kết quả theo thế hệ, rồi bỏ các trường của bản mới để kết quả hai thế hệ đọc như nhau.
fn check_shape(method: &str, result: &mut Value, modern: bool) -> Result<(), String> {
    let Value::Object(object) = result else {
        return Err(format!("`result` of `{method}` is not an object: {result}"));
    };
    if !modern {
        if object.contains_key("resultType") {
            return Err(format!("legacy `{method}` result carries `resultType`"));
        }
        return Ok(());
    }
    if object.remove("resultType") != Some(json!("complete")) {
        return Err(format!("modern `{method}` result lacks `resultType: \"complete\"`"));
    }
    let server = object
        .get_mut("_meta")
        .and_then(Value::as_object_mut)
        .and_then(|meta| meta.remove("io.modelcontextprotocol/serverInfo"));
    if server.as_ref().and_then(|server| server["name"].as_str()) != Some("autoxide") {
        return Err(format!("modern `{method}` result lacks `_meta` serverInfo"));
    }
    if object.get("_meta").and_then(Value::as_object).is_some_and(Map::is_empty) {
        object.remove("_meta");
    }
    let cacheable = matches!(
        method,
        "tools/list"
            | "resources/list"
            | "resources/read"
            | "resources/templates/list"
            | "prompts/list"
    );
    if cacheable || method == "server/discover" {
        let ttl = object.remove("ttlMs");
        let scope = object.remove("cacheScope");
        if !ttl.as_ref().is_some_and(Value::is_u64) || !scope.as_ref().is_some_and(Value::is_string)
        {
            return Err(format!("modern `{method}` result lacks `ttlMs` / `cacheScope`"));
        }
    }
    Ok(())
}

/// Giá trị header `Mcp-Name`: chữ ASCII in được giữ nguyên, còn lại mã hoá `=?base64?…?=` (đặc tả).
fn header_value(value: &str) -> String {
    use base64::Engine as _;
    let plain = value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        && !(value.starts_with("=?base64?") && value.ends_with("?="));
    if plain {
        value.to_owned()
    } else {
        format!("=?base64?{}?=", base64::engine::general_purpose::STANDARD.encode(value))
    }
}

fn serialize<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn parse<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, McpClientError> {
    serde_json::from_value(value.clone()).map_err(|error| {
        broken(StatusCode::OK, format!("unexpected result shape ({error}): {value}"))
    })
}

/// Tham số `tools/call`.
#[derive(Debug, Clone, Serialize)]
pub struct CallToolRequestParams {
    pub name: Cow<'static, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Map<String, Value>>,
}

impl CallToolRequestParams {
    pub fn new(name: impl Into<Cow<'static, str>>) -> Self {
        CallToolRequestParams { name: name.into(), arguments: None }
    }

    pub fn with_arguments(mut self, arguments: Map<String, Value>) -> Self {
        self.arguments = Some(arguments);
        self
    }
}

/// Tham số `resources/read`.
#[derive(Debug, Clone, Serialize)]
pub struct ReadResourceRequestParams {
    pub uri: String,
}

impl ReadResourceRequestParams {
    pub fn new(uri: impl Into<String>) -> Self {
        ReadResourceRequestParams { uri: uri.into() }
    }
}

/// Tham số `prompts/get`.
#[derive(Debug, Clone, Serialize)]
pub struct GetPromptRequestParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Map<String, Value>>,
}

impl GetPromptRequestParams {
    pub fn new(name: impl Into<String>) -> Self {
        GetPromptRequestParams { name: name.into(), arguments: None }
    }

    pub fn with_arguments(mut self, arguments: Map<String, Value>) -> Self {
        self.arguments = Some(arguments);
        self
    }
}

/// Kết quả `tools/call`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    #[serde(default)]
    pub content: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

/// `_meta` của một kết quả (`meta.0["autoxide/deprecation"]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta(pub Map<String, Value>);

/// Một tool trong `tools/list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,
}

/// Gợi ý về tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    pub read_only_hint: Option<bool>,
    pub destructive_hint: Option<bool>,
}

/// Một resource trong `resources/list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

/// Kết quả `resources/read`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadResourceResult {
    pub contents: Vec<ResourceContents>,
}

/// Nội dung một resource (chữ hay nhị phân base64).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResourceContents {
    #[serde(rename_all = "camelCase")]
    TextResourceContents { uri: String, mime_type: Option<String>, text: String },
    #[serde(rename_all = "camelCase")]
    BlobResourceContents { uri: String, mime_type: Option<String>, blob: String },
}

/// Một prompt trong `prompts/list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prompt {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub arguments: Option<Vec<PromptArgument>>,
}

/// Một đối số của prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptArgument {
    pub name: String,
    pub description: Option<String>,
    pub required: Option<bool>,
}

/// Kết quả `prompts/get`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetPromptResult {
    pub description: Option<String>,
    pub messages: Vec<PromptMessage>,
}

/// Một lời trong prompt: vai (`user` / `assistant`) và nội dung (`{"type": "text", "text": …}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptMessage {
    pub role: String,
    pub content: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_ten_ma_hoa_khi_can() {
        assert_eq!(header_value("notes.create"), "notes.create");
        assert_eq!(header_value("autoxide://x y"), "=?base64?YXV0b3hpZGU6Ly94IHk=?=");
        assert_eq!(header_value("Hello, 世界"), "=?base64?SGVsbG8sIOS4lueVjA==?=");
    }

    #[test]
    fn kiem_hinh_dang_ket_qua() {
        let mut legacy = json!({"tools": []});
        assert!(check_shape("tools/list", &mut legacy, false).is_ok());
        let mut modern = json!({
            "tools": [],
            "resultType": "complete",
            "ttlMs": 0,
            "cacheScope": "private",
            "_meta": {"io.modelcontextprotocol/serverInfo": {"name": "autoxide", "version": "1"}}
        });
        assert!(check_shape("tools/list", &mut modern, true).is_ok());
        assert_eq!(modern, json!({"tools": []}));
        let mut missing = json!({"tools": []});
        assert!(check_shape("tools/list", &mut missing, true).is_err());
        let mut leaked = json!({"tools": [], "resultType": "complete"});
        assert!(check_shape("tools/list", &mut leaked, false).is_err());
    }
}
