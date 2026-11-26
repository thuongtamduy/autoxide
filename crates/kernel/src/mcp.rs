//! MCP (Model Context Protocol) tools: an AI agent calls the system's functions as tools.
//!
//! A module declares tools through `Module::tools` (called after `start`), like declaring REST routes. Each
//! tool:
//!
//! - is named `<module>.<name>` (the kernel adds the module prefix);
//! - takes a Rust type as input (`Deserialize + Validate + ToSchema`) - validated just like REST's JSON
//!   body (`ValidJson`), with the JSON Schema generated from that same type;
//! - the required permission (if any): a caller lacking the permission cannot see or call
//!   the tool;
//! - the body receives the request's tenant (`TenantContext`), the caller and the input, then calls into
//!   the service layer — the same service as REST, so a tool cannot do
//!   anything REST with the same permission cannot, and writes go through events /
//!   the audit log like REST (architecture rule 8).
//!
//! Besides tools, a module declares **resources** ([`McpResource`]: read-only documents, e.g. a guide or the
//! JSON Schema of a record kind) and **prompts** ([`Prompt`]: prompt templates for common tasks) through
//! `Module::mcp_resources` and `Module::prompts` (step 40 P2). Both are filtered by permission like tools.
//!
//! ```
//! use autoxide_kernel::access::{Permission, Principal};
//! use autoxide_kernel::mcp::{NoInput, Tool};
//! use autoxide_kernel::tenancy::TenantContext;
//!
//! const NOTES_READ: Permission = Permission::new("notes.read", "Read notes.");
//!
//! let tool = Tool::new(
//!     "count",
//!     "Count the notes of the caller.",
//!     |tenant: TenantContext, principal: Principal, _: NoInput| async move {
//!         // Call the service here, e.g. `service.count(&tenant, &principal).await`.
//!         let _ = (tenant, principal);
//!         Ok(serde_json::json!({ "count": 0 }))
//!     },
//! )
//! .requires(NOTES_READ)
//! .read_only();
//! ```

mod prompts;
mod resources;

pub use prompts::{Prompt, PromptArgument, PromptArguments, PromptMessage};
pub use resources::{McpResource, RESOURCE_SCHEME, SCHEMA_MIME, schema_document};

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
use crate::tenancy::TenantContext;

// Đầu vào của tool không nhận tham số nào.
/// The tool takes no arguments.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NoInput {}

// Phân trang của tool liệt kê — cùng ý nghĩa với tham số `limit` / `cursor` / `order`
// của endpoint danh sách REST.
/// Paging of a list: the same as the `limit`, `cursor`, `order` and `count` query parameters
/// of the REST list endpoints.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PageInput {
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    #[schema(minimum = 1, maximum = 200)]
    pub limit: Option<u32>,
    /// Where to continue: the `next_cursor` of the previous page. Omit for the first page.
    pub cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc` (oldest first).
    pub order: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    pub count: Option<bool>,
}

impl PageInput {
    /// Paging with this limit, cursor and order (each optional, as a client sends them).
    pub const fn new(limit: Option<u32>, cursor: Option<String>, order: Option<String>) -> Self {
        PageInput { limit, cursor, order, count: None }
    }

    /// Also asks for `pagination.total` on the first page (the `count` input of a list tool).
    #[must_use]
    pub fn with_count(mut self, count: bool) -> Self {
        self.count = Some(count);
        self
    }

    /// Validated page request, like REST's.
    pub fn page(self) -> Result<PageRequest, Error> {
        let count = self.count.unwrap_or(false);
        PageRequest::try_from(
            PageQuery::new(self.limit.map(|limit| limit.to_string()), self.cursor, self.order)
                .with_count(count),
        )
    }

    // Danh sách có thứ tự tự nhiên (tên, vị trí, mã): cùng kiểu tham số với REST (`NaturalOrderPageQuery`,
    // `order` mặc định `asc`), nên kiểm và lỗi giống hệt endpoint.
    /// Validated page request of a list in a natural order (`order` defaults to `asc`), like REST's.
    pub(crate) fn natural_page(self) -> Result<PageRequest, Error> {
        let count = self.count.unwrap_or(false);
        PageRequest::try_from(
            crate::pagination::NaturalOrderPageQuery::new(
                self.limit.map(|limit| limit.to_string()),
                self.cursor,
                self.order,
            )
            .with_count(count),
        )
    }
}

/// The body of a tool: receives the tenant, the caller and JSON arguments, returns a JSON result.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait` (xem `api.rs` của module auth).
#[async_trait::async_trait]
pub trait ToolHandler: Send + Sync {
    async fn call(
        &self,
        tenant: TenantContext,
        principal: Principal,
        arguments: Value,
    ) -> Result<Value, Error>;
}

/// A deprecated tool (declared with `Deprecation::tool` in `Module::deprecations`, D-240): the date,
/// the removal date and the replacement tool, full names.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ToolDeprecation {
    /// `YYYY-MM-DD` (UTC).
    pub since: String,
    /// Date the tool is removed, `YYYY-MM-DD` (UTC), if decided.
    pub sunset: Option<String>,
    /// The replacement tool, full name `<module>.<name>`.
    pub successor: Option<String>,
}

