//! Tool MCP của module `automations`. Mỗi tool: một kiểu đầu vào (kiểm như thân JSON của REST),
//! permission như endpoint REST tương ứng, thân là MỘT lời gọi service (luật kiến trúc 8). Luật do
//! AI agent tạo hay sửa chờ một người bật (`pending_approval`).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::http::rules::{CreateRuleRequest, RuleResponse};
use crate::permissions::{AUTOMATIONS_MANAGE, AUTOMATIONS_READ};
use crate::service::Automations;
use crate::service::rules::{RuleChanges, RuleFilter};

/// A rule to act on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleId {
    /// Id of the rule.
    id: Uuid,
}

/// A dry run of a rule.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TestRuleInput {
    /// Id of the rule.
    id: Uuid,
    /// What the conditions and templates see as `event`; omit for a sample of the trigger type.
    #[schema(value_type = Option<Object>)]
    event_sample: Option<Value>,
}

/// Changes to a rule; omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateRuleInput {
    /// Id of the rule.
    id: Uuid,
    /// 1 to 200 characters.
    name: Option<String>,
    /// Up to 2000 characters; an empty text removes it.
    #[validate(length(max = 2000))]
    description: Option<String>,
    /// New trigger (see `create_rule`).
    #[schema(value_type = Option<Object>)]
    trigger: Option<Value>,
    /// New conditions; `{}` removes them.
    #[schema(value_type = Option<Object>)]
    conditions: Option<Value>,
    /// New actions (see `create_rule`).
    #[schema(value_type = Option<Vec<Object>>)]
    actions: Option<Value>,
}

pub(crate) fn all(service: &Arc<Automations>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let (list, get, test, create, update) =
        (service.clone(), service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "list_rules",
            "List the automation rules in your scope, newest first; follow \
             `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: PageInput| {
                let service = list.clone();
                async move {
                    let page = service
                        .list_rules(&tenant, &principal, &RuleFilter::default(), &input.page()?)
                        .await?;
                    Ok::<Page<RuleResponse>, Error>(page.map(RuleResponse::from))
                }
            },
        )
        .requires(AUTOMATIONS_READ)
        .read_only(),
        Tool::new(
            "get_rule",
            "Get an automation rule in your scope: trigger, conditions, actions and status.",
            move |tenant: TenantContext, principal: Principal, input: RuleId| {
                let service = get.clone();
                async move {
                    let rule = service.get_rule(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(RuleResponse::from(rule))
                }
            },
        )
        .requires(AUTOMATIONS_READ)
        .read_only(),
        Tool::new(
            "test_rule",
            "Dry-run an automation rule on a sample: whether the conditions hold, the arguments \
             after filling the templates, and what would fail (unknown tool, missing permission of \
             the person the rule runs as, wrong arguments). Calls no tool.",
            move |tenant: TenantContext, principal: Principal, input: TestRuleInput| {
                let service = test.clone();
                async move { service.test_rule(&tenant, &principal, input.id, input.event_sample).await }
            },
        )
        .requires(AUTOMATIONS_READ)
        .read_only(),
        Tool::new(
            "create_rule",
            "Create a \"when … then …\" automation rule. `trigger`: `{\"type\": \"event\", \
             \"event\": \"<module>.<event>\"}`, `{\"type\": \"schedule\", \"every\": \"day\", \
             \"hour\": 9}`, `{\"type\": \"manual\"}` or `{\"type\": \"metric\", …}`; `conditions`: \
             a tree of `all` / `any` / `not` over `{\"field\": \"event.total\", \"op\": \"gt\", \
             \"value\": 5000000}`; `actions`: `[{\"tool\": \"<module>.<tool>\", \"arguments\": \
             {…}}]`, values may use `{{ event.… }}`; see `GET /api/v1/automations/catalog` for \
             the triggers and tools. Rules you create as an agent wait until a person enables \
             them; try a rule with `automations.test_rule`.",
            move |tenant: TenantContext, principal: Principal, input: CreateRuleRequest| {
                let service = create.clone();
                async move {
                    let rule = service.create_rule(&tenant, &principal, input.into()).await?;
                    Ok::<_, Error>(RuleResponse::from(rule))
                }
            },
        )
        .requires(AUTOMATIONS_MANAGE),
        Tool::new(
            "update_rule",
            "Change an automation rule in your scope. Changing the trigger, conditions or actions \
             checks the whole rule again with your permissions; changed by an agent, the rule \
             waits until a person enables it again.",
            move |tenant: TenantContext, principal: Principal, input: UpdateRuleInput| {
                let service = update.clone();
                async move {
                    let changes = RuleChanges {
                        name: input.name,
                        description: input.description.map(Some),
                        trigger: input.trigger,
                        conditions: input.conditions,
                        actions: input.actions,
                    };
                    let rule = service.update_rule(&tenant, &principal, input.id, changes, None).await?;
                    Ok::<_, Error>(RuleResponse::from(rule))
                }
            },
        )
        .requires(AUTOMATIONS_MANAGE),
    ]
}
