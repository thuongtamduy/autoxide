//! REST handlers of per-tenant settings: the caller's own tenant, and any tenant for platform staff.
//! Moved out of `settings.rs` unchanged (D-20261118-file-length).

use super::*;

// --- REST -----------------------------------------------------------------------------------

/// The new value of a setting.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = TenantSettingValue)]
pub(crate) struct SettingInput {
    /// 1 to 4000 characters. Secret values are stored encrypted and never returned.
    #[schema(example = "https://shop.example")]
    value: String,
}

/// Which settings to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SettingFilter {
    /// Only settings of this group, e.g. `connection` (connections to partner services: mail
    /// server, AI provider…). Omit for every setting; a group no setting is in gives an empty
    /// list.
    #[param(example = "connection")]
    group: Option<String>,
}

/// List this tenant's settings.
///
/// Requires `kernel.settings.manage`. Every setting the installed modules take per tenant, with
/// whether this tenant has set it; unset ones use the server-wide configuration. Values of
/// secret settings are never returned. `?group=connection` lists only the connections to partner
/// services, named `<module>.connection.<kind>.<field>`. By name (`order=desc` reverses it);
/// paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/settings",
    extensions(("x-permissions" = json!(["kernel.settings.manage"]))),
    operation_id = "list_tenant_settings",
    security(("bearer" = [])),
    params(SettingFilter, crate::pagination::NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of settings (of the group, when one is given), by name.", body = crate::pagination::Page<SettingState>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.settings.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_settings(
    State(service): State<Arc<SettingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<SettingFilter>,
    Query(page): Query<crate::pagination::NaturalOrderPageQuery>,
) -> Result<crate::pagination::Page<SettingState>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let page = crate::pagination::PageRequest::try_from(page)?;
    service.page(&tenant, filter.group.as_deref(), &page).await
}

/// Set one of this tenant's settings.
///
/// Requires `kernel.settings.manage` held at the tenant's root unit (a role of a branch, or an
/// API key bound to a branch, is refused: settings apply to the whole tenant). Creates or replaces
/// the value; sending it again is safe. A secret value is stored encrypted and the answer does
/// not contain it. The former name of a renamed setting is still accepted; the answer carries the
/// current name.
#[utoipa::path(
    put,
    path = "/settings/{name}",
    extensions(("x-permissions" = json!(["kernel.settings.manage"]))),
    operation_id = "set_tenant_setting",
    security(("bearer" = [])),
    params(("name" = String, Path, description = "Name of the setting, `<module>.<key>`, e.g. `kernel.public_url`.")),
    request_body = SettingInput,
    responses(
        (status = 200, description = "The setting as stored (without a secret value).", body = Data<SettingState>),
        (status = 401, description = "Not signed in (code `unauthorized` or the code of the authentication module), or a setting of the `connection` group needs a recent sign-in: code `step_up_required`, confirm with `POST /api/v1/auth/me/reauth` and retry.", body = Problem),
        (status = 403, description = "Missing permission `kernel.settings.manage` (code `permission_denied`), or it is held only below the tenant's root unit (code `org_out_of_scope`).", body = Problem),
        (status = 422, description = "No module takes this setting (code `tenant_setting_unknown`) or the value is empty or too long; see `errors`.", body = Problem),
    )
)]
async fn set_setting(
    State(service): State<Arc<SettingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(name): Path<String>,
    crate::http::ValidJson(input): crate::http::ValidJson<SettingInput>,
) -> Result<Data<SettingState>, Error> {
    principal.require_at_root(&SETTINGS_MANAGE)?;
    Ok(Data { data: service.put(&tenant, &principal, &name, &input.value).await? })
}

