//! REST for splitting / merging tenants (D-210): platform staff run it (`/api/v1/kernel/relocations`, dry
//! run by default, a real run is done by a job); a tenant only submits a request
//! (`/api/v1/kernel/tenant/relocation-requests`), and the request goes into the platform staff's work inbox
//! ([`RelocationRequestsWork`]). There is no MCP tool: it is operator work that cannot be undone with a
//! single command.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::{
    RELOCATIONS_REQUEST, RelocationInput, RelocationKind, RelocationPlan, TENANTS_RELOCATE,
    TenantRelocation, TenantRelocations,
};
use crate::access::{OrgTarget, Permission, Principal};
use crate::error::Error;
use crate::event::Actor;
use crate::i18n::Language;
use crate::response::{Created, Data, Problem};
use crate::tenancy::{DEFAULT_TENANT, TENANTS_READ, TenantContext, require_platform};
use crate::work::{Severity, WorkItem, WorkSource};

fn yes() -> bool {
    true
}

/// A split or merge of tenants to run (platform staff).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RelocationRequest {
    /// `split`: a branch of `tenant` becomes a tenant of its own. `merge`: `tenant` joins `into`.
    kind: RelocationKind,
    /// Id or slug of the tenant whose rows go (split: the tenant with the branch; merge: the
    /// tenant that is merged).
    #[validate(length(min = 1, max = 63))]
    tenant: String,
    /// Split: the root unit of the branch. The new tenant gets this id.
    org_id: Option<Uuid>,
    /// Merge: id or slug of the tenant that receives the rows.
    #[validate(length(min = 1, max = 63))]
    into: Option<String>,
    /// Merge: the unit of `into` that the root of the merged tenant goes under.
    under_org: Option<Uuid>,
    /// Split: slug of the new tenant.
    #[validate(length(min = 1, max = 63))]
    slug: Option<String>,
    /// Split: name of the new tenant.
    #[validate(length(min = 1, max = 200))]
    name: Option<String>,
    /// Split: placement (database) of the new tenant; it must be another database than the
    /// source tenant's.
    #[validate(length(min = 1, max = 63))]
    placement: Option<String>,
    /// Split: the new tenant is a child of the source tenant. Defaults to `true`.
    #[serde(default = "yes")]
    #[schema(default = true)]
    keep_parent: bool,
    /// Why the data moves between tenants (contract, consent); kept with the run and in the audit
    /// log.
    #[validate(length(min = 1, max = 500))]
    reason: String,
    /// Carry out this request of a tenant (from `POST /tenant/relocation-requests`); the other
    /// fields still apply.
    request_id: Option<Uuid>,
    /// Only report what would happen, changing nothing. Defaults to `true`: send `false` to run.
    #[serde(default = "yes")]
    #[schema(default = true)]
    dry_run: bool,
}

