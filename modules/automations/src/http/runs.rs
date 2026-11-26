//! Endpoint REST của lần chạy, dưới `/api/v1/automations`.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::permissions::{AUTOMATIONS_READ, AUTOMATIONS_RUN};
use crate::runner::StepLog;
use crate::service::Automations;
use crate::service::rules::parse_json;
use crate::service::runs::{Run, RunFilter};

/// One run of an automation rule, with the log of each step.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct RunResponse {
    id: Uuid,
    rule_id: Uuid,
    /// The version of the rule the run was started with. Editing the rule (even only its name) or
    /// enabling it again stops runs still in progress or waiting, with
    /// `automations.rule_changed`.
    rule_version: i32,
    /// The unit of the rule.
    org_id: Uuid,
    /// `queued`, `running`, `waiting` (a `wait` step), `awaiting_approval` (a risky step waits for
    /// a person), `succeeded`, `failed` or `skipped` (not run, see `error_code`).
    #[schema(example = "succeeded")]
    status: String,
    /// What started the run: `event:<id>`, `schedule:<time>`, `metric:<day>` or `manual:<id>`.
    trigger_key: String,
    event_id: Option<Uuid>,
    event_name: Option<String>,
    /// 1 for a run started by a person, a schedule or an ordinary event; one more for each rule
    /// in a chain (a rule's action caused the event of the next rule).
    depth: i32,
    /// The data of the trigger, as the templates see it: `event`, `trigger`.
    #[schema(value_type = Object)]
    context: Value,
    /// The next step to run, from 0.
    cursor: i32,
    steps: Vec<StepLog>,
    /// The step waiting for approval.
    approval_step: Option<i32>,
    error_code: Option<String>,
    error_message: Option<String>,
    /// When a `wait` step ends.
    resume_at: Option<Timestamp>,
    created_at: Timestamp,
    updated_at: Timestamp,
    finished_at: Option<Timestamp>,
}

impl From<Run> for RunResponse {
    fn from(run: Run) -> Self {
        RunResponse {
            id: run.id,
            rule_id: run.rule_id,
            rule_version: run.rule_version,
            org_id: run.org_id,
            status: run.status,
            trigger_key: run.trigger_key,
            event_id: run.event_id,
            event_name: run.event_name,
            depth: run.depth,
            context: parse_json(&run.context),
            cursor: run.cursor,
            steps: serde_json::from_str(&run.steps).unwrap_or_default(),
            approval_step: run.approval_step,
            error_code: run.error_code,
            error_message: run.error_message,
            resume_at: run.resume_at,
            created_at: run.created_at,
            updated_at: run.updated_at,
            finished_at: run.finished_at,
        }
    }
}

/// Filters of the run list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct RunListQuery {
    /// Only runs of this rule.
    rule_id: Option<Uuid>,
    /// Only runs with this status.
    status: Option<String>,
}

/// Data for a manual run.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunRuleRequest {
    /// What the templates see as `event` (e.g. `{"order": {"id": "…"}}`); default `{}`.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub input: Value,
}

