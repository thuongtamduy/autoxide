//! Hook point for workflow automation (step 58, module `automations`): the registry of every module's MCP
//! tools and the rule templates modules declare.
//!
//! A "when … then …" rule may only do what an MCP tool can do (architecture rule 8): the `automations` module
//! does not call other modules' services but calls tools through [`AutomationDirectory::call`] — with the
//! same permission check, tenant plan and read-only-tenant handling as the `/mcp` endpoint. The registry is
//! filled AFTER every module has started (tools exist only from then on), like `ResourceDirectory`; a module
//! takes the handle with
//! [`StartContext::automation_directory`](crate::module::StartContext::automation_directory) in `start` and
//! only uses it while serving.
//!
//! Rule templates ([`AutomationTemplate`]) are declared by modules through `Module::automation_templates`:
//! the rule body is JSON in the format of the `automations` module (`trigger`, `conditions`, `actions`); that
//! module validates a template as it validates a rule submitted by a user, and only admits usable ones to the
//! template catalog.

use std::sync::{Arc, OnceLock};

use serde_json::Value;

use crate::access::Principal;
use crate::entitlement::EntitlementsHandle;
use crate::error::Error;
use crate::mcp::Tool;
use crate::tenancy::TenantContext;

/// A ready-made rule template declared by a module (e.g. "orders over 5 million get a VIP label").
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct AutomationTemplate {
    /// `<module>.<name>`, vd `orders.tag_big_orders`.
    pub id: &'static str,
    /// Short English name, for the person choosing a template.
    pub title: &'static str,
    /// What the template does, in English.
    pub description: &'static str,
    /// Rule body: `{"trigger": …, "conditions": …, "actions": […]}` (step 58 spec).
    pub rule: Value,
}

impl AutomationTemplate {
    pub fn new(
        id: &'static str,
        title: &'static str,
        description: &'static str,
        rule: Value,
    ) -> Self {
        AutomationTemplate { id, title, description, rule }
    }
}

struct Filled {
    tools: Vec<Tool>,
    templates: Vec<AutomationTemplate>,
}

/// Registry of every module's MCP tools and rule templates. Cheap to clone; empty until every module has started.
//
// `Arc<OnceLock<…>>`: bên điền (`Modules::start`) và bên đọc (module `automations`) giữ cùng một ô;
// ô ghi đúng một lần rồi chỉ đọc, nên đọc từ nhiều thread không cần lock.
#[derive(Clone, Default)]
pub struct AutomationDirectory {
    filled: Arc<OnceLock<Filled>>,
    entitlements: EntitlementsHandle,
}

impl std::fmt::Debug for AutomationDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutomationDirectory").finish_non_exhaustive()
    }
}

impl AutomationDirectory {
    pub(crate) fn new(entitlements: EntitlementsHandle) -> Self {
        AutomationDirectory { filled: Arc::default(), entitlements }
    }

    /// Fills the registry (once, after every module has started); later calls are ignored.
    pub(crate) fn fill(&self, tools: Vec<Tool>, templates: Vec<AutomationTemplate>) {
        let _ = self.filled.set(Filled { tools, templates });
    }

    /// A pre-filled registry: lets a module's tests build a rule runner without starting the program.
    pub fn with(tools: Vec<Tool>, templates: Vec<AutomationTemplate>) -> Self {
        let directory = AutomationDirectory::default();
        directory.fill(tools, templates);
        directory
    }

    /// Every tool, with its full name `<module>.<name>`.
    pub fn tools(&self) -> &[Tool] {
        self.filled.get().map_or(&[], |filled| filled.tools.as_slice())
    }

    /// The tool named `name` (full name), if any.
    pub fn tool(&self, name: &str) -> Option<&Tool> {
        self.tools().iter().find(|tool| tool.name() == name)
    }

    /// Every rule template, in module registration order.
    pub fn templates(&self) -> &[AutomationTemplate] {
        self.filled.get().map_or(&[], |filled| filled.templates.as_slice())
    }

    /// Whether module `module` is enabled for tenant `tenant` (plan, step 35): events of a module that is not
    /// enabled cannot trigger anything.
    pub async fn module_enabled(
        &self,
        module: &str,
        tenant: &TenantContext,
    ) -> Result<bool, Error> {
        Ok(crate::http::mcp_module_allowed(&self.entitlements, module, tenant.id()).await?.0)
    }

    /// Whether the tool's module is enabled for tenant `tenant` (plan, step 35).
    pub async fn tool_enabled(&self, tool: &Tool, tenant: &TenantContext) -> Result<bool, Error> {
        Ok(crate::http::mcp_tool_allowed(&self.entitlements, tool, tenant.id()).await?.0)
    }

    /// Calls tool `tool` for `principal` in tenant `tenant`, exactly as the `/mcp` endpoint does: a caller
    /// without the permission gets `forbidden`; a module not enabled for the tenant or a read-only tenant
    /// gives the same error as REST.
    pub async fn call(
        &self,
        tool: &Tool,
        tenant: TenantContext,
        principal: Principal,
        arguments: Value,
    ) -> Result<Value, Error> {
        if !tool.allows(&principal) {
            // Thiếu permission: cùng lỗi `permission_denied` như REST, nêu permission của tool.
            let permission = tool.permission().or(tool.alternative());
            if let Some(permission) = permission {
                principal.require(&permission)?;
            }
        }
        crate::http::call_mcp_tool(&self.entitlements, tool, tenant, principal, arguments).await
    }
}
