//! REST of the sandbox (D-232), under `/api/v1/kernel`. A handler only: takes input → calls the service →
//! returns the envelope; the service checks permission and scope.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::mailbox::SandboxMessage;
use super::promote::{PromotableKind, PromoteInput, PromoteResult};
use super::service::{NewSandbox, Sandbox, SandboxCreated, TenantSandboxes};
use crate::access::Principal;
use crate::error::Error;
use crate::http::ValidJson;
use crate::pagination::{Page, PageQuery, PageRequest};
use crate::response::{Created, Data, Problem};
use crate::tenancy::TenantContext;

pub(crate) fn routes(service: Arc<TenantSandboxes>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_sandboxes, create_sandbox))
        .routes(routes!(extend_sandbox))
        .routes(routes!(delete_sandbox))
        .routes(routes!(list_sandbox_messages))
        .routes(routes!(list_promotable))
        .routes(routes!(promote_sandbox))
        .with_state(service)
}

/// List the sandboxes of a tenant.
///
/// Requires `kernel.sandboxes.read`. `{id}` is your own tenant (platform staff with
/// `kernel.tenants.manage` may name any tenant). Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/tenants/{id}/sandboxes",
    extensions(("x-permissions" = json!(["kernel.sandboxes.read"]))),
    operation_id = "list_tenant_sandboxes",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the real tenant."), PageQuery),
    responses(
        (status = 200, description = "One page of sandboxes.", body = Page<Sandbox>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.sandboxes.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "Not your tenant; code `tenant_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; code `validation_failed`.", body = Problem),
    )
)]
async fn list_sandboxes(
    State(service): State<Arc<TenantSandboxes>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<Sandbox>, Error> {
    service.list(&tenant, &principal, id, &PageRequest::try_from(page)?).await
}

/// Create a sandbox of a tenant.
///
/// Requires `kernel.sandboxes.manage`, called inside the tenant `{id}` (platform staff enter it
/// first under a support access grant in force). A sandbox is a test copy with the configuration of the
/// tenant and no personal data (no users, buyers, orders, ledger, documents, notifications,
/// secrets or domains). Partners are always simulated in it, mail only reaches staff who may enter
/// it, and every response carries `Autoxide-Sandbox: true` and `X-Robots-Tag: noindex, nofollow`.
/// It is `provisioning` until the copy job (`job_id`) is done, then holders of `role_id` (the
/// `admin` role by default) enter it with the header `Autoxide-Tenant: <sandbox id>`. It is
/// deleted after `AUTOXIDE_TENANCY_SANDBOX_DAYS` days unless extended. Send an `Idempotency-Key`
/// to retry safely.
#[utoipa::path(
    post,
    path = "/tenants/{id}/sandboxes",
    extensions(("x-permissions" = json!(["kernel.sandboxes.manage"]))),
    operation_id = "create_tenant_sandbox",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the real tenant.")),
    request_body = NewSandbox,
    responses(
        (status = 201, description = "Created; the configuration is being copied.", body = Data<SandboxCreated>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.sandboxes.manage` (`permission_denied`), the tenant already keeps as many sandboxes as its plan allows (`limit_exceeded`, 1 when the plan does not say), the platform tenant (`tenancy.sandbox_forbidden`), or platform staff naming a tenant they have not entered (`tenancy.sandbox_needs_tenant_access`).", body = Problem),
        (status = 404, description = "Not your tenant; code `tenant_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid (`validation_failed`, see `errors`), or the tenant is itself a sandbox (`tenancy.sandbox_nested`).", body = Problem),
    )
)]
async fn create_sandbox(
    State(service): State<Arc<TenantSandboxes>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<NewSandbox>,
) -> Result<Created<SandboxCreated>, Error> {
    let created = service.create(&tenant, &principal, id, input).await?;
    let location = format!("/kernel/tenants/{id}/sandboxes");
    Ok(Created::new(location, created))
}