/// Split a tenant or merge tenants.
///
/// Requires `kernel.tenants.relocate` and an account of the platform tenant. With `dry_run` (the
/// default) nothing changes: the answer lists what blocks, the conflicts (rows pointing across the
/// boundary, unique values the target already has) and the rows of every table. With
/// `"dry_run": false` the tenants answer `503 tenant_maintenance` while a background job copies the
/// rows (keeping their ids), checks them, switches the tenants and cleans the source; follow it at
/// the `Location` of the answer. Source and target must be two databases.
#[utoipa::path(
    post,
    path = "/relocations",
    extensions(("x-permissions" = json!(["kernel.tenants.relocate"]))),
    operation_id = "relocate_tenant",
    security(("bearer" = [])),
    request_body = RelocationRequest,
    responses(
        (status = 200, description = "Dry run: what would happen; nothing changed.", body = Data<RelocationPlan>),
        (status = 201, description = "The run started in the background; `Location` is its status.", body = Data<TenantRelocation>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.relocate` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant, placement or request; code `tenant_not_found`, `tenant_placement_not_found` or `tenancy.relocation_not_found`.", body = Problem),
        (status = 409, description = "It cannot run now; code `tenancy.relocation_conflict` (see the plan of the run), `tenancy.relocation_in_progress`, `tenancy.relocation_not_active`, `tenancy.relocation_org_invalid`, `tenancy.relocation_same_database`, `tenancy.relocation_tenant_exists`, `tenancy.relocation_platform`, `tenancy.relocation_cycle`, `tenancy.relocation_schema_differs`, `tenant_slug_taken`, `tenant_move_modules_off` or `tenant_placement_behind`. A dry run lists every reason.", body = Problem),
        (status = 422, description = "A value is invalid or missing; see `errors`.", body = Problem),
        (status = 503, description = "A database cannot be reached; code `tenant_move_unreachable`.", body = Problem),
    )
)]
async fn relocate_tenant(
    State(service): State<Arc<TenantRelocations>>,
    principal: Principal,
    language: Language,
    crate::http::ValidJson(input): crate::http::ValidJson<RelocationRequest>,
) -> Result<Response, Error> {
    principal.require(&TENANTS_RELOCATE)?;
    require_platform(&principal)?;
    let relocation = RelocationInput {
        kind: input.kind,
        tenant: input.tenant,
        org_id: input.org_id,
        into: input.into,
        under_org: input.under_org,
        slug: input.slug,
        name: input.name,
        placement: input.placement,
        keep_parent: input.keep_parent,
        reason: input.reason,
    };
    if input.dry_run {
        return Ok(Data { data: service.plan(&relocation, language).await? }.into_response());
    }
    let started =
        service.start(Actor::from(&principal), &relocation, input.request_id, true).await?;
    Ok(Created::new(format!("/kernel/relocations/{}", started.id), started).into_response())
}

/// Get a split or merge of tenants.
///
/// Requires `kernel.tenants.read` and an account of the platform tenant. `state` goes `planned` →
/// `frozen` → `copied` → `verified` → `switched` → `cleaned` → `done` (or `failed`); `error` tells
/// why a run stopped (the job runs it again).
#[utoipa::path(
    get,
    path = "/relocations/{id}",
    extensions(("x-permissions" = json!(["kernel.tenants.read"]))),
    operation_id = "get_relocation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the split or merge.")),
    responses(
        (status = 200, description = "The split or merge.", body = Data<TenantRelocation>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.read` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such run; code `tenancy.relocation_not_found`.", body = Problem),
    )
)]
async fn get_relocation(
    State(service): State<Arc<TenantRelocations>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<TenantRelocation>, Error> {
    principal.require(&TENANTS_READ)?;
    require_platform(&principal)?;
    Ok(Data { data: service.get(id).await? })
}

/// What a tenant asks the platform staff to do.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RelocationAsk {
    /// `split`: a branch of your tenant becomes a tenant of its own. `merge`: your tenant joins
    /// `into`.
    kind: RelocationKind,
    /// Split: the root unit of the branch.
    org_id: Option<Uuid>,
    /// Merge: id or slug of the tenant to join.
    #[validate(length(min = 1, max = 63))]
    into: Option<String>,
    /// Merge: the unit of `into` to go under.
    under_org: Option<Uuid>,
    /// Split: wished slug of the new tenant.
    #[validate(length(min = 1, max = 63))]
    slug: Option<String>,
    /// Split: wished name of the new tenant.
    #[validate(length(min = 1, max = 200))]
    name: Option<String>,
    /// Split: wished placement (database), if the platform offers a choice.
    #[validate(length(min = 1, max = 63))]
    placement: Option<String>,
    /// Why (contract, consent of the people whose data moves).
    #[validate(length(min = 1, max = 500))]
    reason: String,
}

