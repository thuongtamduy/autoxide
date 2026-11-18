//! Resource MCP: tài liệu chỉ đọc mà agent tự đọc khi cần (bước 40 P2) — hướng dẫn của module,
//! JSON Schema của một loại bản ghi, sổ module…
//!
//! Tên `McpResource` để không lẫn với `autoxide_kernel::resource` (sổ loại bản ghi của D-123).
//! Module khai qua `Module::mcp_resources` (gọi sau `start`); path ngắn (`guide`,
//! `schema/entry`), kernel ghép URI `autoxide://<module>/<path>`. Như tool: mỗi resource hoặc
//! cần một permission (`requires`), hoặc khai tường minh là ai đăng nhập cũng đọc được
//! (`for_any_signed_in`) — thiếu cả hai là lỗi khởi động.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;
use utoipa::ToSchema;
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::Schema;

use crate::access::{Permission, Principal};
use crate::error::Error;
use crate::tenancy::TenantContext;

/// Tiền tố URI của mọi resource.
pub const RESOURCE_SCHEME: &str = "autoxide://";

/// Kiểu MIME của JSON Schema.
pub const SCHEMA_MIME: &str = "application/schema+json";

// `Pin<Box<dyn Future>>`: future của hàm đọc có kiểu khác nhau theo từng resource; đóng hộp để
// cất chung trong một `Vec`.
type ReadFuture = Pin<Box<dyn Future<Output = Result<String, Error>> + Send>>;
type ReadFn = dyn Fn(TenantContext, Principal) -> ReadFuture + Send + Sync;

/// Một resource MCP do module khai.
#[derive(Clone)]
pub struct McpResource {
    /// Path ngắn lúc khai; URI đầy đủ sau khi kernel ghép tên module.
    uri: String,
    name: String,
    description: &'static str,
    mime_type: &'static str,
    permission: Option<Permission>,
    any_signed_in: bool,
    read: Arc<ReadFn>,
}

impl std::fmt::Debug for McpResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpResource").field("uri", &self.uri).finish_non_exhaustive()
    }
}

impl McpResource {
    /// Resource ở `path` (chữ thường, số, `_`, `-`, `.`, `/`), tên `name`, mô tả tiếng Anh cho agent,
    /// kiểu `mime_type`; nội dung do `read` dựng lúc được đọc (nhận khách và người gọi, như tool).
    pub fn new<F, Fut>(
        path: &'static str,
        name: &'static str,
        description: &'static str,
        mime_type: &'static str,
        read: F,
    ) -> Self
    where
        F: Fn(TenantContext, Principal) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, Error>> + Send + 'static,
    {
        McpResource {
            uri: path.to_owned(),
            name: name.to_owned(),
            description,
            mime_type,
            permission: None,
            any_signed_in: false,
            read: Arc::new(move |tenant, principal| Box::pin(read(tenant, principal))),
        }
    }

    /// Tài liệu tĩnh, vd hướng dẫn viết trong file `.md` của crate (`include_str!`).
    pub fn text(
        path: &'static str,
        name: &'static str,
        description: &'static str,
        mime_type: &'static str,
        text: &'static str,
    ) -> Self {
        McpResource::new(path, name, description, mime_type, move |_, _| async move {
            Ok(text.to_owned())
        })
    }

    /// JSON Schema của kiểu `T` — "schema một loại bản ghi", sinh từ cùng kiểu mà OpenAPI dùng nên
    /// không lệch. Kiểu con được nhúng vào `$defs` (`$ref` trỏ `#/$defs/<tên>`), để agent đọc
    /// riêng resource này là đủ.
    pub fn schema<T: ToSchema>(path: &'static str, description: &'static str) -> Self {
        let name = T::name().into_owned();
        let document = schema_document::<T>();
        let text = serde_json::to_string_pretty(&document).unwrap_or_default();
        let mut resource = McpResource::new(path, "", description, SCHEMA_MIME, move |_, _| {
            let text = text.clone();
            async move { Ok(text) }
        });
        resource.name = name;
        resource
    }

    /// Chỉ người có `permission` mới thấy và đọc được resource.
    pub fn requires(mut self, permission: Permission) -> Self {
        self.permission = Some(permission);
        self
    }

    /// Mọi người đã đăng nhập đọc được (tài liệu không có gì riêng tư). Phải khai tường minh, như
    /// tool (D-245).
    pub fn for_any_signed_in(mut self) -> Self {
        self.any_signed_in = true;
        self
    }

