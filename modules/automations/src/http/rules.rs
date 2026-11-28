//! Endpoint REST của luật, dưới `/api/v1/automations/rules`.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::permissions::{AUTOMATIONS_MANAGE, AUTOMATIONS_READ};
use crate::service::Automations;
use crate::service::rules::{NewRule, Rule, RuleChanges, RuleFilter, parse_json};
use crate::service::validate::RawBody;

/// An automation rule: "when the trigger happens and the conditions hold, run the actions".
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct RuleResponse {
    id: Uuid,
    /// The organization unit the rule belongs to; the root unit has the id of the tenant.
    org_id: Uuid,
    #[schema(example = "Tag big orders")]
    name: String,
    description: Option<String>,
    /// What starts a run: `{"type": "event", "event": "<module>.<event>"}`, `{"type":
    /// "schedule", "every": "hour" | "day" | "week", "weekday", "hour", "minute", "timezone"}`,
    /// `{"type": "manual"}` or `{"type": "metric", "metric", "operator", "value", "currency",
    /// "hour", "minute", "timezone"}`. See `GET /api/v1/automations/catalog`.
    #[schema(value_type = Object)]
    trigger: Value,
    /// A tree of `all` / `any` / `not` over comparisons `{"field": "event.total", "op": "gt",
    /// "value": 5000000}` (`op`: eq, ne, gt, gte, lt, lte, in, contains, exists); `null`: always.
    #[schema(value_type = Option<Object>)]
    conditions: Value,
    /// Steps in order: `{"tool": "<module>.<tool>", "arguments": {…}}` (values may use `{{
    /// event.… }}`), `{"wait": {"minutes": N}}`, `{"stop_if": <condition>}`, `{"notify": {"title",
    /// "body"}}`.
    #[schema(value_type = Vec<Object>)]
    actions: Value,
    /// `active`, `disabled`, `pending_approval` (made by an AI agent: a person must enable it) or
    /// `paused` (stopped by itself; see `status_reason`).
    #[schema(example = "active")]
    status: String,
    /// Why the rule paused: `rate_limit`, `permission_lost`, `owner_unavailable`,
    /// `credential_revoked` (the API key that created or enabled it was revoked), or
    /// `owner_access_revoked` (the account it runs as was locked out, recovered by email,
    /// disabled or deleted; a person must check the rule and enable it again).
    status_reason: Option<String>,
    /// IANA time zone of the schedule.
    #[schema(example = "Asia/Ho_Chi_Minh")]
    timezone: String,
    /// The actions run with the current permissions of this user: the author, or whoever enabled
    /// the rule last.
    run_as: Uuid,
    created_by: Uuid,
    /// Raised by one on every change of the rule; each run records the version it used.
    version: i32,
    /// When the schedule or metric check runs next.
    next_run_at: Option<Timestamp>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<Rule> for RuleResponse {
    fn from(rule: Rule) -> Self {
        RuleResponse {
            id: rule.id,
            org_id: rule.org_id,
            name: rule.name,
            description: rule.description,
            trigger: parse_json(&rule.trigger),
            conditions: parse_json(&rule.conditions),
            actions: parse_json(&rule.actions),
            status: rule.status,
            status_reason: rule.status_reason,
            timezone: rule.timezone,
            run_as: rule.run_as,
            created_by: rule.created_by,
            version: rule.version,
            next_run_at: rule.next_run_at,
            created_at: rule.created_at,
            updated_at: rule.updated_at,
        }
    }
}

/// A new automation rule. Every problem (unknown event or tool, a tool you lack the permission
/// for, arguments that do not fit the tool) is reported at once.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateRuleRequest {
    /// 1 to 200 characters.
    #[schema(example = "Tag big orders")]
    name: String,
    /// Up to 2000 characters.
    #[validate(length(max = 2000))]
    description: Option<String>,
    /// The organization unit of the rule; you need `automations.manage` there. Omit when you
    /// manage rules in one unit only (or for the whole tenant); otherwise
    /// `automations.org_required`.
    org_id: Option<Uuid>,
    /// See `trigger` of the rule.
    #[schema(value_type = Object)]
    trigger: Value,
    /// See `conditions` of the rule; omit for "always".
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    conditions: Value,
    /// See `actions` of the rule.
    #[schema(value_type = Vec<Object>)]
    actions: Value,
    /// `false` to save the rule disabled. Rules made by an AI agent always wait for a person to
    /// enable them.
    #[serde(default = "enabled_by_default")]
    enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

impl From<CreateRuleRequest> for NewRule {
    fn from(input: CreateRuleRequest) -> Self {
        NewRule {
            name: input.name,
            description: input.description,
            org_id: input.org_id,
            body: RawBody {
                trigger: input.trigger,
                conditions: input.conditions,
                actions: input.actions,
            },
            enabled: input.enabled,
        }
    }
}

/// Changes to an automation rule. Omitted fields stay as they are. Changing the trigger,
/// conditions or actions checks the whole rule again with your permissions, and the rule then
/// runs as you.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateRuleRequest {
    /// 1 to 200 characters.
    name: Option<String>,
    /// Up to 2000 characters; an empty text removes it.
    #[validate(length(max = 2000))]
    description: Option<String>,
    #[schema(value_type = Option<Object>)]
    trigger: Option<Value>,
    /// New conditions; `{}` removes them (the rule then always runs its actions).
    #[schema(value_type = Option<Object>)]
    conditions: Option<Value>,
    #[schema(value_type = Option<Vec<Object>>)]
    actions: Option<Value>,
}

