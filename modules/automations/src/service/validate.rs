//! Kiểm luật một lần lúc lưu (D-083: mọi lỗi báo một lượt): cấu trúc, event / số liệu có thật, tool
//! có thật và đang bật cho khách, người tạo có permission của MỌI tool, tham số khớp JSON Schema đầu
//! vào của tool, mẫu chỉ dùng biến được phép.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::TenantContext;
use serde_json::Value;

use super::Automations;
use crate::definition::fill;
use crate::definition::{Action, RuleBody, Trigger, schema};

/// Thân luật như client gửi (ba phần JSON).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawBody {
    pub trigger: Value,
    pub conditions: Value,
    pub actions: Value,
}

impl Automations {
    /// Đọc và kiểm thân luật cho người tạo / sửa `principal`. Lỗi nào cũng gom vào một
    /// `validation_failed`.
    pub(crate) async fn validate(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        raw: &RawBody,
    ) -> Result<RuleBody, Error> {
        let mut errors = Vec::new();
        let body = RuleBody::parse(&raw.trigger, &raw.conditions, &raw.actions, &mut errors);
        let Some(body) = body else {
            return Err(Error::validation(errors));
        };
        self.check_trigger(&body.trigger, &mut errors);
        for (index, action) in body.actions.iter().enumerate() {
            let field = format!("actions.{index}");
            match action {
                Action::Tool { tool, arguments } => {
                    self.check_tool(tenant, principal, tool, arguments, &field, &mut errors)
                        .await?;
                }
                Action::Notify { title, body } => {
                    fill::check(
                        &self.templates,
                        &Value::from(title.clone()),
                        &format!("{field}.notify.title"),
                        &mut errors,
                    );
                    fill::check(
                        &self.templates,
                        &Value::from(body.clone()),
                        &format!("{field}.notify.body"),
                        &mut errors,
                    );
                }
                Action::Wait { .. } | Action::StopIf(_) => {}
            }
        }
        if errors.is_empty() { Ok(body) } else { Err(Error::validation(errors)) }
    }

    fn check_trigger(&self, trigger: &Trigger, errors: &mut Vec<FieldError>) {
        match trigger {
            Trigger::Event { event } if !self.events.contains(event) => {
                errors.push(FieldError::new(
                    "trigger.event",
                    "automations.trigger_unknown",
                    "names no known event; see the triggers in `GET /api/v1/automations/catalog`",
                ))
            }
            Trigger::Metric(metric) if !self.metrics.knows(&metric.metric) => {
                errors.push(FieldError::new(
                    "trigger.metric",
                    "automations.trigger_unknown",
                    "names no known metric; see the triggers in `GET /api/v1/automations/catalog`",
                ))
            }
            _ => {}
        }
    }

    /// Một bước gọi tool: tool có thật, bật cho khách, không phải tool của chính module này; người
    /// tạo có permission của nó; tham số khớp schema; mẫu hợp lệ.
    async fn check_tool(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        name: &str,
        arguments: &serde_json::Map<String, Value>,
        field: &str,
        errors: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        let tool_field = format!("{field}.tool");
        let Some(tool) = self.directory.tool(name) else {
            errors.push(unknown_tool(&tool_field));
            return Ok(());
        };
        if !self.directory.tool_enabled(tool, tenant).await? {
            errors.push(unknown_tool(&tool_field));
            return Ok(());
        }
        if name.starts_with("automations.") {
            // Luật gọi luật đi qua event (có chống vòng lặp), không qua tool của module này.
            errors.push(FieldError::new(
                tool_field,
                "automations.tool_not_allowed",
                "cannot be a tool of the automations module itself",
            ));
            return Ok(());
        }
        if !tool.allows(principal) {
            let permission = tool.permission().or(tool.alternative());
            errors.push(
                FieldError::new(
                    tool_field,
                    "automations.permission_missing",
                    "needs the permission `{permission}`, which you do not have: the rule runs with \
                     your permissions",
                )
                .with_param("permission", permission.map_or("", |permission| permission.name())),
            );
            return Ok(());
        }
        let arguments_field = format!("{field}.arguments");
        let value = Value::Object(arguments.clone());
        fill::check(&self.templates, &value, &arguments_field, errors);
        schema::check(tool.input_schema(), &value, &arguments_field, errors);
        Ok(())
    }
}

fn unknown_tool(field: &str) -> FieldError {
    FieldError::new(
        field,
        "automations.tool_unknown",
        "names no MCP tool you may use; see the actions in `GET /api/v1/automations/catalog`",
    )
}
