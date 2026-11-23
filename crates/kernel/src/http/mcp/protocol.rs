//! The part of the MCP protocol the kernel writes itself from the public specification (replacing `rmcp`,
//! D-20261121-mcp-server): JSON-RPC 2.0 messages, the protocol version, and errors.
//!
//! Two "generations" of the protocol are served on one endpoint (spec 2026-07-28, section "Versioning"):
//!
//! - **Old** (`2024-11-05` … `2025-11-25`): the client handshakes with `initialize`; later requests carry
//!   the `MCP-Protocol-Version` header (if missing, it is taken as `2025-03-26`). JSON-RPC errors return
//!   HTTP 200.
//! - **New** (`2026-07-28`): no handshake; each request carries the client's version and capabilities in
//!   `params._meta`, the `Mcp-Method` / `Mcp-Name` headers must match the body; there is `server/discover`;
//!   results carry `resultType`. Some errors change HTTP code (400, 404).
//!
//! The server keeps no session (same as the earlier `rmcp` configuration): each request is self-contained.

use serde_json::{Map, Value, json};

/// The protocol versions the server supports, newest first.
pub(super) const SUPPORTED_VERSIONS: [&str; 5] =
    ["2026-07-28", "2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The first version without `initialize` (the new generation).
pub(super) const MODERN: &str = "2026-07-28";

/// The newest version that still has the `initialize` handshake: returned to a client whose `initialize`
/// asks for an unknown version.
pub(super) const LATEST_WITH_INITIALIZE: &str = "2025-11-25";

/// The `_meta` key of the specification.
pub(super) const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub(super) const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
pub(super) const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// JSON-RPC 2.0 and MCP error codes.
pub(super) mod code {
    pub(crate) const PARSE_ERROR: i64 = -32700;
    pub(crate) const INVALID_REQUEST: i64 = -32600;
    pub(crate) const METHOD_NOT_FOUND: i64 = -32601;
    pub(crate) const INVALID_PARAMS: i64 = -32602;
    pub(crate) const INTERNAL_ERROR: i64 = -32603;
    pub(crate) const HEADER_MISMATCH: i64 = -32020;
    pub(crate) const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;
}

/// Whether a version is supported.
pub(super) fn is_supported(version: &str) -> bool {
    SUPPORTED_VERSIONS.contains(&version)
}

/// Whether a version belongs to the new generation (no `initialize`). Versions of the form `YYYY-MM-DD`
/// compare correctly as strings.
pub(super) fn is_modern(version: &str) -> bool {
    version >= MODERN
}

/// The version returned for `initialize`: exactly the one the client asks for if it is supported and still
/// has `initialize`, otherwise the newest version that still has `initialize` (the client decides whether
/// it can use it).
pub(super) fn negotiate_initialize(requested: &str) -> &'static str {
    SUPPORTED_VERSIONS
        .iter()
        .copied()
        .find(|version| *version == requested && !is_modern(version))
        .unwrap_or(LATEST_WITH_INITIALIZE)
}

/// The ID of a JSON-RPC request: a number or a string, returned exactly as the client sent it.
pub(super) type RequestId = Value;

/// A message the client sent, parsed.
pub(super) enum Incoming {
    /// Has an `id`: must be answered.
    Request { id: RequestId, method: String, params: Map<String, Value> },
    /// No `id`: only received (`notifications/initialized`, `notifications/cancelled`...).
    Notification,
    /// The client sends a reply (`result` / `error`): the server keeps no session so no request is
    /// waiting - rejected.
    Response,
}

/// Parses a POST body into a message. On error: the message to return to the client (without an `id` when
/// it cannot be read).
pub(super) fn parse(body: &[u8]) -> Result<Incoming, RpcError> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|error| RpcError::new(code::PARSE_ERROR, format!("Parse error: {error}")))?;
    let Value::Object(mut object) = value else {
        // Đặc tả 2025-06-18 bỏ gửi theo lô: thân phải là một object.
        return Err(RpcError::new(
            code::INVALID_REQUEST,
            "Invalid Request: expected one JSON-RPC message",
        ));
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(RpcError::new(
            code::INVALID_REQUEST,
            "Invalid Request: `jsonrpc` must be \"2.0\"",
        ));
    }
    let Some(method) = object.remove("method") else {
        if object.contains_key("result") || object.contains_key("error") {
            return Ok(Incoming::Response);
        }
        return Err(RpcError::new(code::INVALID_REQUEST, "Invalid Request: missing `method`"));
    };
    let Value::String(method) = method else {
        return Err(RpcError::new(
            code::INVALID_REQUEST,
            "Invalid Request: `method` must be a string",
        ));
    };
    let params = match object.remove("params") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(params)) => params,
        Some(_) => {
            return Err(RpcError::new(
                code::INVALID_REQUEST,
                "Invalid Request: `params` must be an object",
            ));
        }
    };
    match object.remove("id") {
        None => Ok(Incoming::Notification),
        Some(id @ (Value::String(_) | Value::Number(_))) => {
            Ok(Incoming::Request { id, method, params })
        }
        Some(_) => Err(RpcError::new(
            code::INVALID_REQUEST,
            "Invalid Request: `id` must be a string or an integer",
        )),
    }
}

/// The request's `params._meta` (empty when absent).
pub(super) fn meta(params: &Map<String, Value>) -> Option<&Map<String, Value>> {
    params.get("_meta").and_then(Value::as_object)
}

