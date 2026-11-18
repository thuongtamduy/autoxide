//! Chỗ nối cho tự động hoá quy trình (bước 58, module `automations`): sổ tool MCP của mọi module và
//! mẫu luật do module khai.
//!
//! Luật "khi … thì …" chỉ được làm đúng điều tool MCP làm được (luật kiến trúc 8): module
//! `automations` không gọi service của module khác, mà gọi tool qua [`AutomationDirectory::call`] —
//! cùng kiểm permission, gói của khách, khách chỉ đọc như endpoint `/mcp`. Sổ được điền SAU khi mọi
//! module khởi động (tool chỉ có từ lúc đó), như `ResourceDirectory`; module lấy tay cầm bằng
//! [`StartContext::automation_directory`](crate::module::StartContext::automation_directory) lúc
//! `start` và chỉ dùng khi phục vụ.
//!
//! Mẫu luật ([`AutomationTemplate`]) do module khai qua `Module::automation_templates`: thân luật
//! dạng JSON theo định dạng của module `automations` (`trigger`, `conditions`, `actions`); module
//! đó kiểm mẫu như kiểm luật người dùng gửi lên, và chỉ đưa vào danh mục mẫu dùng được.

use std::sync::{Arc, OnceLock};

use serde_json::Value;

use crate::access::Principal;
use crate::entitlement::EntitlementsHandle;
use crate::error::Error;
use crate::mcp::Tool;
use crate::tenancy::TenantContext;

/// Một mẫu luật dựng sẵn do module khai (vd "đơn trên 5 triệu thì gắn nhãn VIP").
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct AutomationTemplate {
    /// `<module>.<tên>`, vd `orders.tag_big_orders`.
    pub id: &'static str,
    /// Tên ngắn tiếng Anh, cho người chọn mẫu.
    pub title: &'static str,
    /// Mẫu làm gì, tiếng Anh.
    pub description: &'static str,
    /// Thân luật: `{"trigger": …, "conditions": …, "actions": […]}` (đặc tả bước 58).
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

/// Sổ tool MCP và mẫu luật của mọi module. Clone rẻ; rỗng tới khi mọi module đã khởi động.
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

    /// Điền sổ (một lần, sau khi mọi module khởi động); lần sau bị bỏ qua.
    pub(crate) fn fill(&self, tools: Vec<Tool>, templates: Vec<AutomationTemplate>) {
        let _ = self.filled.set(Filled { tools, templates });
    }

    /// Sổ đã điền sẵn: cho test của module dựng bộ chạy luật mà không khởi động chương trình.
    pub fn with(tools: Vec<Tool>, templates: Vec<AutomationTemplate>) -> Self {
        let directory = AutomationDirectory::default();
        directory.fill(tools, templates);
        directory
    }

    /// Mọi tool, tên đầy đủ `<module>.<tên>`.
    pub fn tools(&self) -> &[Tool] {
        self.filled.get().map_or(&[], |filled| filled.tools.as_slice())
    }

    /// Tool tên `name` (đầy đủ), nếu có.
    pub fn tool(&self, name: &str) -> Option<&Tool> {
        self.tools().iter().find(|tool| tool.name() == name)
    }

    /// Mọi mẫu luật, theo thứ tự đăng ký module.
    pub fn templates(&self) -> &[AutomationTemplate] {
        self.filled.get().map_or(&[], |filled| filled.templates.as_slice())
    }

    /// Module `module` có bật cho khách `tenant` không (gói, bước 35): event của module chưa bật
    /// không làm kích hoạt được.
    pub async fn module_enabled(
        &self,
        module: &str,
        tenant: &TenantContext,
    ) -> Result<bool, Error> {
        Ok(crate::http::mcp_module_allowed(&self.entitlements, module, tenant.id()).await?.0)
    }

    /// Module của tool có bật cho khách `tenant` không (gói, bước 35).
    pub async fn tool_enabled(&self, tool: &Tool, tenant: &TenantContext) -> Result<bool, Error> {
        Ok(crate::http::mcp_tool_allowed(&self.entitlements, tool, tenant.id()).await?.0)
    }

    /// Gọi tool `tool` cho `principal` ở khách `tenant`, đúng như endpoint `/mcp` gọi: người gọi
    /// thiếu permission thì `forbidden`, module chưa bật cho khách hay khách chỉ đọc thì cùng lỗi
    /// như REST.
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
