//! Quyền dùng và mức đã dùng: của khách đang gọi và của một khách bất kỳ (nhân sự vận hành).

use super::*;

pub(crate) async fn describe_entitlements(
    service: &PlansService,
    tenant: TenantId,
) -> Result<EntitlementsResponse, Error> {
    let snapshot = service.snapshot(tenant).await?;
    let limits = limit_uses(service, tenant).await?;
    let catalog = service.entitlements.catalog();
    let modules = catalog
        .modules()
        .iter()
        .filter(|module| catalog.is_always_enabled(module) || snapshot.has_module(module))
        .map(|module| (*module).to_owned())
        .collect();
    let features = if snapshot.unrestricted {
        catalog.features().map(|found| found.feature.key().to_owned()).collect()
    } else {
        snapshot.features.iter().cloned().collect()
    };
    Ok(EntitlementsResponse {
        unrestricted: snapshot.unrestricted,
        access: match snapshot.access {
            TenantAccess::Full => "full",
            TenantAccess::ReadOnly => "read_only",
        }
        .to_owned(),
        modules,
        features,
        limits,
    })
}

pub(crate) async fn limit_uses(
    service: &PlansService,
    tenant: TenantId,
) -> Result<Vec<LimitUse>, Error> {
    let snapshot = service.snapshot(tenant).await?;
    let used = service.usage(tenant, now()).await?;
    Ok(limit_uses_of(service, &snapshot, &used))
}

/// Một trang lượng dùng của khách `tenant`, theo khoá hạn mức. Không nằm trong database: dựng từ sổ
/// hạn mức module khai (trong code) cộng phần riêng theo phần của gói — số dòng theo cấu trúc sổ hạn mức,
/// nên phân trang trong bộ nhớ (`page_of`, D-20261125-list-pages mục 4).
pub(crate) async fn limit_use_page(
    service: &PlansService,
    tenant: TenantId,
    page: &PageRequest,
) -> Result<Page<LimitUse>, Error> {
    let uses = limit_uses(service, tenant).await?;
    page_of(uses, |row| row.key.clone(), page)
}

/// Lượng dùng từng hạn mức, từ quyền dùng và lượng đã đọc sẵn (dùng chung cho một khách và cho cả
/// trang khách của `expand=usage`).
pub(crate) fn limit_uses_of(
    service: &PlansService,
    snapshot: &autoxide_kernel::entitlement::TenantEntitlements,
    used: &crate::usage::Usage,
) -> Vec<LimitUse> {
    let mut uses = Vec::new();
    for found in service.entitlements.catalog().limits() {
        let key = found.limit.key();
        let kind = found.limit.kind().as_str();
        let max = snapshot.limit(key);
        uses.push(limit_use(key, kind, max, used.get(key).copied().unwrap_or(0)));
        if found.limit.parts().is_none() {
            continue;
        }
        // Mức riêng theo phần (vd theo loại nút): phần gói đặt mức, và phần đang có lượng dùng.
        let prefix = format!("{key}.");
        let parts: std::collections::BTreeSet<&String> = snapshot
            .limits
            .keys()
            .chain(used.keys())
            .filter(|part| part.starts_with(&prefix))
            .collect();
        for part in parts {
            let max = snapshot.part_limit(part);
            uses.push(limit_use(part, kind, max, used.get(part).copied().unwrap_or(0)));
        }
    }
    uses
}

pub(super) fn limit_use(key: &str, kind: &str, max: Option<i64>, used: i64) -> LimitUse {
    LimitUse {
        key: key.to_owned(),
        kind: kind.to_owned(),
        max,
        used,
        over_limit: max.is_some_and(|max| used > max),
    }
}

/// Get what your tenant may use now.
///
/// Any signed-in user. Modules, features, limits with their use, and whether the data is
/// read-only. User interfaces use it to hide what the plan does not include.
#[utoipa::path(
    get,
    path = "/entitlements",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_entitlements",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "What the tenant may use now.", body = Data<EntitlementsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
pub(super) async fn entitlements(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    _principal: Principal,
) -> Result<Data<EntitlementsResponse>, Error> {
    Ok(Data { data: describe_entitlements(&service, tenant.id()).await? })
}

/// Get your tenant's use of every limit.
///
/// Requires `plans.billing.read`.
#[utoipa::path(
    get,
    path = "/usage",
    extensions(("x-permissions" = json!(["plans.billing.read"]))),
    operation_id = "get_own_usage",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the use of each limit (metered limits: this month), by limit key.", body = Page<LimitUse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `plans.billing.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(super) async fn own_usage(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<LimitUse>, Error> {
    principal.require(&BILLING_READ)?;
    let page = PageRequest::try_from(page)?;
    limit_use_page(&service, tenant.id(), &page).await
}

/// Get a tenant's use of every limit.
///
/// Platform staff with `plans.subscriptions.read`.
#[utoipa::path(
    get,
    path = "/tenants/{tenant_id}/usage",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "get_tenant_usage",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant."), NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the use of each limit (metered limits: this month), by limit key.", body = Page<LimitUse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(super) async fn tenant_usage(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<LimitUse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    let page = PageRequest::try_from(page)?;
    limit_use_page(&service, TenantId::from_uuid(tenant_id), &page).await
}

/// Get what a tenant may use now.
///
/// Platform staff with `plans.subscriptions.read`. The same as the tenant sees with
/// `GET /entitlements`: modules, features, limits with their use, and whether its data is read-only.
#[utoipa::path(
    get,
    path = "/tenants/{tenant_id}/entitlements",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "get_tenant_entitlements",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant.")),
    responses(
        (status = 200, description = "What the tenant may use now.", body = Data<EntitlementsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
    )
)]
pub(super) async fn tenant_entitlements(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
) -> Result<Data<EntitlementsResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    let tenant = TenantId::from_uuid(tenant_id);
    // Khách phải có thật (404 của kernel), không trả "không giới hạn" cho ID bịa.
    service.databases.tenant(tenant).await?;
    Ok(Data { data: describe_entitlements(&service, tenant).await? })
}