/// A JSON-RPC error returned to the client.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RpcError {
    pub(super) code: i64,
    pub(super) message: String,
    pub(super) data: Option<Value>,
}

impl RpcError {
    pub(super) fn new(code: i64, message: impl Into<String>) -> Self {
        RpcError { code, message: message.into(), data: None }
    }

    pub(super) fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    pub(super) fn invalid_params(message: impl Into<String>) -> Self {
        RpcError::new(code::INVALID_PARAMS, message)
    }

    pub(super) fn internal() -> Self {
        RpcError::new(code::INTERNAL_ERROR, "internal error")
    }

    pub(super) fn method_not_found(method: &str) -> Self {
        RpcError::new(code::METHOD_NOT_FOUND, format!("Method not found: {method}"))
    }

    pub(super) fn header_mismatch(message: impl Into<String>) -> Self {
        RpcError::new(code::HEADER_MISMATCH, format!("Header mismatch: {}", message.into()))
    }

    pub(super) fn unsupported_version(requested: &str) -> Self {
        RpcError::new(code::UNSUPPORTED_PROTOCOL_VERSION, "Unsupported protocol version")
            .with_data(json!({"supported": SUPPORTED_VERSIONS, "requested": requested}))
    }

    /// A JSON-RPC error message (`id` is `null` when the request ID cannot be read).
    pub(super) fn into_message(self, id: Option<RequestId>) -> Value {
        let mut error = json!({"code": self.code, "message": self.message});
        if let Some(data) = self.data {
            error["data"] = data;
        }
        json!({"jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "error": error})
    }
}

/// A JSON-RPC result message.
pub(super) fn result_message(id: RequestId, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// A block of text content (`ContentBlock` of type `text`).
pub(super) fn text_block(text: impl Into<String>) -> Value {
    json!({"type": "text", "text": text.into()})
}

/// A structured `tools/call` result: the value is in `structuredContent`, with a text copy of that same
/// value in `content` for clients that read only text (like `CallToolResult::structured` of `rmcp`).
pub(super) fn tool_result(value: Value, is_error: bool) -> Map<String, Value> {
    let mut result = Map::new();
    result.insert("content".to_owned(), Value::Array(vec![text_block(value.to_string())]));
    result.insert("structuredContent".to_owned(), value);
    result.insert("isError".to_owned(), Value::Bool(is_error));
    result
}

/// Server information (`Implementation`).
pub(super) fn server_info() -> Value {
    json!({"name": "autoxide", "version": env!("CARGO_PKG_VERSION")})
}

/// The capabilities the server declares: tools, resources, prompts (no change notifications are sent).
pub(super) fn capabilities() -> Value {
    json!({"prompts": {}, "resources": {}, "tools": {}})
}

/// Decodes `=?base64?…?=` in the `Mcp-Name` / `Mcp-Param-*` headers (spec: "Value Encoding"); plain values
/// are kept as is. `None` when the base64 form is malformed.
pub(super) fn decode_header_value(raw: &str) -> Option<String> {
    use base64::Engine as _;
    match raw.strip_prefix("=?base64?").and_then(|rest| rest.strip_suffix("?=")) {
        Some(encoded) => base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok()),
        None => Some(raw.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_request_thong_bao_va_tra_loi() {
        let request = parse(br#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#).unwrap();
        assert!(matches!(request, Incoming::Request { ref method, .. } if method == "tools/list"));
        let notification =
            parse(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).unwrap();
        assert!(matches!(notification, Incoming::Notification));
        let response = parse(br#"{"jsonrpc":"2.0","id":1,"result":{}}"#).unwrap();
        assert!(matches!(response, Incoming::Response));
        assert_eq!(parse(b"{").err().unwrap().code, code::PARSE_ERROR);
        assert_eq!(parse(b"[]").err().unwrap().code, code::INVALID_REQUEST);
        assert_eq!(
            parse(br#"{"jsonrpc":"2.0","id":null,"method":"x"}"#).err().unwrap().code,
            code::INVALID_REQUEST
        );
        assert_eq!(parse(br#"{"id":1,"method":"x"}"#).err().unwrap().code, code::INVALID_REQUEST);
    }

    #[test]
    fn chon_phien_ban_cho_initialize() {
        assert_eq!(negotiate_initialize("2025-06-18"), "2025-06-18");
        assert_eq!(negotiate_initialize("2024-11-05"), "2024-11-05");
        assert_eq!(negotiate_initialize("2026-07-28"), LATEST_WITH_INITIALIZE);
        assert_eq!(negotiate_initialize("1900-01-01"), LATEST_WITH_INITIALIZE);
        assert!(is_modern("2026-07-28") && !is_modern("2025-11-25"));
    }

    #[test]
    fn giai_header_base64() {
        assert_eq!(decode_header_value("notes.create").as_deref(), Some("notes.create"));
        assert_eq!(
            decode_header_value("=?base64?SGVsbG8sIOS4lueVjA==?=").as_deref(),
            Some("Hello, 世界")
        );
        assert_eq!(decode_header_value("=?base64?***?="), None);
    }

    #[test]
    fn loi_phien_ban_liet_ke_phien_ban_ho_tro() {
        let message = RpcError::unsupported_version("1900-01-01").into_message(Some(json!(1)));
        assert_eq!(message["error"]["code"], -32022);
        assert_eq!(message["error"]["data"]["requested"], "1900-01-01");
        assert_eq!(message["error"]["data"]["supported"][0], "2026-07-28");
    }
}
