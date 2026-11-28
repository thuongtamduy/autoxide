//! REST handlers and MCP tools of tenant data exports: they call exactly the service functions.
//! Moved out of `exports.rs` unchanged (D-20261118-file-length).

use super::*;

// ---- REST ----------------------------------------------------------------------------------------

/// The tenant of the `{id}` path (ID or slug) that the caller may export (D-313, D-20261121-export-entry):
///
/// - the tenant's own administrator, who has `kernel.data.export`;
/// - platform staff who have ENTERED that tenant (header `Autoxide-Tenant`, through a support access grant
///   still valid; grounds `support` or `legal_request` with a document number) and have
///   `kernel.tenants.operate` or `kernel.data.export`;
/// - platform staff in the platform tenant can export only the platform tenant itself. Targeting another
///   tenant without entering it gives 403 `kernel.export_needs_tenant_access`; there is no shortcut where
///   "`kernel.tenants.operate` exports every tenant" (a tenant's personal data leaves only with grounds and
///   leaves a trace).
async fn allowed(
    exports: &TenantExports,
    principal: &Principal,
    id: &str,
) -> Result<TenantId, Error> {
    let operate = principal.has(&crate::builtin::TENANTS_OPERATE);
    // Ở khách nền tảng, chưa vào khách nào.
    let platform_staff = operate && super::super::require_platform(principal).is_ok();
    // Nhân sự nền tảng đang ở trong khách khác (đường vào đã do bộ xác thực quyết ở request.rs).
    let entered_staff = operate && principal.operator_from() == Some(super::super::DEFAULT_TENANT);
    // Không có permission nào thì báo 403 trước khi tra khách, như mọi endpoint khác.
    if !platform_staff && !entered_staff {
        principal.require(&DATA_EXPORT)?;
    }
    let tenant = match Uuid::parse_str(id) {
        Ok(uuid) => exports.registry.tenant(TenantId::from_uuid(uuid)).await?,
        Err(_) => exports.registry.tenant_by_slug(id).await?,
    };
    let tenant = tenant.id();
    if principal.tenant_id() == tenant {
        return Ok(tenant);
    }
    if platform_staff {
        return Err(export_needs_tenant_access(tenant));
    }
    // Khách khác: như không có (không lộ khách nào tồn tại).
    Err(errors::tenant_not_found(id))
}

fn export_needs_tenant_access(tenant: TenantId) -> Error {
    Error::forbidden(
        "kernel.export_needs_tenant_access",
        "Exporting the data of another tenant needs access to that tenant: get a support access \
         grant in force for it, then call with the header `Autoxide-Tenant: {tenant}`.",
    )
    .with_param("tenant", tenant.as_uuid())
}

pub(crate) fn routes(exports: Arc<TenantExports>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(request_export, list_exports))
        .routes(routes!(get_export))
        .routes(routes!(download_export))
        .routes(routes!(export_link))
        .routes(routes!(download_by_link))
        .with_state(exports)
}

/// Export all the data of a tenant.
///
/// An administrator of the tenant itself with `kernel.data.export`, or platform staff with
/// `kernel.tenants.operate` who entered the tenant under a support access grant in force (header
/// `Autoxide-Tenant`). The request and every download are recorded in the tenant's own audit
/// events (`kernel.tenant_export_requested`, `kernel.tenant_export_accessed`). The export runs in the background into one JSON Lines file (one line per
/// record, `{"table": …, "row": {…}}`; password hashes and secrets left out; uploaded file contents
/// are not included). Poll `GET …/exports/{export_id}` until it is `ready`. If an export is already
/// running it is returned instead of starting another.
#[utoipa::path(
    post,
    path = "/tenants/{id}/exports",
    extensions(("x-permissions" = json!(["kernel.data.export", "kernel.tenants.operate"]))),
    operation_id = "request_tenant_export",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant.")),
    responses(
        (status = 201, description = "The export, `pending`; `Location` is its status.", body = Data<TenantExport>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing both `kernel.data.export` and `kernel.tenants.operate` (`permission_denied`), or platform staff targeting another tenant without having entered it under a support access grant (`kernel.export_needs_tenant_access`).", body = Problem),
        (status = 404, description = "No such tenant you may export; code `tenant_not_found`.", body = Problem),
    )
)]
async fn request_export(
    State(exports): State<Arc<TenantExports>>,
    principal: Principal,
    Path(id): Path<String>,
) -> Result<Created<TenantExport>, Error> {
    let tenant = allowed(&exports, &principal, &id).await?;
    let export = exports.request(Actor::from(&principal), tenant, "manual").await?;
    Ok(Created::new(format!("/kernel/tenants/{tenant}/exports/{}", export.id), export))
}