/// Filters of the rule list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct RuleListQuery {
    /// Only rules with this status: `active`, `disabled`, `pending_approval`, `paused`.
    status: Option<String>,
    /// Only rules with this trigger type: `event`, `schedule`, `manual`, `metric`.
    trigger_type: Option<String>,
}

/// List the automation rules in your scope.
///
/// Requires `automations.read`; its scope decides which rules you see. Newest first unless
/// `order=asc`.
#[utoipa::path(
    get,
    path = "/rules",
    extensions(("x-permissions" = json!(["automations.read"]))),
    operation_id = "list_automation_rules",
    security(("bearer" = [])),
    params(PageQuery, RuleListQuery),
    responses(
        (status = 200, description = "One page of the rules in your scope.", body = Page<RuleResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_rules(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
    Query(query): Query<RuleListQuery>,
) -> Result<Page<RuleResponse>, Error> {
    principal.require(&AUTOMATIONS_READ)?;
    let filter = RuleFilter { status: query.status, trigger_type: query.trigger_type };
    let page =
        service.list_rules(&tenant, &principal, &filter, &PageRequest::try_from(page)?).await?;
    Ok(page.map(RuleResponse::from))
}

/// Create an automation rule.
///
/// Requires `automations.manage` covering the unit of the rule, and the permission of every tool
/// in its actions: the rule runs with your permissions. Made by an AI agent (API key of kind
/// `agent`), the rule waits in `pending_approval` until a person enables it. Retry safely with an
/// `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/rules",
    extensions(("x-permissions" = json!(["automations.manage"]))),
    operation_id = "create_automation_rule",
    security(("bearer" = [])),
    request_body = CreateRuleRequest,
    responses(
        (status = 201, description = "Created.", body = autoxide_kernel::response::Data<RuleResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.manage` (`permission_denied`), `org_id` is outside your scope (`org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 422, description = "The rule is invalid; `errors` lists every problem, with codes such as `automations.trigger_unknown`, `automations.tool_unknown`, `automations.permission_missing` (a tool you lack the permission for) and the codes of wrong arguments.", body = Problem),
    )
)]
pub(crate) async fn create_rule(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateRuleRequest>,
) -> Result<Created<RuleResponse>, Error> {
    principal.require(&AUTOMATIONS_MANAGE)?;
    let rule = service.create_rule(&tenant, &principal, input.into()).await?;
    let mut created =
        Created::new(format!("/automations/rules/{}", rule.id), RuleResponse::from(rule.clone()));
    created.version = Some(rule.updated_at);
    Ok(created)
}

