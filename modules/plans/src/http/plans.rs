//! Gói và giá: danh mục công khai, đọc, tạo, sửa, ngừng bán (nhân sự vận hành).

use super::*;

/// List the plans on sale, with their prices.
///
/// No sign-in needed: a sign-up page shows these. Only active plans and active prices.
#[utoipa::path(
    get,
    path = "/catalog",
    extensions(("x-public" = json!(true))),
    operation_id = "plans_catalog",
    responses((status = 200, description = "Active base plans and add-ons, by code.", body = Data<Vec<PlanResponse>>))
)]
pub(super) async fn catalog(
    State(service): State<Arc<PlansService>>,
) -> Result<Data<Vec<PlanResponse>>, Error> {
    Ok(Data { data: service.catalog().await?.into_iter().map(PlanResponse::from).collect() })
}

/// List every plan, including drafts and retired plans.
///
/// Platform staff with `plans.plans.read`.
#[utoipa::path(
    get,
    path = "/plans",
    extensions(("x-permissions" = json!(["plans.plans.read"]))),
    operation_id = "list_plans",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of plans, newest first.", body = Page<PlanResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.plans.read` (`permission_denied`).", body = Problem),
    )
)]
pub(super) async fn list_plans(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<PlanResponse>, Error> {
    operator(&principal, &PLANS_READ)?;
    Ok(service.list_plans(&PageRequest::try_from(page)?).await?.map(PlanResponse::from))
}

/// Create a plan.
///
/// Platform staff with `plans.plans.manage`. Modules, features and limits must be ones the
/// installed modules declare.
#[utoipa::path(
    post,
    path = "/plans",
    extensions(("x-permissions" = json!(["plans.plans.manage"]))),
    operation_id = "create_plan",
    security(("bearer" = [])),
    request_body = PlanRequest,
    responses(
        (status = 201, description = "Created.", body = Data<PlanResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.plans.manage` (`permission_denied`).", body = Problem),
        (status = 409, description = "Another plan has this code; code `plans.plan_code_taken`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`; an unknown module, feature or limit is `unknown`), or a module lacks a module it needs (`plans.module_dependency_missing`).", body = Problem),
    )
)]
pub(super) async fn create_plan(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    ValidJson(input): ValidJson<PlanRequest>,
) -> Result<Created<PlanResponse>, Error> {
    operator(&principal, &PLANS_MANAGE)?;
    let row = service.create_plan(Actor::from(&principal), input.into()).await?;
    Ok(Created::new(format!("/plans/plans/{}", row.plan.id), row.into()))
}

/// Get a plan.
///
/// Platform staff with `plans.plans.read`.
#[utoipa::path(
    get,
    path = "/plans/{id}",
    extensions(("x-permissions" = json!(["plans.plans.read"]))),
    operation_id = "get_plan",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the plan.")),
    responses(
        (status = 200, description = "The plan.", body = Data<PlanResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.plans.read` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such plan; code `plans.plan_not_found`.", body = Problem),
    )
)]
pub(super) async fn get_plan(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<PlanResponse>, Error> {
    operator(&principal, &PLANS_READ)?;
    let row = service.get_plan(id).await?;
    Ok(Tagged { version: row.plan.updated_at, data: row.into() })
}

/// Replace a plan.
///
/// Platform staff with `plans.plans.manage`. The body is the whole plan; tenants on it get the new
/// modules, features and limits within `AUTOXIDE_PLANS_CACHE_SECONDS`.
#[utoipa::path(
    put,
    path = "/plans/{id}",
    extensions(("x-permissions" = json!(["plans.plans.manage"]))),
    operation_id = "replace_plan",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the plan."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the change is made only if nobody changed the plan since; otherwise `412`.")),
    request_body = PlanRequest,
    responses(
        (status = 200, description = "The plan after the change.", body = Data<PlanResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.plans.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such plan; code `plans.plan_not_found`.", body = Problem),
        (status = 409, description = "Another plan has this code (`plans.plan_code_taken`), the plan is retired (`plans.plan_not_active`), or tenants use it and `kind` would change (`plans.plan_in_use`).", body = Problem),
        (status = 412, description = "`If-Match` does not match: get the plan again and retry; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or a module lacks a module it needs (`plans.module_dependency_missing`).", body = Problem),
    )
)]
pub(super) async fn replace_plan(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<PlanRequest>,
) -> Result<Tagged<PlanResponse>, Error> {
    operator(&principal, &PLANS_MANAGE)?;
    let expected = if_match.expected()?;
    let row = service.update_plan(Actor::from(&principal), id, input.into(), expected).await?;
    Ok(Tagged { version: row.plan.updated_at, data: row.into() })
}

/// Retire a plan.
///
/// Platform staff with `plans.plans.manage`. The plan is no longer sold or the default; tenants on
/// it keep it. A plan is never deleted.
#[utoipa::path(
    post,
    path = "/plans/{id}/retire",
    extensions(("x-permissions" = json!(["plans.plans.manage"]))),
    operation_id = "retire_plan",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the plan.")),
    responses(
        (status = 200, description = "The plan, now `retired`.", body = Data<PlanResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.plans.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such plan; code `plans.plan_not_found`.", body = Problem),
    )
)]
pub(super) async fn retire_plan(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PlanResponse>, Error> {
    operator(&principal, &PLANS_MANAGE)?;
    Ok(Data { data: service.retire_plan(Actor::from(&principal), id).await?.into() })
}

/// Add a price to a plan.
///
/// Platform staff with `plans.plans.manage`. To change a price, add a new one and deactivate the old.
#[utoipa::path(
    post,
    path = "/plans/{id}/prices",
    extensions(("x-permissions" = json!(["plans.plans.manage"]))),
    operation_id = "add_plan_price",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the plan.")),
    request_body = PriceRequest,
    responses(
        (status = 201, description = "The plan with the new price.", body = Data<PlanResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.plans.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such plan; code `plans.plan_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(super) async fn add_price(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<PriceRequest>,
) -> Result<Created<PlanResponse>, Error> {
    operator(&principal, &PLANS_MANAGE)?;
    let row = service.add_price(Actor::from(&principal), id, input.into()).await?;
    Ok(Created::new(format!("/plans/plans/{id}"), row.into()))
}

/// Deactivate a price.
///
/// Platform staff with `plans.plans.manage`. Subscriptions at this price keep it until they change
/// plan.
#[utoipa::path(
    post,
    path = "/prices/{id}/deactivate",
    extensions(("x-permissions" = json!(["plans.plans.manage"]))),
    operation_id = "deactivate_plan_price",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the price.")),
    responses(
        (status = 200, description = "The plan of the price.", body = Data<PlanResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.plans.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such price; code `plans.price_not_found`.", body = Problem),
    )
)]
pub(super) async fn deactivate_price(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PlanResponse>, Error> {
    operator(&principal, &PLANS_MANAGE)?;
    Ok(Data { data: service.deactivate_price(Actor::from(&principal), id).await?.into() })
}