/// List the data exports of a tenant.
///
/// Same permissions as `POST /tenants/{id}/exports`. Newest first unless `order=asc`; paged with
/// `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/tenants/{id}/exports",
    extensions(("x-permissions" = json!(["kernel.data.export", "kernel.tenants.operate"]))),
    operation_id = "list_tenant_exports",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant."), crate::pagination::PageQuery),
    responses(
        (status = 200, description = "One page of exports, newest first.", body = crate::pagination::Page<TenantExport>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing both `kernel.data.export` and `kernel.tenants.operate` (`permission_denied`), or platform staff targeting another tenant without having entered it under a support access grant (`kernel.export_needs_tenant_access`).", body = Problem),
        (status = 404, description = "No such tenant you may export; code `tenant_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_exports(
    State(exports): State<Arc<TenantExports>>,
    principal: Principal,
    Path(id): Path<String>,
    Query(page): Query<crate::pagination::PageQuery>,
) -> Result<crate::pagination::Page<TenantExport>, Error> {
    let tenant = allowed(&exports, &principal, &id).await?;
    let page = crate::pagination::PageRequest::try_from(page)?;
    exports.page(tenant, &page).await
}

/// Get a data export of a tenant.
///
/// Same permissions as `POST /tenants/{id}/exports`.
#[utoipa::path(
    get,
    path = "/tenants/{id}/exports/{export_id}",
    extensions(("x-permissions" = json!(["kernel.data.export", "kernel.tenants.operate"]))),
    operation_id = "get_tenant_export",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant."), ("export_id" = Uuid, Path, description = "Id of the export.")),
    responses(
        (status = 200, description = "The export.", body = Data<TenantExport>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing both `kernel.data.export` and `kernel.tenants.operate` (`permission_denied`), or platform staff targeting another tenant without having entered it under a support access grant (`kernel.export_needs_tenant_access`).", body = Problem),
        (status = 404, description = "No such tenant or export; code `tenant_not_found` or `kernel.export_not_found`.", body = Problem),
    )
)]
async fn get_export(
    State(exports): State<Arc<TenantExports>>,
    principal: Principal,
    Path((id, export_id)): Path<(String, Uuid)>,
) -> Result<Data<TenantExport>, Error> {
    let tenant = allowed(&exports, &principal, &id).await?;
    Ok(Data { data: exports.get(tenant, export_id).await? })
}

/// Download a data export of a tenant.
///
/// Same permissions as `POST /tenants/{id}/exports`. A JSON Lines file: the first line describes
/// the export, then one line per record.
#[utoipa::path(
    get,
    path = "/tenants/{id}/exports/{export_id}/content",
    extensions(("x-permissions" = json!(["kernel.data.export", "kernel.tenants.operate"]))),
    operation_id = "download_tenant_export",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant."), ("export_id" = Uuid, Path, description = "Id of the export.")),
    responses(
        (status = 200, description = "The file.", body = String, content_type = "application/x-ndjson"),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing both `kernel.data.export` and `kernel.tenants.operate` (`permission_denied`), or platform staff targeting another tenant without having entered it under a support access grant (`kernel.export_needs_tenant_access`).", body = Problem),
        (status = 404, description = "No such tenant or export; code `tenant_not_found` or `kernel.export_not_found`.", body = Problem),
        (status = 409, description = "The export is not `ready`; code `kernel.export_not_ready`.", body = Problem),
    )
)]
async fn download_export(
    State(exports): State<Arc<TenantExports>>,
    principal: Principal,
    Path((id, export_id)): Path<(String, Uuid)>,
) -> Result<Response, Error> {
    let tenant = allowed(&exports, &principal, &id).await?;
    exports.content(Actor::from(&principal), tenant, export_id).await
}

