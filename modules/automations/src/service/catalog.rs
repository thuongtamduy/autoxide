//! Danh mục cho người dựng luật: kích hoạt (event, số liệu) và hành động (tool) người gọi dùng
//! được, mẫu luật do module khai; và chạy thử khô một luật — tính điều kiện, điền mẫu, kiểm quyền,
//! KHÔNG gọi tool nào.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::TenantContext;
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::Automations;
use super::rules::parse_json;
use super::validate::RawBody;
use crate::definition::{Action, RuleBody, Trigger, fill, schema};
use crate::permissions::AUTOMATIONS_READ;

/// An event that can trigger a rule.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub(crate) struct EventTriggerInfo {
    /// `<module>.<event>`.
    #[schema(example = "notes.note_created")]
    pub name: String,
    pub description: String,
}

/// A figure that a `metric` trigger can watch.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub(crate) struct MetricTriggerInfo {
    /// `fact:<kind>` (daily total of a report figure) or `summary:<id>` (a dashboard figure read
    /// at check time).
    pub name: String,
    pub description: String,
    /// `amount` (smallest unit of a currency), `quantity`, or `reading` (a dashboard figure;
    /// ratios in basis points, 1% = 100).
    pub unit: String,
}

/// An MCP tool that a rule can run as an action.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub(crate) struct ActionInfo {
    /// Full tool name `<module>.<tool>`.
    pub tool: String,
    pub description: String,
    /// JSON Schema of `arguments`.
    #[schema(value_type = Object)]
    pub input_schema: Value,
    /// The tool deletes or overwrites data, or needs a person's approval when an agent calls it:
    /// in a rule, the step waits for a person to approve each run.
    pub needs_approval: bool,
    /// The permission the tool needs.
    pub permission: Option<String>,
}

/// The triggers and actions you may use in rules.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub(crate) struct Catalog {
    pub events: Vec<EventTriggerInfo>,
    pub metrics: Vec<MetricTriggerInfo>,
    pub tools: Vec<ActionInfo>,
}

/// A ready-made rule you can save now.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub(crate) struct TemplateInfo {
    /// `<module>.<name>`.
    pub id: String,
    pub title: String,
    pub description: String,
    /// `{"trigger", "conditions", "actions"}`: send it to `POST /rules` with a name.
    #[schema(value_type = Object)]
    pub rule: Value,
}

/// What one step would do in a dry run.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub(crate) struct DryStep {
    pub index: usize,
    /// The tool name, or `wait`, `stop_if`, `notify`.
    pub action: String,
    /// The arguments after filling the templates with the sample.
    #[schema(value_type = Option<Object>)]
    pub arguments: Option<Value>,
    /// For `stop_if`: whether the run would stop here (results of earlier steps are unknown in a
    /// dry run and read as missing).
    pub would_stop: Option<bool>,
    /// The step waits for a person's approval in each real run.
    pub needs_approval: bool,
    /// What would go wrong: unknown tool, missing permission of the rule's `run_as`, arguments
    /// that do not fit the tool after filling.
    pub problems: Vec<DryProblem>,
}

/// A problem a real run would hit.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub(crate) struct DryProblem {
    pub field: String,
    pub code: String,
    pub message: String,
}

impl From<&FieldError> for DryProblem {
    fn from(error: &FieldError) -> Self {
        DryProblem {
            field: error.field.clone(),
            code: error.code.clone(),
            message: error.message.clone(),
        }
    }
}

/// What a run would do, without calling any tool.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub(crate) struct DryRun {
    /// The conditions hold for the sample: a real run would run the actions.
    pub matched: bool,
    /// What the templates see.
    #[schema(value_type = Object)]
    pub context: Value,
    pub steps: Vec<DryStep>,
    /// No problem anywhere: the rule would run as written.
    pub ok: bool,
}

