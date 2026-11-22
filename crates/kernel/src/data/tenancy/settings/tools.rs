//! Tool MCP của cấu hình theo khách: liệt kê, đặt cấu hình KHÔNG bí mật, bỏ cấu hình. Dùng chung
//! service với REST.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use validator::Validate;

use super::{SETTINGS_MANAGE, SettingsService};
use crate::access::Principal;
use crate::mcp::Tool;
use crate::tenancy::TenantContext;

// Đầu vào của tool `list_settings`: cùng bộ lọc với REST.
/// Which settings to list.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListSettings {
    /// Only settings of this group, e.g. `connection` (connections to partner services). Omit
    /// for every setting.
    #[schema(example = "connection")]
    group: Option<String>,
}

// Tool MCP: liệt kê, đặt cấu hình KHÔNG bí mật, bỏ cấu hình. Không có tool ghi secret (đặc tả 33,
// D-258): giá trị bí mật không đi qua ngữ cảnh của agent — đặt bằng REST.
/// Input of `kernel.set_setting`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetSetting {
    /// Name of the setting, `<module>.<key>`.
    #[schema(example = "kernel.public_url")]
    name: String,
    /// The value.
    #[schema(example = "https://shop.example")]
    value: String,
}

/// Input of `kernel.delete_setting`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteSetting {
    /// Name of the setting.
    #[schema(example = "kernel.public_url")]
    name: String,
}

/// The answer of `kernel.delete_setting`.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct SettingDeleted {
    name: String,
    deleted: bool,
}

pub(crate) fn tools(service: Arc<SettingsService>) -> Vec<Tool> {
    let (setter, remover) = (service.clone(), service.clone());
    vec![
        Tool::new(
            "set_setting",
            "Set a setting of the current tenant that is not secret, e.g. a plain field of its \
             account at a partner (`<module>.connection.<kind>.<field>`). Secret settings (API \
             keys, passwords) are refused with `kernel.setting_secret_rest_only`: a person sets \
             them with `PUT /api/v1/kernel/settings/{name}`, so the value never passes through \
             an agent.",
            move |tenant: TenantContext, principal: Principal, input: SetSetting| {
                let service = setter.clone();
                async move {
                    service.put_plain(&tenant, &principal, &input.name, &input.value).await
                }
            },
        )
        .requires(SETTINGS_MANAGE),
        Tool::new(
            "delete_setting",
            "Remove a setting of the current tenant; the module then works as when it was never \
             set (a partner connection stops working until it is set again).",
            move |tenant: TenantContext, principal: Principal, input: DeleteSetting| {
                let service = remover.clone();
                async move {
                    service.delete(&tenant, &principal, &input.name).await?;
                    Ok(SettingDeleted { name: input.name, deleted: true })
                }
            },
        )
        .requires(SETTINGS_MANAGE)
        .destructive(),
        Tool::new(
            "list_settings",
            "List the settings of the current tenant: every setting the installed modules take \
             per tenant, whether it is set, and its value unless it is secret. `group: \
             \"connection\"` lists only the connections to partner services (mail server, AI \
             provider…).",
            move |tenant: TenantContext, _principal: Principal, filter: ListSettings| {
                let service = service.clone();
                async move { service.list(&tenant, filter.group.as_deref()).await }
            },
        )
        .requires(SETTINGS_MANAGE)
        .read_only(),
    ]
}

// --- Migration ------------------------------------------------------------------------------
