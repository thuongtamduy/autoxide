//! Tool MCP (Model Context Protocol): AI agent gọi chức năng của hệ thống như công cụ.
//!
//! Module khai tool qua `Module::tools` (gọi sau `start`), như khai route REST. Mỗi tool:
//!
//! - tên `<module>.<tên>` (kernel thêm tiền tố module);
//! - đầu vào là một kiểu Rust (`Deserialize + Validate + ToSchema`) — kiểm y như thân
//!   JSON của REST (`ValidJson`), JSON Schema sinh từ cùng kiểu đó;
//! - permission cần có (nếu có): người gọi thiếu permission thì không thấy và không gọi
//!   được tool;
//! - thân là một lời gọi vào service layer — cùng service với REST, nên tool không làm
//!   được gì mà REST với cùng permission không làm được, và lệnh ghi đi qua event /
//!   audit log như REST (luật kiến trúc 8).
//!
//! ```
//! use autoxide_kernel::access::{Permission, Principal};
//! use autoxide_kernel::mcp::{NoInput, Tool};
//!
//! const NOTES_READ: Permission = Permission::new("notes.read", "Read notes.");
//!
//! let tool = Tool::new("count", "Count the notes of the caller.", |principal: Principal, _: NoInput| async move {
//!     // Gọi service ở đây, vd `service.count(&principal).await`.
//!     Ok(serde_json::json!({ "count": 0 }))
//! })
//! .requires(NOTES_READ)
//! .read_only();
//! ```

use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::{PartialSchema, ToSchema};
use validator::Validate;

use crate::access::{Permission, Principal};
use crate::error::{Error, FieldError};
use crate::pagination::{PageQuery, PageRequest};

// Đầu vào của tool không nhận tham số nào.
/// The tool takes no arguments.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NoInput {}

// Phân trang của tool liệt kê — cùng ý nghĩa với tham số `limit` / `cursor` / `order`
// của endpoint danh sách REST.
/// Paging of a list: the same as the `limit`, `cursor` and `order` query parameters of
/// the REST list endpoints.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PageInput {
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    #[schema(minimum = 1, maximum = 200)]
    pub limit: Option<u32>,
    /// Where to continue: the `next_cursor` of the previous page. Omit for the first page.
    pub cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc` (oldest first).
    pub order: Option<String>,
}

impl PageInput {
    /// Yêu cầu trang đã kiểm, như của REST.
    pub fn page(self) -> Result<PageRequest, Error> {
        PageRequest::try_from(PageQuery {
            limit: self.limit.map(|limit| limit.to_string()),
            cursor: self.cursor,
            order: self.order,
        })
    }
}

/// Thân của một tool: nhận người gọi và đối số JSON, trả kết quả JSON.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait` (xem `api.rs` của module auth).
#[async_trait::async_trait]
pub trait ToolHandler: Send + Sync {
    async fn call(&self, principal: Principal, arguments: Value) -> Result<Value, Error>;
}

/// Một tool MCP do module khai.
#[derive(Clone)]
pub struct Tool {
    name: String,
    description: &'static str,
    permission: Option<Permission>,
    /// Permission thay thế: có một trong hai là được (vd `content.write` hay
    /// `content.write_own` — service kiểm phạm vi chi tiết).
    alternative: Option<Permission>,
    read_only: bool,
    destructive: bool,
    input_schema: Value,
    handler: Arc<dyn ToolHandler>,
}

impl std::fmt::Debug for Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tool").field("name", &self.name).finish_non_exhaustive()
    }
}

