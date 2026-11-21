//! Phần giao thức của MCP mà kernel tự viết theo đặc tả công khai (thay `rmcp`, D-20261121-time-library):
//! tin nhắn JSON-RPC 2.0, phiên bản giao thức và lỗi.
//!
//! Hai "thế hệ" giao thức cùng phục vụ trên một endpoint (đặc tả 2026-07-28, mục "Versioning"):
//!
//! - **Cũ** (`2024-11-05` … `2025-11-25`): client bắt tay bằng `initialize`; các request sau mang header
//!   `MCP-Protocol-Version` (thiếu thì hiểu là `2025-03-26`). Lỗi JSON-RPC trả HTTP 200.
//! - **Mới** (`2026-07-28`): không bắt tay; mỗi request mang phiên bản và khả năng của client trong
//!   `params._meta`, header `Mcp-Method` / `Mcp-Name` phải khớp thân; có `server/discover`; kết quả mang
//!   `resultType`. Một số lỗi đổi mã HTTP (400, 404).
//!
//! Máy chủ không giữ phiên (giống cấu hình `rmcp` trước đây): mỗi request tự đủ thông tin.

use serde_json::{Map, Value, json};

/// Các phiên bản giao thức máy chủ hỗ trợ, mới nhất trước.
pub(super) const SUPPORTED_VERSIONS: [&str; 5] =
    ["2026-07-28", "2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Phiên bản đầu tiên không còn `initialize` (thế hệ mới).
pub(super) const MODERN: &str = "2026-07-28";

/// Phiên bản mới nhất còn bắt tay bằng `initialize`: trả cho client `initialize` xin phiên bản lạ.
pub(super) const LATEST_WITH_INITIALIZE: &str = "2025-11-25";

/// Khoá `_meta` của đặc tả.
pub(super) const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub(super) const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
pub(super) const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// Mã lỗi JSON-RPC 2.0 và của MCP.
pub(super) mod code {
    pub(crate) const PARSE_ERROR: i64 = -32700;
    pub(crate) const INVALID_REQUEST: i64 = -32600;
    pub(crate) const METHOD_NOT_FOUND: i64 = -32601;
    pub(crate) const INVALID_PARAMS: i64 = -32602;
    pub(crate) const INTERNAL_ERROR: i64 = -32603;
    pub(crate) const HEADER_MISMATCH: i64 = -32020;
    pub(crate) const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;
}

/// Một phiên bản có được hỗ trợ không.
pub(super) fn is_supported(version: &str) -> bool {
    SUPPORTED_VERSIONS.contains(&version)
}

/// Phiên bản thuộc thế hệ mới (không `initialize`). Phiên bản `YYYY-MM-DD` so như chuỗi là đúng thứ tự.
pub(super) fn is_modern(version: &str) -> bool {
    version >= MODERN
}

/// Phiên bản trả cho `initialize`: đúng bản client xin nếu hỗ trợ và còn `initialize`, không thì bản mới
/// nhất còn `initialize` (client tự quyết có dùng được không).
pub(super) fn negotiate_initialize(requested: &str) -> &'static str {
    SUPPORTED_VERSIONS
        .iter()
        .copied()
        .find(|version| *version == requested && !is_modern(version))
        .unwrap_or(LATEST_WITH_INITIALIZE)
}

/// ID của request JSON-RPC: số hay chuỗi, trả lại nguyên như client gửi.
pub(super) type RequestId = Value;

/// Một tin nhắn client gửi, đã tách ra.
pub(super) enum Incoming {
    /// Có `id`: phải trả lời.
    Request { id: RequestId, method: String, params: Map<String, Value> },
    /// Không `id`: chỉ nhận (`notifications/initialized`, `notifications/cancelled`…).
    Notification,
    /// Client gửi câu trả lời (`result` / `error`): máy chủ không giữ phiên nên không có request nào đang
    /// chờ — từ chối.
    Response,
}

/// Đọc thân POST thành một tin nhắn. Lỗi: tin nhắn trả về cho client (không có `id` khi không đọc được).
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

/// `params._meta` của request (rỗng khi không có).
pub(super) fn meta(params: &Map<String, Value>) -> Option<&Map<String, Value>> {
    params.get("_meta").and_then(Value::as_object)
}

/// Lỗi JSON-RPC trả cho client.
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

    /// Tin nhắn lỗi JSON-RPC (`id` là `null` khi không đọc được ID của request).
    pub(super) fn into_message(self, id: Option<RequestId>) -> Value {
        let mut error = json!({"code": self.code, "message": self.message});
        if let Some(data) = self.data {
            error["data"] = data;
        }
        json!({"jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "error": error})
    }
}

/// Tin nhắn kết quả JSON-RPC.
pub(super) fn result_message(id: RequestId, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// Một khối nội dung chữ (`ContentBlock` loại `text`).
pub(super) fn text_block(text: impl Into<String>) -> Value {
    json!({"type": "text", "text": text.into()})
}

/// Kết quả `tools/call` có cấu trúc: giá trị nằm ở `structuredContent`, kèm bản chữ của chính giá trị đó
/// trong `content` cho client chỉ đọc chữ (như `CallToolResult::structured` của `rmcp`).
pub(super) fn tool_result(value: Value, is_error: bool) -> Map<String, Value> {
    let mut result = Map::new();
    result.insert("content".to_owned(), Value::Array(vec![text_block(value.to_string())]));
    result.insert("structuredContent".to_owned(), value);
    result.insert("isError".to_owned(), Value::Bool(is_error));
    result
}

/// Thông tin máy chủ (`Implementation`).
pub(super) fn server_info() -> Value {
    json!({"name": "autoxide", "version": env!("CARGO_PKG_VERSION")})
}

/// Khả năng máy chủ công bố: tool, resource, prompt (không gửi thông báo thay đổi).
pub(super) fn capabilities() -> Value {
    json!({"prompts": {}, "resources": {}, "tools": {}})
}

/// Giải `=?base64?…?=` của header `Mcp-Name` / `Mcp-Param-*` (đặc tả: "Value Encoding"); giá trị thường giữ
/// nguyên. `None` khi dạng base64 hỏng.
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