/// Get an automation rule.
///
/// Requires `automations.read` covering the rule; a rule outside your scope answers 404.
#[utoipa::path(
    get,
    path = "/rules/{id}",
    extensions(("x-permissions" = json!(["automations.read"]))),
    operation_id = "get_automation_rule",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the rule.")),
    responses(
        (status = 200, description = "The rule.", body = autoxide_kernel::response::Data<RuleResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No rule with this id in your scope; code `automations.rule_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_rule(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<RuleResponse>, Error> {
    principal.require(&AUTOMATIONS_READ)?;
    let rule = service.get_rule(&tenant, &principal, id).await?;
    Ok(Tagged { version: rule.updated_at, data: rule.into() })
}

/// Change an automation rule.
///
/// Requires `automations.manage` covering the rule, the `If-Match` header with the `ETag` you
/// read, and — when the trigger, conditions or actions change — the permission of every tool in
/// the rule (it then runs as you). Changed by an AI agent, the rule waits for a person again.
#[utoipa::path(
    patch,
    path = "/rules/{id}",
    extensions(("x-permissions" = json!(["automations.manage"]))),
    operation_id = "update_automation_rule",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the rule."), ("If-Match" = String, Header, description = "The `ETag` you read. The change is made only if nobody changed the rule since; otherwise `412`.")),
    request_body = UpdateRuleRequest,
    responses(
        (status = 200, description = "The rule after the change.", body = autoxide_kernel::response::Data<RuleResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.manage` (`permission_denied`), it does not cover this rule (`org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 404, description = "No rule with this id in your scope; code `automations.rule_not_found`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the rule changed since you read it. Get it again and retry; code `precondition_failed`.", body = Problem),
        (status = 422, description = "The rule is invalid; `errors` lists every problem.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn update_rule(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateRuleRequest>,
) -> Result<Tagged<RuleResponse>, Error> {
    principal.require(&AUTOMATIONS_MANAGE)?;
    let expected = if_match.required()?;
    let changes = RuleChanges {
        name: input.name,
        description: input.description.map(Some),
        trigger: input.trigger,
        conditions: input.conditions,
        actions: input.actions,
    };
    let rule = service.update_rule(&tenant, &principal, id, changes, expected).await?;
    Ok(Tagged { version: rule.updated_at, data: rule.into() })
}

/// Delete an automation rule.
///
/// Requires `automations.manage` covering the rule. The log of its runs stays until it expires.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/rules/{id}",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["automations.manage"]))),
    operation_id = "delete_automation_rule",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the rule."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the rule is deleted only if nobody changed it since; otherwise `412`.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.manage` (`permission_denied`), it does not cover this rule (`org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 404, description = "No rule with this id in your scope; code `automations.rule_not_found`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the rule changed since you read it; code `precondition_failed`.", body = Problem),
    )
)]
pub(crate) async fn delete_rule(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<StatusCode, Error> {
    principal.require(&AUTOMATIONS_MANAGE)?;
    service.delete_rule(&tenant, &principal, id, if_match.expected()?).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Enable an automation rule.
///
/// Requires `automations.manage` covering the rule and the permission of every tool in it. The
/// rule is checked again and from now on runs as you. A person must do this: an AI agent cannot
/// enable rules (`automations.human_required`), so rules made by agents need a person's approval.
#[utoipa::path(
    post,
    path = "/rules/{id}/enable",
    extensions(("x-permissions" = json!(["automations.manage"]))),
    operation_id = "enable_automation_rule",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the rule."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read; when given, the rule must not have changed since.")),
    responses(
        (status = 200, description = "The rule, now `active`.", body = autoxide_kernel::response::Data<RuleResponse>, headers(("ETag" = String, description = "Version of the record."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.manage` (`permission_denied`), it does not cover this rule (`org_out_of_scope`), or the caller is an AI agent (`automations.human_required`).", body = Problem),
        (status = 404, description = "No rule with this id in your scope; code `automations.rule_not_found`.", body = Problem),
        (status = 412, description = "`If-Match` does not match; code `precondition_failed`.", body = Problem),
        (status = 422, description = "The rule is no longer valid for you (e.g. you lack a tool's permission); `errors` lists every problem.", body = Problem),
    )
)]
pub(crate) async fn enable_rule(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<Tagged<RuleResponse>, Error> {
    principal.require(&AUTOMATIONS_MANAGE)?;
    let rule = service.enable_rule(&tenant, &principal, id, if_match.expected()?).await?;
    Ok(Tagged { version: rule.updated_at, data: rule.into() })
}

/// Disable an automation rule.
///
/// Requires `automations.manage` covering the rule. Runs already started finish.
#[utoipa::path(
    post,
    path = "/rules/{id}/disable",
    extensions(("x-permissions" = json!(["automations.manage"]))),
    operation_id = "disable_automation_rule",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the rule."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read; when given, the rule must not have changed since.")),
    responses(
        (status = 200, description = "The rule, now `disabled`.", body = autoxide_kernel::response::Data<RuleResponse>, headers(("ETag" = String, description = "Version of the record."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.manage` (`permission_denied`) or it does not cover this rule (`org_out_of_scope`).", body = Problem),
        (status = 404, description = "No rule with this id in your scope; code `automations.rule_not_found`.", body = Problem),
        (status = 412, description = "`If-Match` does not match; code `precondition_failed`.", body = Problem),
    )
)]
pub(crate) async fn disable_rule(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<Tagged<RuleResponse>, Error> {
    principal.require(&AUTOMATIONS_MANAGE)?;
    let rule = service.disable_rule(&tenant, &principal, id, if_match.expected()?).await?;
    Ok(Tagged { version: rule.updated_at, data: rule.into() })
}