/// Remove one of this tenant's settings.
///
/// Requires `kernel.settings.manage` held at the tenant's root unit, like setting one. The
/// server-wide configuration applies again. Removing a setting this tenant never set changes
/// nothing.
#[utoipa::path(
    delete,
    path = "/settings/{name}",
    extensions(("x-permissions" = json!(["kernel.settings.manage"]))),
    operation_id = "delete_tenant_setting",
    security(("bearer" = [])),
    params(("name" = String, Path, description = "Name of the setting.")),
    responses(
        (status = 204, description = "The setting is unset (or never was)."),
        (status = 401, description = "Not signed in (code `unauthorized` or the code of the authentication module), or a setting of the `connection` group needs a recent sign-in: code `step_up_required`, confirm with `POST /api/v1/auth/me/reauth` and retry.", body = Problem),
        (status = 403, description = "Missing permission `kernel.settings.manage` (code `permission_denied`), or it is held only below the tenant's root unit (code `org_out_of_scope`).", body = Problem),
        (status = 422, description = "No module takes this setting; code `tenant_setting_unknown`.", body = Problem),
    )
)]
async fn delete_setting(
    State(service): State<Arc<SettingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(name): Path<String>,
) -> Result<StatusCode, Error> {
    principal.require_at_root(&SETTINGS_MANAGE)?;
    service.delete(&tenant, &principal, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- REST cho nhân sự nền tảng: cấu hình của một khách bất kỳ ------------------------------

/// What the platform staff handlers need: the service and the database registry (find a tenant by ID / slug).
#[derive(Clone)]
pub(crate) struct PlatformSettings {
    service: Arc<SettingsService>,
    registry: DatabaseRegistry,
}

impl PlatformSettings {
    /// Tenant `id_or_slug`. Being moved: a write now would go to the old database → `tenant_moving`.
    async fn tenant(&self, id_or_slug: &str, writing: bool) -> Result<TenantContext, Error> {
        let tenant = match id_or_slug.parse::<uuid::Uuid>() {
            Ok(id) => self.registry.tenant(TenantId::from_uuid(id)).await?,
            Err(_) => self.registry.tenant_by_slug(id_or_slug).await?,
        };
        if writing && tenant.status() == TenantStatus::Moving {
            return Err(super::super::errors::tenant_moving());
        }
        Ok(tenant)
    }
}

/// List a tenant's settings (platform staff).
///
/// Requires `kernel.tenants.read` and an account of the platform tenant. Like
/// `GET /api/v1/kernel/settings`, for the tenant in the path: every setting the installed modules
/// take per tenant and whether that tenant has set it. Values of secret settings are never
/// returned. `?group=connection` lists only the connections to partner services. By name
/// (`order=desc` reverses it); paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/tenants/{id}/settings",
    extensions(("x-permissions" = json!(["kernel.tenants.read"]))),
    operation_id = "list_settings_of_tenant",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id or slug of the tenant."), SettingFilter, crate::pagination::NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of settings (of the group, when one is given), by name.", body = crate::pagination::Page<SettingState>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.read` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_tenant_settings(
    State(platform): State<PlatformSettings>,
    principal: Principal,
    Path(id): Path<String>,
    Query(filter): Query<SettingFilter>,
    Query(page): Query<crate::pagination::NaturalOrderPageQuery>,
) -> Result<crate::pagination::Page<SettingState>, Error> {
    principal.require(&super::super::TENANTS_READ)?;
    super::super::require_platform(&principal)?;
    let page = crate::pagination::PageRequest::try_from(page)?;
    let tenant = platform.tenant(&id, false).await?;
    platform.service.page(&tenant, filter.group.as_deref(), &page).await
}

/// Set a setting of a tenant (platform staff).
///
/// Requires `kernel.tenants.manage` and an account of the platform tenant. Like
/// `PUT /api/v1/kernel/settings/{name}`, for the tenant in the path: creates or replaces the
/// value; sending it again is safe. The tenant's servers use the new value within
/// `AUTOXIDE_TENANCY_CACHE_SECONDS`.
#[utoipa::path(
    put,
    path = "/tenants/{id}/settings/{name}",
    extensions(("x-permissions" = json!(["kernel.tenants.manage"]))),
    operation_id = "set_setting_of_tenant",
    security(("bearer" = [])),
    params(
        ("id" = String, Path, description = "Id or slug of the tenant."),
        ("name" = String, Path, description = "Name of the setting, `<module>.<key>`, e.g. `comments.max_links`."),
    ),
    request_body = SettingInput,
    responses(
        (status = 200, description = "The setting as stored (without a secret value).", body = Data<SettingState>),
        (status = 401, description = "Not signed in (code `unauthorized` or the code of the authentication module), or a setting of the `connection` group needs a recent sign-in: code `step_up_required`, confirm with `POST /api/v1/auth/me/reauth` and retry.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.manage` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
        (status = 409, description = "The tenant is being moved to another placement; code `tenant_moving`.", body = Problem),
        (status = 422, description = "No module takes this setting (code `tenant_setting_unknown`) or the value is not valid for it; see `errors`.", body = Problem),
    )
)]
async fn set_tenant_setting(
    State(platform): State<PlatformSettings>,
    principal: Principal,
    Path((id, name)): Path<(String, String)>,
    crate::http::ValidJson(input): crate::http::ValidJson<SettingInput>,
) -> Result<Data<SettingState>, Error> {
    principal.require(&super::super::TENANTS_MANAGE)?;
    super::super::require_platform(&principal)?;
    let tenant = platform.tenant(&id, true).await?;
    Ok(Data { data: platform.service.put(&tenant, &principal, &name, &input.value).await? })
}

/// Remove a setting of a tenant (platform staff).
///
/// Requires `kernel.tenants.manage` and an account of the platform tenant. The server-wide
/// configuration applies to that tenant again. Removing a setting the tenant never set changes
/// nothing.
#[utoipa::path(
    delete,
    path = "/tenants/{id}/settings/{name}",
    extensions(("x-permissions" = json!(["kernel.tenants.manage"]))),
    operation_id = "delete_setting_of_tenant",
    security(("bearer" = [])),
    params(
        ("id" = String, Path, description = "Id or slug of the tenant."),
        ("name" = String, Path, description = "Name of the setting."),
    ),
    responses(
        (status = 204, description = "The setting is unset (or never was)."),
        (status = 401, description = "Not signed in (code `unauthorized` or the code of the authentication module), or a setting of the `connection` group needs a recent sign-in: code `step_up_required`, confirm with `POST /api/v1/auth/me/reauth` and retry.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.manage` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
        (status = 409, description = "The tenant is being moved to another placement; code `tenant_moving`.", body = Problem),
        (status = 422, description = "No module takes this setting; code `tenant_setting_unknown`.", body = Problem),
    )
)]
async fn delete_tenant_setting(
    State(platform): State<PlatformSettings>,
    principal: Principal,
    Path((id, name)): Path<(String, String)>,
) -> Result<StatusCode, Error> {
    principal.require(&super::super::TENANTS_MANAGE)?;
    super::super::require_platform(&principal)?;
    let tenant = platform.tenant(&id, true).await?;
    platform.service.delete(&tenant, &principal, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Routes under `/api/v1/kernel`.
pub(crate) fn routes(service: Arc<SettingsService>, registry: DatabaseRegistry) -> OpenApiRouter {
    let platform = PlatformSettings { service: service.clone(), registry };
    OpenApiRouter::new()
        .routes(routes!(list_settings))
        .routes(routes!(set_setting, delete_setting))
        .with_state(service)
        .merge(
            OpenApiRouter::new()
                .routes(routes!(list_tenant_settings))
                .routes(routes!(set_tenant_setting, delete_tenant_setting))
                .with_state(platform),
        )
}