/// List the runs of automation rules in your scope.
///
/// Requires `automations.read`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/runs",
    extensions(("x-permissions" = json!(["automations.read"]))),
    operation_id = "list_automation_runs",
    security(("bearer" = [])),
    params(PageQuery, RunListQuery),
    responses(
        (status = 200, description = "One page of runs.", body = Page<RunResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_runs(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
    Query(query): Query<RunListQuery>,
) -> Result<Page<RunResponse>, Error> {
    principal.require(&AUTOMATIONS_READ)?;
    let filter = RunFilter { rule_id: query.rule_id, status: query.status };
    let page =
        service.list_runs(&tenant, &principal, &filter, &PageRequest::try_from(page)?).await?;
    Ok(page.map(RunResponse::from))
}

/// Get a run of an automation rule, with the log of each step.
///
/// Requires `automations.read` covering the rule's unit.
#[utoipa::path(
    get,
    path = "/runs/{id}",
    extensions(("x-permissions" = json!(["automations.read"]))),
    operation_id = "get_automation_run",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the run.")),
    responses(
        (status = 200, description = "The run.", body = Data<RunResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No run with this id in your scope; code `automations.run_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_run(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<RunResponse>, Error> {
    principal.require(&AUTOMATIONS_READ)?;
    let run = service.get_run(&tenant, &principal, id).await?;
    Ok(Data { data: run.into() })
}

/// Run an automation rule now.
///
/// Requires `automations.run` covering the rule. The rule must be `active`; its actions run with
/// the permissions of the rule's `run_as`, in the background — follow the run with `GET
/// /runs/{id}`.
#[utoipa::path(
    post,
    path = "/rules/{id}/run",
    extensions(("x-permissions" = json!(["automations.run"]))),
    operation_id = "run_automation_rule",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the rule.")),
    request_body = RunRuleRequest,
    responses(
        (status = 202, description = "The run was queued.", body = Data<RunResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.run` (`permission_denied`) or it does not cover this rule (`org_out_of_scope`).", body = Problem),
        (status = 404, description = "No rule with this id in your scope; code `automations.rule_not_found`.", body = Problem),
        (status = 409, description = "The rule is not `active` (or just paused itself for running too often); code `automations.rule_paused`.", body = Problem),
    )
)]
pub(crate) async fn run_rule(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<RunRuleRequest>,
) -> Result<(StatusCode, Data<RunResponse>), Error> {
    principal.require(&AUTOMATIONS_RUN)?;
    let run = service.run_rule(&tenant, &principal, id, input.input).await?;
    Ok((StatusCode::ACCEPTED, Data { data: run.into() }))
}

/// Retry a failed run from the step that failed.
///
/// Requires `automations.run` covering the rule. Steps that already succeeded are not done again
/// (each step has its own idempotency key).
#[utoipa::path(
    post,
    path = "/runs/{id}/retry",
    extensions(("x-permissions" = json!(["automations.run"]))),
    operation_id = "retry_automation_run",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the run.")),
    responses(
        (status = 202, description = "The run was queued again.", body = Data<RunResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.run` (`permission_denied`) or it does not cover this rule (`org_out_of_scope`).", body = Problem),
        (status = 404, description = "No run with this id in your scope; code `automations.run_not_found`.", body = Problem),
        (status = 409, description = "The run is not `failed`; code `automations.run_state`.", body = Problem),
    )
)]
pub(crate) async fn retry_run(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Data<RunResponse>), Error> {
    principal.require(&AUTOMATIONS_RUN)?;
    let run = service.retry_run(&tenant, &principal, id).await?;
    Ok((StatusCode::ACCEPTED, Data { data: run.into() }))
}

/// Approve the risky step a run waits on.
///
/// Requires `automations.run` covering the rule and the permission of the step's tool. A person
/// must decide: AI agents get `automations.human_required`. The step then runs once.
#[utoipa::path(
    post,
    path = "/runs/{id}/approve",
    extensions(("x-permissions" = json!(["automations.run"]))),
    operation_id = "approve_automation_run",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the run.")),
    responses(
        (status = 202, description = "Approved; the run continues in the background.", body = Data<RunResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.run` or the tool's permission (`permission_denied`), outside your scope (`org_out_of_scope`), or the caller is an AI agent (`automations.human_required`).", body = Problem),
        (status = 404, description = "No run with this id in your scope; code `automations.run_not_found`.", body = Problem),
        (status = 409, description = "The run is not `awaiting_approval`; code `automations.run_state`.", body = Problem),
    )
)]
pub(crate) async fn approve_run(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Data<RunResponse>), Error> {
    principal.require(&AUTOMATIONS_RUN)?;
    let run = service.approve_run(&tenant, &principal, id).await?;
    Ok((StatusCode::ACCEPTED, Data { data: run.into() }))
}

/// Reject the risky step a run waits on: the run ends as `failed`.
///
/// Requires `automations.run` covering the rule and the permission of the step's tool; a person
/// must decide.
#[utoipa::path(
    post,
    path = "/runs/{id}/reject",
    extensions(("x-permissions" = json!(["automations.run"]))),
    operation_id = "reject_automation_run",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the run.")),
    responses(
        (status = 200, description = "Rejected; the run is `failed` with `automations.approval_rejected`.", body = Data<RunResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.run` or the tool's permission (`permission_denied`), outside your scope (`org_out_of_scope`), or the caller is an AI agent (`automations.human_required`).", body = Problem),
        (status = 404, description = "No run with this id in your scope; code `automations.run_not_found`.", body = Problem),
        (status = 409, description = "The run is not `awaiting_approval`; code `automations.run_state`.", body = Problem),
    )
)]
pub(crate) async fn reject_run(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<RunResponse>, Error> {
    principal.require(&AUTOMATIONS_RUN)?;
    let run = service.reject_run(&tenant, &principal, id).await?;
    Ok(Data { data: run.into() })
}
