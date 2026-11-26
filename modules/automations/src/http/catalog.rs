//! Endpoint REST của danh mục, mẫu luật và chạy thử khô, dưới `/api/v1/automations`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::permissions::AUTOMATIONS_READ;
use crate::service::Automations;
use crate::service::catalog::{Catalog, DryRun, TemplateInfo};

/// Data for a dry run.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TestRuleRequest {
    /// What the templates and conditions see as `event` (e.g. a sample payload of the trigger
    /// event). Omit to use a sample for the trigger type.
    #[schema(value_type = Option<Object>)]
    pub event_sample: Option<Value>,
}

/// List the triggers and actions you may use in automation rules.
///
/// Requires `automations.read`. Events of modules in your plan, metrics for `metric` triggers,
/// and the MCP tools you have the permission for (with their input schemas).
#[utoipa::path(
    get,
    path = "/catalog",
    extensions(("x-permissions" = json!(["automations.read"]))),
    operation_id = "get_automation_catalog",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Triggers and actions.", body = Data<Catalog>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.read`; code `permission_denied`.", body = Problem),
    )
)]
pub(crate) async fn get_catalog(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Catalog>, Error> {
    principal.require(&AUTOMATIONS_READ)?;
    Ok(Data { data: service.catalog(&tenant, &principal).await? })
}

/// List the ready-made rule templates you can use.
///
/// Requires `automations.read`. Modules declare templates; only those you could save now (their
/// events, metrics and tools are available to you) are listed. Send a template's `rule` with a
/// `name` to `POST /rules`. Ordered by `id` unless `order=desc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/templates",
    extensions(("x-permissions" = json!(["automations.read"]))),
    operation_id = "list_automation_templates",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of templates, by `id`.", body = Page<TemplateInfo>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_templates(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<TemplateInfo>, Error> {
    principal.require(&AUTOMATIONS_READ)?;
    let page = PageRequest::try_from(page)?;
    service.templates(&tenant, &principal, &page).await
}

/// Dry-run an automation rule.
///
/// Requires `automations.read` covering the rule. Evaluates the conditions on the sample, fills
/// the templates and checks the tools and the permissions of the rule's `run_as` — without
/// calling any tool.
#[utoipa::path(
    post,
    path = "/rules/{id}/test",
    extensions(("x-permissions" = json!(["automations.read"]))),
    operation_id = "test_automation_rule",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the rule.")),
    request_body = TestRuleRequest,
    responses(
        (status = 200, description = "What a run would do.", body = Data<DryRun>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `automations.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No rule with this id in your scope; code `automations.rule_not_found`.", body = Problem),
    )
)]
pub(crate) async fn test_rule(
    State(service): State<Arc<Automations>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<TestRuleRequest>,
) -> Result<Data<DryRun>, Error> {
    principal.require(&AUTOMATIONS_READ)?;
    Ok(Data { data: service.test_rule(&tenant, &principal, id, input.event_sample).await? })
}
