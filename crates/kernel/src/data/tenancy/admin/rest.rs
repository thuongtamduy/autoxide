//! REST handlers and MCP tools of tenant management: they call exactly the `TenantAdmin` functions.
//! Moved out of `admin.rs` unchanged (D-20261118-file-length).

use super::*;

// --- REST -----------------------------------------------------------------------------------

/// Get my tenant.
///
/// Requires signing in, no permission. The tenant the request is for — the one of the caller, or
/// the one entered with `Autoxide-Tenant` by platform staff — and the mode of the installation.
#[utoipa::path(
    get,
    path = "/tenant",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_current_tenant",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The tenant of the request.", body = Data<CurrentTenant>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
    )
)]
async fn get_current_tenant(
    State(admin): State<Arc<TenantAdmin>>,
    tenant: TenantContext,
    _principal: Principal,
) -> Data<CurrentTenant> {
    Data { data: admin.current(&tenant) }
}

/// List the tenants of the platform.
///
/// Requires `kernel.tenants.read` and an account of the platform tenant (not one entered with
/// `Autoxide-Tenant`). Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/tenants",
    extensions(("x-permissions" = json!(["kernel.tenants.read"]))),
    operation_id = "list_tenants",
    security(("bearer" = [])),
    params(PageQuery, TenantFilter),
    responses(
        (status = 200, description = "One page of tenants.", body = Page<Tenant>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.read` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_tenants(
    State(admin): State<Arc<TenantAdmin>>,
    principal: Principal,
    Query(page): Query<PageQuery>,
    Query(filter): Query<TenantFilter>,
) -> Result<Page<Tenant>, Error> {
    principal.require(&TENANTS_READ)?;
    require_platform(&principal)?;
    admin.list(&filter, &PageRequest::try_from(page)?).await
}

/// Create a tenant.
///
/// Requires `kernel.tenants.manage` and an account of the platform tenant. Creates the tenant and
/// prepares it (every module sets up what a new tenant needs) before answering. Retry safely with
/// an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/tenants",
    extensions(("x-permissions" = json!(["kernel.tenants.manage"]))),
    operation_id = "create_tenant",
    security(("bearer" = [])),
    request_body = NewTenant,
    responses(
        (status = 201, description = "The tenant, ready to use.", body = Data<Tenant>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.manage` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such placement or parent; code `tenant_placement_not_found` or `tenant_not_found`.", body = Problem),
        (status = 409, description = "The slug is taken (`tenant_slug_taken`), the installation runs one tenant (`tenant_single_mode`), or the placement has migrations to apply (`tenant_placement_behind`).", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn create_tenant(
    State(admin): State<Arc<TenantAdmin>>,
    principal: Principal,
    crate::http::ValidJson(input): crate::http::ValidJson<NewTenant>,
) -> Result<Created<Tenant>, Error> {
    principal.require(&TENANTS_MANAGE)?;
    require_platform(&principal)?;
    let tenant = admin.create(Actor::from(&principal), input).await?;
    Ok(Created::new(format!("/kernel/tenants/{}", tenant.id), tenant))
}

/// Get a tenant.
///
/// Requires `kernel.tenants.read` and an account of the platform tenant.
#[utoipa::path(
    get,
    path = "/tenants/{id}",
    extensions(("x-permissions" = json!(["kernel.tenants.read"]))),
    operation_id = "get_tenant",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant.")),
    responses(
        (status = 200, description = "The tenant.", body = Data<Tenant>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.read` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
    )
)]
async fn get_tenant(
    State(admin): State<Arc<TenantAdmin>>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Tagged<Tenant>, Error> {
    principal.require(&TENANTS_READ)?;
    require_platform(&principal)?;
    let tenant = admin.get(&id).await?;
    Ok(Tagged { version: tenant.updated_at, data: tenant })
}

/// Rename, suspend or resume a tenant.
///
/// Requires `kernel.tenants.manage` and an account of the platform tenant. A suspended tenant
/// answers every request with `403 tenant_suspended` (other servers see the change within
/// `AUTOXIDE_TENANCY_CACHE_SECONDS`). The platform tenant cannot be suspended.
#[utoipa::path(
    patch,
    path = "/tenants/{id}",
    extensions(("x-permissions" = json!(["kernel.tenants.manage"]))),
    operation_id = "update_tenant",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the change is made only if nobody changed the record since; otherwise `412`.")),
    request_body = TenantChanges,
    responses(
        (status = 200, description = "The tenant after the change.", body = Data<Tenant>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.manage` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the record changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem is listed in `errors` (code `validation_failed`). Making the platform tenant read-only or suspending it is refused this way, on the field `read_only` or `status`.", body = Problem),
    )
)]
async fn update_tenant(
    State(admin): State<Arc<TenantAdmin>>,
    principal: Principal,
    Path(id): Path<String>,
    if_match: IfMatch,
    crate::http::ValidJson(changes): crate::http::ValidJson<TenantChanges>,
) -> Result<Tagged<Tenant>, Error> {
    principal.require(&TENANTS_MANAGE)?;
    require_platform(&principal)?;
    let expected = if_match.expected()?;
    // ID hay slug, như `GET /tenants/{id}`, `…/settings` và `…/move`.
    let id = admin.get(&id).await?.id;
    let tenant = admin.update(Actor::from(&principal), id, changes, expected).await?;
    Ok(Tagged { version: tenant.updated_at, data: tenant })
}

/// Routes under `/api/v1/kernel`.
pub(crate) fn routes(admin: Arc<TenantAdmin>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(get_current_tenant))
        .routes(routes!(list_tenants, create_tenant))
        .routes(routes!(get_tenant, update_tenant))
        .routes(routes!(placement_list::list_placements))
        .merge(super::super::erasure::routes())
        .with_state(admin)
}

// Đầu vào của tool `get_tenant`.
/// Which tenant to get.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TenantRef {
    /// Id or slug of the tenant.
    #[validate(length(min = 1, max = 63))]
    tenant: String,
}

/// MCP tools: call exactly the functions REST uses.
pub(crate) fn tools(admin: Arc<TenantAdmin>) -> Vec<Tool> {
    let (current, list, get) = (admin.clone(), admin.clone(), admin);
    vec![
        Tool::new(
            "current_tenant",
            "Tell which tenant (customer) this request is for, and whether the installation runs \
             one tenant or many.",
            move |tenant: TenantContext, _principal: Principal, _: NoInput| {
                let current = current.clone();
                async move { Ok::<_, Error>(current.current(&tenant)) }
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "list_tenants",
            "List the tenants of the platform, newest first (platform staff only).",
            move |_tenant: TenantContext, principal: Principal, input: crate::mcp::PageInput| {
                let list = list.clone();
                async move {
                    require_platform(&principal)?;
                    list.list(&TenantFilter::default(), &input.page()?).await
                }
            },
        )
        .requires(TENANTS_READ)
        .read_only(),
        Tool::new(
            "get_tenant",
            "Get one tenant of the platform by id or slug (platform staff only).",
            move |_tenant: TenantContext, principal: Principal, input: TenantRef| {
                let get = get.clone();
                async move {
                    require_platform(&principal)?;
                    get.get(&input.tenant).await
                }
            },
        )
        .requires(TENANTS_READ)
        .read_only(),
    ]
}