impl ToolDeprecation {
    /// The English sentence appended to the tool's description and to the warning in the result.
    pub fn sentence(&self, tool: &str) -> String {
        let mut text = format!("Tool `{tool}` is deprecated since {}.", self.since);
        if let Some(sunset) = &self.sunset {
            text.push_str(&format!(" It will be removed on {sunset}."));
        }
        if let Some(successor) = &self.successor {
            text.push_str(&format!(" Use `{successor}` instead."));
        }
        text
    }
}

/// An MCP tool declared by a module.
#[derive(Clone)]
pub struct Tool {
    name: String,
    description: &'static str,
    permission: Option<Permission>,
    /// Alternative permission: having either one is enough (e.g. `content.write` or
    /// `content.write_own` - the service checks the detailed scope).
    alternative: Option<Permission>,
    read_only: bool,
    destructive: bool,
    secret: bool,
    input_schema: Value,
    handler: Arc<dyn ToolHandler>,
    deprecation: Option<ToolDeprecation>,
    /// Tool of an experimental module (set by `Modules::tools`, step 70 P3).
    experimental: bool,
    /// Declared explicitly: anyone signed in may call it (the service limits itself to their own data).
    any_signed_in: bool,
    /// Risky command: when an agent calls it, it is rejected and pointed to REST to ask for approval (step
    /// 51 P12).
    approval_for_agent: bool,
}

impl std::fmt::Debug for Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tool").field("name", &self.name).finish_non_exhaustive()
    }
}