impl Tool {
    /// Tool tên `name` (chữ thường, số, `_`; kernel thêm tiền tố `<module>.`), mô tả
    /// tiếng Anh `description` cho AI agent đọc, đầu vào kiểu `I`, thân `handler`.
    pub fn new<I, O, F, Fut>(name: &'static str, description: &'static str, handler: F) -> Self
    where
        I: DeserializeOwned + Validate + ToSchema + Send + 'static,
        O: Serialize + Send + 'static,
        F: Fn(Principal, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<O, Error>> + Send + 'static,
    {
        Tool {
            name: name.to_owned(),
            description,
            permission: None,
            alternative: None,
            read_only: false,
            destructive: false,
            input_schema: serde_json::to_value(<I as PartialSchema>::schema())
                .unwrap_or(Value::Null),
            handler: Arc::new(Typed { handler, input: PhantomData }),
        }
    }

    /// Chỉ người có `permission` mới thấy và gọi được tool.
    pub fn requires(mut self, permission: Permission) -> Self {
        self.permission = Some(permission);
        self
    }

    /// Người có `permission` này (thay cho permission của [`requires`](Self::requires)) cũng
    /// thấy và gọi được tool. Dùng khi một permission hẹp hơn (vd "chỉ mục của mình") đủ để gọi;
    /// service vẫn kiểm phạm vi.
    pub fn or_requires(mut self, permission: Permission) -> Self {
        self.alternative = Some(permission);
        self
    }

    /// Tool chỉ đọc, không đổi gì (gợi ý cho client MCP: gọi không cần hỏi lại).
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Tool xoá hay ghi đè dữ liệu không lấy lại được (gợi ý cho client MCP: hỏi người
    /// dùng trước khi gọi).
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &'static str {
        self.description
    }

    pub fn permission(&self) -> Option<Permission> {
        self.permission
    }

    /// Permission thay thế (xem [`or_requires`](Self::or_requires)).
    pub fn alternative(&self) -> Option<Permission> {
        self.alternative
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn is_destructive(&self) -> bool {
        self.destructive
    }

    /// JSON Schema của đầu vào.
    pub fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    /// Người gọi có được thấy và gọi tool không.
    pub fn allows(&self, principal: &Principal) -> bool {
        self.permission.is_none_or(|permission| principal.has(&permission))
            || self.alternative.is_some_and(|permission| principal.has(&permission))
    }

    /// Gọi tool (không kiểm permission — bên gọi kiểm bằng [`allows`](Self::allows)).
    pub async fn call(&self, principal: Principal, arguments: Value) -> Result<Value, Error> {
        self.handler.call(principal, arguments).await
    }

    /// Đổi tên thành tên đầy đủ `<module>.<tên>`.
    pub(crate) fn qualify(mut self, module: &str) -> Self {
        self.name = format!("{module}.{}", self.name);
        self
    }

    /// Các vấn đề của khai báo (tên sai dạng, schema đầu vào dùng được không).
    pub(crate) fn problems(&self, module: &str) -> Vec<String> {
        let mut problems = Vec::new();
        let short = self.name.strip_prefix(&format!("{module}.")).unwrap_or(&self.name);
        let valid = !short.is_empty()
            && short.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !valid {
            problems.push(format!(
                "tool `{short}` of module `{module}`: the name must be lower-case letters, digits \
                 and `_`"
            ));
        }
        if self.input_schema.get("type").and_then(Value::as_str) != Some("object") {
            problems.push(format!(
                "tool `{}`: the input type must be a struct (a JSON object)",
                self.name
            ));
        }
        let described =
            serde_json::json!({"description": self.description, "input": self.input_schema});
        problems
            .extend(crate::http::untranslated_text(&described, &format!("tool `{}`", self.name)));
        if self.input_schema.to_string().contains("\"$ref\"") {
            problems.push(format!(
                "tool `{}`: the input type must not nest other schema types (MCP clients cannot \
                 resolve `$ref`); use plain fields",
                self.name
            ));
        }
        problems
    }
}

/// Thân tool có kiểu: đọc đối số thành `I`, kiểm, gọi `handler`, đổi kết quả thành JSON.
struct Typed<I, F> {
    handler: F,
    // `PhantomData<fn() -> I>`: nhớ kiểu `I` mà không giữ giá trị nào của nó, và không
    // bắt `I` phải `Sync` để cả struct `Sync`.
    input: PhantomData<fn() -> I>,
}

#[async_trait::async_trait]
impl<I, O, F, Fut> ToolHandler for Typed<I, F>
where
    I: DeserializeOwned + Validate + Send + 'static,
    O: Serialize + Send + 'static,
    F: Fn(Principal, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O, Error>> + Send + 'static,
{
    async fn call(&self, principal: Principal, arguments: Value) -> Result<Value, Error> {
        let arguments = match arguments {
            Value::Null => Value::Object(Map::new()),
            other => other,
        };
        let input: I = serde_json::from_value(arguments).map_err(|error| {
            Error::validation(vec![FieldError::new("arguments", "invalid", error.to_string())])
        })?;
        input.validate()?;
        let output = (self.handler)(principal, input).await?;
        serde_json::to_value(output).map_err(Error::internal)
    }
}