    /// URI: path ngắn trước khi kernel ghép, `autoxide://<module>/<path>` sau đó.
    pub fn uri(&self) -> &str {
        &self.uri
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &'static str {
        self.description
    }

    pub fn mime_type(&self) -> &'static str {
        self.mime_type
    }

    pub fn permission(&self) -> Option<Permission> {
        self.permission
    }

    /// Xem [`for_any_signed_in`](Self::for_any_signed_in).
    pub fn is_for_any_signed_in(&self) -> bool {
        self.any_signed_in
    }

    /// Người gọi có được thấy và đọc resource không.
    pub fn allows(&self, principal: &Principal) -> bool {
        self.permission.is_none_or(|permission| principal.has(&permission))
    }

    /// Đọc nội dung (không kiểm permission — bên gọi kiểm bằng [`allows`](Self::allows)).
    pub async fn read(&self, tenant: TenantContext, principal: Principal) -> Result<String, Error> {
        (self.read)(tenant, principal).await
    }

    /// Module của resource (phần giữa `autoxide://` và `/` kế tiếp).
    pub fn module(&self) -> &str {
        self.uri
            .strip_prefix(RESOURCE_SCHEME)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or_default()
    }

    /// Ghép URI đầy đủ `autoxide://<module>/<path>`.
    pub(crate) fn qualify(mut self, module: &str) -> Self {
        self.uri = format!("{RESOURCE_SCHEME}{module}/{}", self.uri);
        self
    }

    /// Các vấn đề của khai báo (path sai dạng, thiếu khai quyền, mô tả chưa dịch).
    pub(crate) fn problems(&self, module: &str) -> Vec<String> {
        let mut problems = Vec::new();
        let path =
            self.uri.strip_prefix(&format!("{RESOURCE_SCHEME}{module}/")).unwrap_or(&self.uri);
        let valid = !path.is_empty()
            && !path.starts_with('/')
            && !path.ends_with('/')
            && !path.contains("//")
            && path.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.' | '/')
            });
        if !valid {
            problems.push(format!(
                "resource `{path}` of module `{module}`: the path must be lower-case letters, \
                 digits, `_`, `-`, `.` and `/` between parts"
            ));
        }
        if self.name.is_empty() || self.description.is_empty() {
            problems.push(format!("resource `{}`: give it a name and a description", self.uri));
        }
        if self.permission.is_none() && !self.any_signed_in {
            problems.push(format!(
                "resource `{}`: declare who may read it, with `requires(permission)` or \
                 `for_any_signed_in()`",
                self.uri
            ));
        }
        let described = serde_json::json!({"description": self.description, "title": self.name});
        problems.extend(crate::http::untranslated_text(
            &described,
            &format!("resource `{}`", self.uri),
        ));
        problems
    }
}

/// JSON Schema đứng một mình của `T`: schema của `T`, thêm `title`, kiểu con trong `$defs`.
pub fn schema_document<T: ToSchema>() -> Value {
    let mut nested: Vec<(String, RefOr<Schema>)> = Vec::new();
    T::schemas(&mut nested);
    let mut document = serde_json::to_value(T::schema()).unwrap_or(Value::Null);
    if let Value::Object(object) = &mut document {
        object.insert(
            "$schema".to_owned(),
            Value::from("https://json-schema.org/draft/2020-12/schema"),
        );
        object.entry("title").or_insert_with(|| Value::from(T::name().into_owned()));
        if !nested.is_empty() {
            let defs: serde_json::Map<String, Value> = nested
                .into_iter()
                .map(|(name, schema)| (name, serde_json::to_value(schema).unwrap_or(Value::Null)))
                .collect();
            object.insert("$defs".to_owned(), Value::Object(defs));
        }
    }
    rewrite_refs(&mut document);
    document
}

/// `#/components/schemas/X` (chỗ OpenAPI để kiểu con) thành `#/$defs/X`.
fn rewrite_refs(node: &mut Value) {
    match node {
        Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                match value {
                    Value::String(target) if key == "$ref" => {
                        if let Some(name) = target.strip_prefix("#/components/schemas/") {
                            *target = format!("#/$defs/{name}");
                        }
                    }
                    _ => rewrite_refs(value),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(rewrite_refs),
        _ => {}
    }
}