/// Create a download link for a data export.
///
/// Same permissions as `POST /tenants/{id}/exports`. The link works without signing in for 24
/// hours (to open in a browser); a new link replaces the previous one.
#[utoipa::path(
    post,
    path = "/tenants/{id}/exports/{export_id}/link",
    extensions(("x-permissions" = json!(["kernel.data.export", "kernel.tenants.operate"]))),
    operation_id = "create_tenant_export_link",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant."), ("export_id" = Uuid, Path, description = "Id of the export.")),
    responses(
        (status = 200, description = "The link.", body = Data<TenantExportLink>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing both `kernel.data.export` and `kernel.tenants.operate` (`permission_denied`), or platform staff targeting another tenant without having entered it under a support access grant (`kernel.export_needs_tenant_access`).", body = Problem),
        (status = 404, description = "No such tenant or export; code `tenant_not_found` or `kernel.export_not_found`.", body = Problem),
        (status = 409, description = "The export is not `ready`; code `kernel.export_not_ready`.", body = Problem),
    )
)]
async fn export_link(
    State(exports): State<Arc<TenantExports>>,
    principal: Principal,
    Path((id, export_id)): Path<(String, Uuid)>,
) -> Result<Data<TenantExportLink>, Error> {
    let tenant = allowed(&exports, &principal, &id).await?;
    Ok(Data { data: exports.link(Actor::from(&principal), tenant, export_id).await? })
}

/// The download link of a data export.
#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
struct LinkQuery {
    /// The token from `POST /tenants/{id}/exports/{export_id}/link`.
    #[validate(length(min = 1, max = 200))]
    token: String,
}

/// Download a data export with a link.
///
/// No sign-in: the token in the link is the permission. Wrong, replaced and expired links answer
/// the same 404.
#[utoipa::path(
    get,
    path = "/exports/download",
    extensions(("x-public" = json!(true))),
    operation_id = "download_tenant_export_by_link",
    params(LinkQuery),
    responses(
        (status = 200, description = "The file.", body = String, content_type = "application/x-ndjson"),
        (status = 404, description = "The link is wrong, replaced or expired; code `kernel.export_link_invalid`.", body = Problem),
        (status = 409, description = "The export was deleted after its retention days; code `kernel.export_not_ready`.", body = Problem),
    )
)]
async fn download_by_link(
    State(exports): State<Arc<TenantExports>>,
    origin: Option<axum::Extension<crate::http::OriginGrant>>,
    Query(query): Query<LinkQuery>,
) -> Result<Response, Error> {
    let origin = match origin {
        Some(axum::Extension(crate::http::OriginGrant::Tenant(owner))) => Some(owner),
        _ => None,
    };
    exports.content_by_token_for(&query.token, origin).await
}

// ---- MCP -------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportTenantInput {
    /// Id or slug of the tenant; your own tenant when left out.
    tenant: Option<String>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportRef {
    /// Id or slug of the tenant; your own tenant when left out.
    tenant: Option<String>,
    /// Id of the export.
    export_id: Uuid,
}

pub(crate) fn tools(exports: Arc<TenantExports>) -> Vec<crate::mcp::Tool> {
    use crate::mcp::Tool;
    let (start, get) = (exports.clone(), exports);
    let own = |principal: &Principal, tenant: Option<String>| {
        tenant.unwrap_or_else(|| principal.tenant_id().to_string())
    };
    vec![
        Tool::new(
            "export_tenant",
            "Start an export of all the data of a tenant (yours by default) into one JSON Lines file, \
             in the background; follow it with `kernel.get_tenant_export` until it is `ready`, \
             then download it over REST.",
            move |_tenant: TenantContext, principal: Principal, input: ExportTenantInput| {
                let exports = start.clone();
                async move {
                    let id = own(&principal, input.tenant);
                    let tenant = allowed(&exports, &principal, &id).await?;
                    exports.request(Actor::from(&principal), tenant, "manual").await
                }
            },
        )
        .requires(DATA_EXPORT),
        Tool::new(
            "get_tenant_export",
            "Get a data export of a tenant: its `status` (`pending`, `running`, `ready`, `failed`, \
             `expired`), rows per table and size.",
            move |_tenant: TenantContext, principal: Principal, input: ExportRef| {
                let exports = get.clone();
                async move {
                    let id = own(&principal, input.tenant);
                    let tenant = allowed(&exports, &principal, &id).await?;
                    exports.get(tenant, input.export_id).await
                }
            },
        )
        .requires(DATA_EXPORT)
        .read_only(),
    ]
}
