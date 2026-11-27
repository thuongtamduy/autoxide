//! MCP client for tests (replaces the `rmcp` client, D-20261121-mcp-server): sends JSON-RPC requests straight
//! to the app's router (no TCP port is opened), using either the new MCP spec (`2026-07-28`, the default) or
//! the old one (`initialize`, [`McpClient::legacy`]).
//!
//! Function and field names match the `rmcp` client that tests are used to (`call_tool`, `list_all_tools`,
//! `structured_content`, `is_error`…), so switching libraries needs no changes to module tests. The client
//! also checks that the server returns the shape the spec requires (`resultType`, server info in `_meta` for
//! the new version; none of those fields for the old one) — so every test that calls MCP is also a
//! protocol-compatibility test.

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

/// Latest version of the spec: no handshake, every request carries `_meta`.
pub const MODERN_VERSION: &str = "2026-07-28";
/// Latest old version (still has `initialize`).
pub const LEGACY_VERSION: &str = "2025-11-25";

/// Error when calling MCP: HTTP failure, or the server returned a JSON-RPC error (its `message` is printed).
#[derive(Debug, Clone)]
pub struct McpClientError {
    /// HTTP status code of the response.
    pub status: StatusCode,
    /// JSON-RPC error (`code`, `message`, `data`), if any.
    pub error: Option<Value>,
    /// Response body (text), printed when a test fails.
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

/// MCP client attached to `path` (`/mcp`, `/mcp/<module>`) of a router.
#[derive(Clone)]
pub struct McpClient {
    router: Router,
    path: String,
    token: Option<String>,
    /// Extra headers sent with every request (e.g. `Autoxide-Tenant`).
    headers: Vec<(String, String)>,
    /// `None`: new version. `Some(v)`: old version, handshake done at version `v`.
    legacy: Option<String>,
    next_id: std::sync::Arc<AtomicU64>,
}

impl fmt::Debug for McpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpClient").field("path", &self.path).field("legacy", &self.legacy).finish()
    }
}

impl McpClient {
    /// New-version client (`2026-07-28`) for `path` of `router`, authenticating with `Bearer <token>`.
    pub fn new(router: Router, path: &str, token: Option<&str>) -> Self {
        McpClient {
            router,
            path: path.to_owned(),
            token: token.map(str::to_owned),
            headers: Vec::new(),
            legacy: None,
            next_id: std::sync::Arc::new(AtomicU64::new(1)),
        }
    }

    /// Like [`McpClient::new`] but speaks the old version `version`: sends `initialize` (+
    /// `notifications/initialized`), then every later request carries the header
    /// `MCP-Protocol-Version: <version the server chose>`.
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

    /// Sends `name: value` with every later request — e.g. `Autoxide-Tenant: <sandbox id>` to act inside a
    /// sandbox the token may enter.
    #[must_use]
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// The protocol version being spoken.
    pub fn protocol_version(&self) -> &str {
        self.legacy.as_deref().unwrap_or(MODERN_VERSION)
    }

    /// Sends one request and returns `result` (an HTTP or JSON-RPC error becomes [`McpClientError`]).
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
        // Tên tool lấy trước khi `params` bị chuyển (move) vào thân request.
        let tool = (method == "tools/call")
            .then(|| params.get("name").and_then(Value::as_str).map(str::to_owned))
            .flatten();
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
        if let Some(tool) = tool
            && result.get("isError") != Some(&Value::Bool(true))
        {
            crate::coverage::tool(&tool);
        }
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
        for (name, value) in &self.headers {
            builder = builder.header(name.as_str(), value.as_str());
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
        // Lỗi của `Router` là `Infallible` (không bao giờ xảy ra): `let Ok(…) = …` không cần nhánh lỗi.
        let Ok(response) = self.router.clone().oneshot(request).await;
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|error| broken(status, error.to_string()))?
            .to_bytes();
        Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// `server/discover` (new version).
    pub async fn discover(&self) -> Result<Value, McpClientError> {
        self.request("server/discover", json!({})).await
    }

    /// Calls a tool.
    pub async fn call_tool(
        &self,
        params: CallToolRequestParams,
    ) -> Result<CallToolResult, McpClientError> {
        let result = self.request("tools/call", serialize(&params)).await?;
        parse(result)
    }

    /// Every tool the caller may use.
    pub async fn list_all_tools(&self) -> Result<Vec<Tool>, McpClientError> {
        let result = self.request("tools/list", json!({})).await?;
        parse(result["tools"].clone())
    }

    /// Every resource the caller may read.
    pub async fn list_all_resources(&self) -> Result<Vec<Resource>, McpClientError> {
        let result = self.request("resources/list", json!({})).await?;
        parse(result["resources"].clone())
    }

    /// Every prompt the caller may use.
    pub async fn list_all_prompts(&self) -> Result<Vec<Prompt>, McpClientError> {
        let result = self.request("prompts/list", json!({})).await?;
        parse(result["prompts"].clone())
    }

    /// Reads a resource.
    pub async fn read_resource(
        &self,
        params: ReadResourceRequestParams,
    ) -> Result<ReadResourceResult, McpClientError> {
        let result = self.request("resources/read", serialize(&params)).await?;
        parse(result)
    }

    /// Builds a prompt.
    pub async fn get_prompt(
        &self,
        params: GetPromptRequestParams,
    ) -> Result<GetPromptResult, McpClientError> {
        let result = self.request("prompts/get", serialize(&params)).await?;
        parse(result)
    }

    /// Closes the client. The server keeps no session, so there is nothing to report; kept so tests read as they do with `rmcp`.
    pub async fn cancel(self) -> Result<(), McpClientError> {
        Ok(())
    }
}

/// Checks the result shape for its generation, then strips the new version's fields so both generations read alike.
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

/// Value of the `Mcp-Name` header: printable ASCII is kept as is, anything else is encoded as `=?base64?…?=` (per the spec).
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

/// `tools/call` parameters.
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

/// `resources/read` parameters.
#[derive(Debug, Clone, Serialize)]
pub struct ReadResourceRequestParams {
    pub uri: String,
}

impl ReadResourceRequestParams {
    pub fn new(uri: impl Into<String>) -> Self {
        ReadResourceRequestParams { uri: uri.into() }
    }
}

/// `prompts/get` parameters.
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

/// `tools/call` result.
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

/// The `_meta` of a result (`meta.0["autoxide/deprecation"]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta(pub Map<String, Value>);

/// A tool in `tools/list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,
}

/// Hints about a tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    pub read_only_hint: Option<bool>,
    pub destructive_hint: Option<bool>,
}

/// A resource in `resources/list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

/// `resources/read` result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadResourceResult {
    pub contents: Vec<ResourceContents>,
}

/// Content of a resource (text or base64 binary).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResourceContents {
    #[serde(rename_all = "camelCase")]
    TextResourceContents { uri: String, mime_type: Option<String>, text: String },
    #[serde(rename_all = "camelCase")]
    BlobResourceContents { uri: String, mime_type: Option<String>, blob: String },
}

/// A prompt in `prompts/list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prompt {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub arguments: Option<Vec<PromptArgument>>,
}

/// An argument of a prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptArgument {
    pub name: String,
    pub description: Option<String>,
    pub required: Option<bool>,
}

/// `prompts/get` result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetPromptResult {
    pub description: Option<String>,
    pub messages: Vec<PromptMessage>,
}

/// A message in a prompt: the role (`user` / `assistant`) and content (`{"type": "text", "text": …}`).
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