impl Tool {
    /// A tool named `name` (lowercase, digits, `_`; the kernel adds the `<module>.` prefix), with the English
    /// description `description` for AI agents to read, input type `I`, body `handler`.
    pub fn new<I, O, F, Fut>(name: &'static str, description: &'static str, handler: F) -> Self
    where
        I: DeserializeOwned + Validate + ToSchema + Send + 'static,
        O: Serialize + Send + 'static,
        F: Fn(TenantContext, Principal, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<O, Error>> + Send + 'static,
    {
        Tool {
            name: name.to_owned(),
            description,
            permission: None,
            alternative: None,
            read_only: false,
            destructive: false,
            secret: false,
            input_schema: serde_json::to_value(<I as PartialSchema>::schema())
                .unwrap_or(Value::Null),
            handler: Arc::new(Typed { handler, input: PhantomData }),
            deprecation: None,
            experimental: false,
            any_signed_in: false,
            approval_for_agent: false,
        }
    }

    /// Risky command (step 51 P12): the corresponding REST declares `x-approval: "agent"`. When an agent
    /// calls this tool it is rejected (`approval_required`) and pointed to the REST endpoint to ask a person
    /// to approve - MCP cannot do what REST forbids.
    pub fn requires_approval_for_agent(mut self) -> Self {
        self.approval_for_agent = true;
        self
    }

    /// Xem [`requires_approval_for_agent`](Self::requires_approval_for_agent).
    pub fn needs_approval_for_agent(&self) -> bool {
        self.approval_for_agent
    }

    /// Only people with `permission` can see and call the tool.
    pub fn requires(mut self, permission: Permission) -> Self {
        self.permission = Some(permission);
        self
    }

    /// People with this `permission` (in place of the permission of [`requires`](Self::requires)) can also
    /// see and call the tool. Use when a narrower permission (e.g. "only my own entries") is enough to call it;
    /// the service still checks the scope.
    pub fn or_requires(mut self, permission: Permission) -> Self {
        self.alternative = Some(permission);
        self
    }

    /// Anyone signed in may call it, no permission needed - e.g. an action on their own account (the service
    /// limits itself). Like an endpoint's `x-permissions: []`: the permission-coverage test requires every
    /// tool to either have a permission or declare this (D-245).
    pub fn for_any_signed_in(mut self) -> Self {
        self.any_signed_in = true;
        self
    }

    /// Xem [`for_any_signed_in`](Self::for_any_signed_in).
    pub fn is_for_any_signed_in(&self) -> bool {
        self.any_signed_in
    }

    /// A read-only tool that changes nothing (a hint for MCP clients: call without asking again).
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// A tool that deletes or overwrites data irrecoverably (a hint for MCP clients: ask the
    /// user before calling).
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    /// The result contains a secret returned only once (e.g. a new API key): the response carries
    /// `Cache-Control: no-store`, so it is not stored - including by the idempotency layer (D-149).
    pub fn returns_secret(mut self) -> Self {
        self.secret = true;
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

    /// The alternative permission (see [`or_requires`](Self::or_requires)).
    pub fn alternative(&self) -> Option<Permission> {
        self.alternative
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn is_destructive(&self) -> bool {
        self.destructive
    }

    /// Xem [`returns_secret`](Self::returns_secret).
    pub fn is_secret(&self) -> bool {
        self.secret
    }

    /// JSON Schema of the input.
    pub fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    /// Present when the tool is deprecated (set by `Modules::tools` from `Module::deprecations`).
    pub fn deprecation(&self) -> Option<&ToolDeprecation> {
        self.deprecation.as_ref()
    }

    /// The tool belongs to an experimental module: parameters and results may change between releases.
    pub fn is_experimental(&self) -> bool {
        self.experimental
    }

    pub(crate) fn experimental(mut self, experimental: bool) -> Self {
        self.experimental = experimental;
        self
    }

    pub(crate) fn deprecated(mut self, deprecation: ToolDeprecation) -> Self {
        self.deprecation = Some(deprecation);
        self
    }

    /// Whether the caller may see and call the tool.
    pub fn allows(&self, principal: &Principal) -> bool {
        self.permission.is_none_or(|permission| principal.has(&permission))
            || self.alternative.is_some_and(|permission| principal.has(&permission))
    }

    /// Like [`allows`](Self::allows) but as complete as REST: also checks the re-confirmation (step-up) of a
    /// sensitive permission through [`Principal::require`]. The tool runner calls this before running -
    /// `allows` is only for filtering the tool list.
    pub fn authorize(&self, principal: &Principal) -> Result<(), Error> {
        let Some(permission) = self.permission else { return Ok(()) };
        if principal.has(&permission) {
            return principal.require(&permission);
        }
        if let Some(alternative) = self.alternative
            && principal.has(&alternative)
        {
            return principal.require(&alternative);
        }
        principal.require(&permission)
    }

    /// Calls the tool (no permission check - the caller checks with [`allows`](Self::allows)).
    pub async fn call(
        &self,
        tenant: TenantContext,
        principal: Principal,
        arguments: Value,
    ) -> Result<Value, Error> {
        if self.approval_for_agent && principal.actor_kind() == crate::access::ActorKind::Agent {
            return Err(Error::forbidden(
                "approval_required",
                "Tool `{tool}` needs a person's approval when an agent calls it: call its REST \
                 endpoint instead, which answers 202 with an approval request.",
            )
            .with_param("tool", self.name.clone()));
        }
        self.handler.call(tenant, principal, arguments).await
    }

    /// Renames to the full name `<module>.<name>`.
    pub(crate) fn qualify(mut self, module: &str) -> Self {
        self.name = format!("{module}.{}", self.name);
        self
    }

    /// The problems of the declaration (a badly formed name, whether the input schema is usable).
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

/// A typed tool body: reads the arguments as `I`, validates, calls `handler`, converts the result to JSON.
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
    F: Fn(TenantContext, Principal, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O, Error>> + Send + 'static,
{
    async fn call(
        &self,
        tenant: TenantContext,
        principal: Principal,
        arguments: Value,
    ) -> Result<Value, Error> {
        let arguments = match arguments {
            Value::Null => Value::Object(Map::new()),
            other => other,
        };
        if crate::http::value_contains_nul(&arguments) {
            return Err(crate::http::nul_refused());
        }
        let input: I = serde_json::from_value(arguments).map_err(|error| {
            Error::validation(vec![
                FieldError::new("arguments", "invalid", "is not valid: {reason}")
                    .with_param("reason", error),
            ])
        })?;
        input.validate()?;
        let output = (self.handler)(tenant, principal, input).await?;
        serde_json::to_value(output).map_err(Error::internal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::{Risk, StepUp};

    const DROP: Permission =
        Permission::new("demo.drop", "Drop things.").with_risk(Risk::Destructive);

    #[derive(Debug, Deserialize, Validate, ToSchema)]
    struct Nothing {}

    fn tool() -> Tool {
        Tool::new("drop", "Drop.", |_: TenantContext, _: Principal, _: Nothing| async {
            Ok::<_, Error>(())
        })
        .requires(DROP)
    }

    /// A tool called through MCP (or an automation rule) also demands re-confirmation like REST: with the
    /// permission but an old authentication it gives `step_up_required`.
    #[test]
    fn tool_nhay_cam_doi_xac_nhan_lai_nhu_rest() {
        let stale = StepUp {
            authenticated_at: Some(
                crate::time::Timestamp::now() - jiff::SignedDuration::from_hours(2),
            ),
            window: jiff::SignedDuration::from_mins(10),
        };
        let person = Principal::new(uuid::Uuid::nil(), ["demo.drop".to_owned()]);
        let tool = tool();
        assert!(tool.allows(&person.clone().with_step_up(stale)));
        let error = tool.authorize(&person.clone().with_step_up(stale)).unwrap_err();
        assert_eq!(error.code(), "step_up_required");
        let fresh = StepUp { authenticated_at: Some(crate::time::Timestamp::now()), ..stale };
        tool.authorize(&person.clone().with_step_up(fresh)).unwrap();
        // Không gắn step-up (API key, job): như trước.
        tool.authorize(&person).unwrap();
        let other = Principal::new(uuid::Uuid::nil(), Vec::<String>::new());
        assert_eq!(tool.authorize(&other).unwrap_err().code(), "permission_denied");
    }
}