/// Extend a sandbox.
///
/// Requires `kernel.sandboxes.manage`, in the real tenant of the sandbox. It is then deleted
/// `AUTOXIDE_TENANCY_SANDBOX_DAYS` days from now.
#[utoipa::path(
    post,
    path = "/sandboxes/{id}/extend",
    extensions(("x-permissions" = json!(["kernel.sandboxes.manage"]))),
    operation_id = "extend_sandbox",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the sandbox.")),
    responses(
        (status = 200, description = "The sandbox with its new expiry date.", body = Data<Sandbox>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.sandboxes.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such sandbox of your tenant; code `tenancy.sandbox_not_found`.", body = Problem),
    )
)]
async fn extend_sandbox(
    State(service): State<Arc<TenantSandboxes>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Sandbox>, Error> {
    Ok(Data { data: service.extend(&tenant, &principal, id).await? })
}

/// Delete a sandbox.
///
/// Requires `kernel.sandboxes.manage`, in the real tenant of the sandbox. Everything tried in it is
/// deleted now and cannot be recovered; promote what you want to keep first.
#[utoipa::path(
    delete,
    path = "/sandboxes/{id}",
    extensions(("x-permissions" = json!(["kernel.sandboxes.manage"]))),
    operation_id = "delete_sandbox",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the sandbox.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.sandboxes.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such sandbox of your tenant; code `tenancy.sandbox_not_found`.", body = Problem),
    )
)]
async fn delete_sandbox(
    State(service): State<Arc<TenantSandboxes>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Filter of the sandbox message box.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct MessageFilter {
    /// Only this channel: `partner`, `sms`, `zns`, `push`, `mail` or `webhook`.
    channel: Option<String>,
}

/// List what this sandbox kept instead of sending out.
///
/// Requires `kernel.sandboxes.read`, called inside a sandbox (`Autoxide-Tenant: <sandbox id>`).
/// Partners are always simulated in a sandbox: payments succeed, e-invoices get numbers starting
/// with `SANDBOX-`, identity checks pass, and every such call is listed here with SMS, Zalo
/// messages and push notifications. Mail to anyone but staff who may enter the sandbox, and
/// webhooks to URLs not listed in the tenant setting `kernel.sandbox.webhook_urls`, are listed
/// here too instead of being sent. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/sandbox/messages",
    extensions(("x-permissions" = json!(["kernel.sandboxes.read"]))),
    operation_id = "list_sandbox_messages",
    security(("bearer" = [])),
    params(MessageFilter, PageQuery),
    responses(
        (status = 200, description = "One page of kept messages.", body = Page<SandboxMessage>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.sandboxes.read`; code `permission_denied`.", body = Problem),
        (status = 409, description = "The current tenant is not a sandbox; code `tenancy.sandbox_only`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; code `validation_failed`.", body = Problem),
    )
)]
async fn list_sandbox_messages(
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<MessageFilter>,
    Query(page): Query<PageQuery>,
) -> Result<Page<SandboxMessage>, Error> {
    let page = PageRequest::try_from(page)?;
    super::mailbox::list(&tenant, &principal, filter.channel.as_deref(), &page).await
}

/// List the kinds of records that can be promoted from a sandbox.
///
/// Requires `kernel.sandboxes.read`. Each module declares its own kinds (pages, mail templates…).
#[utoipa::path(
    get,
    path = "/sandboxes/promotable",
    extensions(("x-permissions" = json!(["kernel.sandboxes.read"]))),
    operation_id = "list_sandbox_promotable",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every kind that can be promoted.", body = Data<Vec<PromotableKind>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.sandboxes.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_promotable(
    State(service): State<Arc<TenantSandboxes>>,
    principal: Principal,
) -> Result<Data<Vec<PromotableKind>>, Error> {
    Ok(Data { data: service.promotable(&principal)? })
}

/// Promote records tried in a sandbox to the real tenant, as drafts.
///
/// Requires `kernel.sandboxes.manage`, called in the real tenant of the sandbox, plus the
/// permission each module needs to write the record (e.g. `content.write` for a page): you cannot
/// promote what you could not write by hand. Up to 100 records. Each one becomes a **draft** in the
/// real tenant — nothing is published or enabled, and what is live is never overwritten. A record
/// copied from the real tenant updates the draft of its original; a record made in the sandbox
/// becomes a new draft. Promoting again with the same content gives `unchanged`. A name, path or
/// code already taken in the real tenant gives `conflict` for that record while the others go on.
/// Send an `Idempotency-Key` to retry safely.
#[utoipa::path(
    post,
    path = "/sandboxes/{id}/promote",
    extensions(("x-permissions" = json!(["kernel.sandboxes.manage"]))),
    operation_id = "promote_sandbox",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the sandbox.")),
    request_body = PromoteInput,
    responses(
        (status = 200, description = "One outcome per record, in the order sent.", body = Data<PromoteResult>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.sandboxes.manage` or the permission of a module (`permission_denied`), or called outside the real tenant of the sandbox (`tenancy.sandbox_forbidden`).", body = Problem),
        (status = 404, description = "No such sandbox of your tenant (`tenancy.sandbox_not_found`), or no such record in it (`tenancy.promote_record_not_found`).", body = Problem),
        (status = 422, description = "No records, more than 100, or a kind that cannot be promoted (`validation_failed`, see `errors`: `tenancy.promote_kind_unknown`).", body = Problem),
    )
)]
async fn promote_sandbox(
    State(service): State<Arc<TenantSandboxes>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<PromoteInput>,
) -> Result<Data<PromoteResult>, Error> {
    Ok(Data { data: service.promote(&tenant, &principal, id, input).await? })
}