impl Automations {
    /// Kích hoạt và hành động `principal` dùng được ở khách `tenant`.
    pub(crate) async fn catalog(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Catalog, Error> {
        principal.require(&AUTOMATIONS_READ)?;
        let mut events = Vec::new();
        for info in self.events.iter() {
            let module = info.name().split('.').next().unwrap_or_default();
            if self.directory.module_enabled(module, tenant).await? {
                events.push(EventTriggerInfo {
                    name: info.name().to_owned(),
                    description: info.description().to_owned(),
                });
            }
        }
        let metrics = self
            .metrics
            .list()
            .into_iter()
            .map(|(name, description, unit)| MetricTriggerInfo {
                name,
                description,
                unit: unit.to_owned(),
            })
            .collect();
        let mut tools = Vec::new();
        for tool in self.directory.tools() {
            if tool.name().starts_with("automations.") || !tool.allows(principal) {
                continue;
            }
            if self.directory.tool_enabled(tool, tenant).await? {
                tools.push(ActionInfo {
                    tool: tool.name().to_owned(),
                    description: tool.description().to_owned(),
                    input_schema: tool.input_schema().clone(),
                    needs_approval: tool.is_destructive() || tool.needs_approval_for_agent(),
                    permission: tool.permission().map(|permission| permission.name().to_owned()),
                });
            }
        }
        Ok(Catalog { events, metrics, tools })
    }

    /// Mẫu luật mà `principal` lưu được ngay (mẫu dùng event / tool / quyền họ không có thì ẩn).
    pub(crate) async fn templates(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Vec<TemplateInfo>, Error> {
        principal.require(&AUTOMATIONS_READ)?;
        let mut usable = Vec::new();
        for template in self.directory.templates() {
            let raw = RawBody {
                trigger: template.rule.get("trigger").cloned().unwrap_or(Value::Null),
                conditions: template.rule.get("conditions").cloned().unwrap_or(Value::Null),
                actions: template.rule.get("actions").cloned().unwrap_or(Value::Null),
            };
            match self.validate(tenant, principal, &raw).await {
                Ok(_) => usable.push(TemplateInfo {
                    id: template.id.to_owned(),
                    title: template.title.to_owned(),
                    description: template.description.to_owned(),
                    rule: template.rule.clone(),
                }),
                Err(error) if error.kind() == autoxide_kernel::error::ErrorKind::Internal => {
                    return Err(error);
                }
                Err(_) => {}
            }
        }
        Ok(usable)
    }

    /// Chạy thử khô luật `id` với dữ liệu mẫu `sample` (thành `event`; không gửi thì dùng mẫu theo
    /// loại kích hoạt).
    pub(crate) async fn test_rule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        sample: Option<Value>,
    ) -> Result<DryRun, Error> {
        let rule = self.get_rule(tenant, principal, id).await?;
        let mut errors = Vec::new();
        let body = RuleBody::parse(
            &parse_json(&rule.trigger),
            &parse_json(&rule.conditions),
            &parse_json(&rule.actions),
            &mut errors,
        )
        .ok_or_else(|| Error::validation(errors))?;
        let event = sample.unwrap_or_else(|| match &body.trigger {
            Trigger::Metric(metric) => crate::metrics::sample_event(&metric.metric),
            Trigger::Schedule(_) => json!({"scheduled_at": super::now()}),
            Trigger::Event { .. } | Trigger::Manual => json!({}),
        });
        let trigger = match &body.trigger {
            Trigger::Event { event } => {
                json!({"type": "event", "name": event, "event_id": Uuid::nil()})
            }
            other => json!({"type": other.kind()}),
        };
        let mut context = json!({"event": event, "trigger": trigger, "steps": []});
        let matched = Self::matches(body.conditions.as_ref(), &context);
        // Quyền của người luật chạy thay (không phải người chạy thử).
        let runner = self.rule_principal(tenant, &rule).await?;
        let mut steps = Vec::new();
        for (index, action) in body.actions.iter().enumerate() {
            let mut step = DryStep {
                index,
                action: action.kind().to_owned(),
                arguments: None,
                would_stop: None,
                needs_approval: false,
                problems: Vec::new(),
            };
            let field = format!("actions.{index}");
            match action {
                Action::Tool { tool: name, arguments } => {
                    let mut problems = Vec::new();
                    match self.directory.tool(name) {
                        None => problems.push(FieldError::new(
                            format!("{field}.tool"),
                            "automations.tool_unknown",
                            "names no MCP tool",
                        )),
                        Some(tool) => {
                            step.needs_approval =
                                tool.is_destructive() || tool.needs_approval_for_agent();
                            if !runner.as_ref().is_some_and(|runner| tool.allows(runner)) {
                                problems.push(FieldError::new(
                                    format!("{field}.tool"),
                                    "automations.permission_missing",
                                    "the person the rule runs as cannot use this tool",
                                ));
                            }
                            match fill::fill(
                                &self.templates,
                                &Value::Object(arguments.clone()),
                                &context,
                            ) {
                                Ok(filled) => {
                                    schema::check(
                                        tool.input_schema(),
                                        &filled,
                                        &format!("{field}.arguments"),
                                        &mut problems,
                                    );
                                    step.arguments = Some(filled);
                                }
                                Err(error) => problems.push(
                                    FieldError::new(
                                        format!("{field}.arguments"),
                                        error.code().to_owned(),
                                        "cannot be filled: {reason}",
                                    )
                                    .with_param("reason", error.message()),
                                ),
                            }
                        }
                    }
                    step.problems = problems.iter().map(DryProblem::from).collect();
                }
                Action::StopIf(condition) => step.would_stop = Some(condition.holds(&context)),
                Action::Notify { title, body } => {
                    let filled = json!({"title": title, "body": body});
                    match fill::fill(&self.templates, &filled, &context) {
                        Ok(filled) => step.arguments = Some(filled),
                        Err(error) => step.problems.push(DryProblem {
                            field,
                            code: error.code().to_owned(),
                            message: error.message(),
                        }),
                    }
                }
                Action::Wait { minutes } => step.arguments = Some(json!({"minutes": minutes})),
            }
            if let Some(list) = context["steps"].as_array_mut() {
                list.push(json!({"status": "dry_run", "result": null}));
            }
            steps.push(step);
        }
        if let Some(object) = context.as_object_mut() {
            object.remove("steps");
        }
        let ok = steps.iter().all(|step| step.problems.is_empty());
        Ok(DryRun { matched, context, steps, ok })
    }
}
