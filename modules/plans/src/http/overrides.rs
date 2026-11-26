//! Ngoại lệ quyền dùng theo khách (nhân sự vận hành).

use super::*;

/// List a tenant's overrides.
///
/// Platform staff with `plans.subscriptions.read`. Expired overrides included; newest first
/// unless `order=asc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/tenants/{tenant_id}/overrides",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "list_overrides",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant."), PageQuery),
    responses(
        (status = 200, description = "One page of the overrides, newest first (expired ones included).", body = Page<OverrideResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(super) async fn list_overrides(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<OverrideResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.overrides(TenantId::from_uuid(tenant_id), &page).await?;
    Ok(rows.map(OverrideResponse::from))
}

/// Set an override for a tenant.
///
/// Platform staff with `plans.subscriptions.manage`. Replaces an override of the same kind and key.
#[utoipa::path(
    post,
    path = "/tenants/{tenant_id}/overrides",
    extensions(("x-permissions" = json!(["plans.subscriptions.manage"]))),
    operation_id = "set_override",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant.")),
    request_body = OverrideRequest,
    responses(
        (status = 201, description = "Created.", body = Data<OverrideResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or the tenant is the platform (`plans.platform_exempt`).", body = Problem),
    )
)]
pub(super) async fn set_override(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
    ValidJson(input): ValidJson<OverrideRequest>,
) -> Result<Created<OverrideResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_MANAGE)?;
    let row =
        service.set_override(&principal, TenantId::from_uuid(tenant_id), input.into()).await?;
    Ok(Created::new(format!("/plans/tenants/{tenant_id}/overrides/{}", row.id), row.into()))
}

/// Remove an override of a tenant.
///
/// Platform staff with `plans.subscriptions.manage`.
#[utoipa::path(
    delete,
    path = "/tenants/{tenant_id}/overrides/{id}",
    extensions(("x-permissions" = json!(["plans.subscriptions.manage"]))),
    operation_id = "remove_override",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant."), ("id" = Uuid, Path, description = "Id of the override.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such override; code `plans.override_not_found`.", body = Problem),
    )
)]
pub(super) async fn remove_override(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path((tenant_id, id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, Error> {
    operator(&principal, &SUBSCRIPTIONS_MANAGE)?;
    service.remove_override(Actor::from(&principal), TenantId::from_uuid(tenant_id), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List the overrides of every tenant.
///
/// Platform staff with `plans.subscriptions.read`. To list one tenant's, filter by `tenant_id` or
/// use `GET /tenants/{tenant_id}/overrides`.
#[utoipa::path(
    get,
    path = "/overrides",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "list_all_overrides",
    security(("bearer" = [])),
    params(OverridesQuery, PageQuery),
    responses(
        (status = 200, description = "One page of overrides, newest first.", body = Page<OverrideResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
    )
)]
pub(super) async fn all_overrides(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Query(filter): Query<OverridesQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<OverrideResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    let filter = OverrideFilter {
        tenant_id: filter.tenant_id,
        kind: filter.kind,
        key: filter.key,
        active: filter.active,
    };
    let page = service.all_overrides(&filter, now(), &PageRequest::try_from(page)?).await?;
    Ok(page.map(OverrideResponse::from))
}