/// Ask for a split or merge of your tenant.
///
/// Requires `kernel.relocations.request`. Nothing changes: the request goes to the work inbox of
/// the platform staff, who check it and run it (or not). Follow it at the `Location` of the answer.
#[utoipa::path(
    post,
    path = "/tenant/relocation-requests",
    extensions(("x-permissions" = json!(["kernel.relocations.request"]))),
    operation_id = "request_relocation",
    security(("bearer" = [])),
    request_body = RelocationAsk,
    responses(
        (status = 201, description = "The request was sent to the platform staff.", body = Data<TenantRelocation>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.relocations.request`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A value is invalid or missing; see `errors`.", body = Problem),
    )
)]
async fn request_relocation(
    State(service): State<Arc<TenantRelocations>>,
    tenant: TenantContext,
    principal: Principal,
    crate::http::ValidJson(input): crate::http::ValidJson<RelocationAsk>,
) -> Result<Response, Error> {
    principal.require(&RELOCATIONS_REQUEST)?;
    let ask = RelocationInput {
        kind: input.kind,
        tenant: tenant.id().to_string(),
        org_id: input.org_id,
        into: input.into,
        under_org: input.under_org,
        slug: input.slug,
        name: input.name,
        placement: input.placement,
        keep_parent: true,
        reason: input.reason,
    };
    let requested = service.request(Actor::from(&principal), &tenant, ask).await?;
    let path = format!("/kernel/tenant/relocation-requests/{}", requested.id);
    Ok(Created::new(path, requested).into_response())
}

/// Get a request of your tenant.
///
/// Requires `kernel.relocations.request`. `state` is `requested` until the platform staff run it.
#[utoipa::path(
    get,
    path = "/tenant/relocation-requests/{id}",
    extensions(("x-permissions" = json!(["kernel.relocations.request"]))),
    operation_id = "get_relocation_request",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the request.")),
    responses(
        (status = 200, description = "The request (and the run, once started).", body = Data<TenantRelocation>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.relocations.request`; code `permission_denied`.", body = Problem),
        (status = 404, description = "Your tenant has no such request; code `tenancy.relocation_not_found`.", body = Problem),
    )
)]
async fn get_relocation_request(
    State(service): State<Arc<TenantRelocations>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<TenantRelocation>, Error> {
    principal.require(&RELOCATIONS_REQUEST)?;
    Ok(Data { data: service.get_own(&tenant, id).await? })
}

/// Routes under `/api/v1/kernel`.
pub(crate) fn routes(service: Arc<TenantRelocations>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(relocate_tenant))
        .routes(routes!(get_relocation))
        .routes(routes!(request_relocation))
        .routes(routes!(get_relocation_request))
        .with_state(service)
}

/// The platform staff's work inbox: tenants' pending split / merge requests.
pub(crate) struct RelocationRequestsWork(pub(crate) Arc<TenantRelocations>);

#[async_trait::async_trait]
impl WorkSource for RelocationRequestsWork {
    fn id(&self) -> &'static str {
        "kernel.relocation_requests"
    }

    fn permission(&self) -> Permission {
        TENANTS_RELOCATE
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        _principal: &Principal,
        _target: OrgTarget,
    ) -> Result<u64, Error> {
        if tenant.id() != DEFAULT_TENANT {
            return Ok(0);
        }
        Ok(self.0.requested(1000).await?.len() as u64)
    }

    async fn items(
        &self,
        tenant: &TenantContext,
        _principal: &Principal,
        _target: OrgTarget,
        limit: u16,
    ) -> Result<Vec<WorkItem>, Error> {
        if tenant.id() != DEFAULT_TENANT {
            return Ok(Vec::new());
        }
        Ok(self
            .0
            .requested(u64::from(limit))
            .await?
            .into_iter()
            .map(|row| {
                WorkItem::new(
                    row.id.to_string(),
                    Severity::Attention,
                    "A tenant asks for a split or merge of tenants",
                    "platform.tenants",
                )
            })
            .collect())
    }
}
