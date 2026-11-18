//! Thuê bao: của khách đang gọi, danh sách và gán gói (nhân sự vận hành).

use super::*;

/// Get your tenant's subscription.
///
/// Requires `plans.billing.read`.
#[utoipa::path(
    get,
    path = "/subscription",
    extensions(("x-permissions" = json!(["plans.billing.read"]))),
    operation_id = "get_own_subscription",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The subscription.", body = Data<SubscriptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `plans.billing.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "The tenant has no subscription; code `plans.subscription_not_found`.", body = Problem),
    )
)]
pub(super) async fn own_subscription(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<SubscriptionResponse>, Error> {
    principal.require(&BILLING_READ)?;
    let row = service.subscription(tenant.id()).await?.ok_or_else(subscription_not_found)?;
    Ok(Data { data: row.into() })
}

/// List the subscriptions of every tenant.
///
/// Platform staff with `plans.subscriptions.read`.
#[utoipa::path(
    get,
    path = "/subscriptions",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "list_subscriptions",
    security(("bearer" = [])),
    params(SubscriptionsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of subscriptions, newest first (without add-ons), with the plan's code and name.", body = Page<SubscriptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
    )
)]
pub(super) async fn list_subscriptions(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Query(filter): Query<SubscriptionsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<SubscriptionResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    let filter = SubscriptionFilter {
        status: filter.status,
        plan_id: filter.plan_id,
        tenant_id: filter.tenant_id,
    };
    let page = service.list_subscriptions(&filter, &PageRequest::try_from(page)?).await?;
    Ok(page.map(SubscriptionResponse::from))
}

/// Get a tenant's subscription.
///
/// Platform staff with `plans.subscriptions.read`.
#[utoipa::path(
    get,
    path = "/tenants/{tenant_id}/subscription",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "get_tenant_subscription",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant.")),
    responses(
        (status = 200, description = "The subscription.", body = Data<SubscriptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
        (status = 404, description = "The tenant has no subscription; code `plans.subscription_not_found`.", body = Problem),
    )
)]
pub(super) async fn tenant_subscription(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
) -> Result<Data<SubscriptionResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    let row = service
        .subscription(TenantId::from_uuid(tenant_id))
        .await?
        .ok_or_else(subscription_not_found)?;
    Ok(Data { data: row.into() })
}

/// Put a tenant on a plan.
///
/// Platform staff with `plans.subscriptions.manage`. The first time this creates the
/// subscription (with the plan's trial when the tenant never had one); afterwards it moves the
/// tenant to the plan at once. A canceled subscription starts again.
#[utoipa::path(
    put,
    path = "/tenants/{tenant_id}/subscription",
    extensions(("x-permissions" = json!(["plans.subscriptions.manage"]))),
    operation_id = "assign_subscription",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant.")),
    request_body = AssignRequest,
    responses(
        (status = 200, description = "The subscription after the change.", body = Data<SubscriptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such tenant (`tenant_not_found`), plan (`plans.plan_not_found`) or price (`plans.price_not_found`).", body = Problem),
        (status = 409, description = "The plan is not sold (`plans.plan_not_active`) or the subscription is deleted (`plans.invalid_transition`).", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), a plan is of the wrong kind (`plans.addon_not_allowed`), or the tenant is the platform (`plans.platform_exempt`).", body = Problem),
    )
)]
pub(super) async fn assign_subscription(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
    ValidJson(input): ValidJson<AssignRequest>,
) -> Result<Data<SubscriptionResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_MANAGE)?;
    let row = service
        .assign(Actor::from(&principal), TenantId::from_uuid(tenant_id), input.into())
        .await?;
    Ok(Data { data: row.into() })
}
