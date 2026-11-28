//! REST (`/api/v1/kernel/domains…`) and MCP tools for custom domains; they call exactly the functions of
//! [`TenantDomains`].

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::{DOMAINS_MANAGE, TenantDomain, TenantDomains, tenant_domain_not_found};
use crate::access::Principal;
use crate::error::Error;
use crate::event::Actor;
use crate::mcp::{PageInput, Tool};
use crate::pagination::{Page, PageQuery, PageRequest};
use crate::response::{Created, Data, Problem};
use crate::tenancy::{OrgId, TenantContext};

/// A domain to add to the tenant.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NewTenantDomain)]
pub(crate) struct NewDomain {
    /// The domain, e.g. `shop.example` (punycode for international names). Not an IP address,
    /// `localhost` or the platform's own domain.
    #[schema(example = "shop.example")]
    domain: String,
}

/// Where a domain belongs in the tenant's organization tree.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = TenantDomainChanges)]
pub(crate) struct DomainChanges {
    /// Id of a node of the tenant's organization tree (e.g. one store, for its own site), or
    /// `null` for the tenant as a whole. Required: send `null` explicitly to detach.
    // `deserialize_with`: serde coi trường `Option` vắng mặt là `null`; ép phải gửi.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schema(required = true)]
    org_id: Option<Uuid>,
}

/// Which domain to ask about.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct AllowedQuery {
    /// The domain, as the TLS server sees it (e.g. `shop.example`).
    domain: String,
}

/// A domain that may get a TLS certificate.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = AllowedTenantDomain)]
pub(crate) struct Allowed {
    #[schema(example = "shop.example")]
    domain: String,
}

/// List this tenant's domains.
///
/// Requires `kernel.domains.manage`. Domains added to the tenant, with their status and the DNS
/// record that verifies them; removed domains are not listed. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/domains",
    extensions(("x-permissions" = json!(["kernel.domains.manage"]))),
    operation_id = "list_tenant_domains",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of the tenant's domains.", body = Page<TenantDomain>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.domains.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_domains(
    State(service): State<Arc<TenantDomains>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<TenantDomain>, Error> {
    principal.require(&DOMAINS_MANAGE)?;
    service.list(&tenant, &PageRequest::try_from(page)?).await
}

/// Add a domain to this tenant.
///
/// Requires `kernel.domains.manage`. The domain starts `pending`: set the returned DNS `record`
/// (TXT `_autoxide-challenge.<domain>`) at your DNS provider, then call
/// `POST /domains/{id}/verify` (pending domains are also checked every 10 minutes for 7 days).
/// Point the domain itself (A/AAAA or CNAME) at the platform's server. Adding a domain the tenant
/// already has returns it unchanged; retry safely with an `Idempotency-Key` header. Other tenants
/// waiting on the same domain do not block it: the first tenant to verify gets the domain, and the
/// others' claims turn `failed` when they verify.
#[utoipa::path(
    post,
    path = "/domains",
    extensions(("x-permissions" = json!(["kernel.domains.manage"]))),
    operation_id = "add_tenant_domain",
    security(("bearer" = [])),
    request_body = NewDomain,
    responses(
        (status = 201, description = "The domain, with the DNS record to set.", body = Data<TenantDomain>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.domains.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "Another tenant has verified the domain (`tenant_domain_taken`), or the installation runs one tenant (`tenant_single_mode`).", body = Problem),
        (status = 422, description = "Not a usable domain; code `tenant_domain_invalid`.", body = Problem),
    )
)]
async fn add_domain(
    State(service): State<Arc<TenantDomains>>,
    tenant: TenantContext,
    principal: Principal,
    crate::http::ValidJson(input): crate::http::ValidJson<NewDomain>,
) -> Result<Created<TenantDomain>, Error> {
    principal.require(&DOMAINS_MANAGE)?;
    let domain = service.add(Actor::from(&principal), &tenant, &input.domain).await?;
    Ok(Created::new(format!("/kernel/domains/{}", domain.id), domain))
}

/// Verify a domain now.
///
/// Requires `kernel.domains.manage`. Looks up the domain's TXT record right away and returns the
/// new status: `verified` when the record holds the expected value, otherwise still `pending`
/// with the reason in `check_error`. A `failed` domain can be verified again this way. When another
/// tenant has verified the same domain, the answer is `failed` with that reason in `check_error` at
/// once, whether or not your record is set.
#[utoipa::path(
    post,
    path = "/domains/{id}/verify",
    extensions(("x-permissions" = json!(["kernel.domains.manage"]))),
    operation_id = "verify_tenant_domain",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the domain.")),
    responses(
        (status = 200, description = "The domain after the lookup.", body = Data<TenantDomain>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.domains.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "The tenant has no such domain; code `tenant_domain_not_found`.", body = Problem),
    )
)]
async fn verify_domain(
    State(service): State<Arc<TenantDomains>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<TenantDomain>, Error> {
    principal.require(&DOMAINS_MANAGE)?;
    Ok(Data { data: service.verify(Actor::from(&principal), &tenant, id).await? })
}

/// Attach a domain to a node of the organization tree.
///
/// Requires `kernel.domains.manage`. Sets which node of the tenant's organization tree the domain
/// belongs to (`org_id`), e.g. a store that has its own site; `null` attaches it to the tenant as
/// a whole. The node must belong to the current tenant. The domain's status and its 7-day
/// verification deadline do not change. Sending the current value changes nothing.
#[utoipa::path(
    patch,
    path = "/domains/{id}",
    extensions(("x-permissions" = json!(["kernel.domains.manage"]))),
    operation_id = "update_tenant_domain",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the domain.")),
    request_body = DomainChanges,
    responses(
        (status = 200, description = "The domain after the change.", body = Data<TenantDomain>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.domains.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "The tenant has no such domain; code `tenant_domain_not_found`.", body = Problem),
        (status = 409, description = "The domain was removed; code `tenant_domain_revoked`.", body = Problem),
        (status = 422, description = "`org_id` is missing or not a node of this tenant (no installed module keeps an organization tree: only `null` is accepted); see `errors`.", body = Problem),
    )
)]
async fn update_domain(
    State(service): State<Arc<TenantDomains>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    crate::http::ValidJson(input): crate::http::ValidJson<DomainChanges>,
) -> Result<Data<TenantDomain>, Error> {
    principal.require(&DOMAINS_MANAGE)?;
    let org = input.org_id.map(OrgId::from_uuid);
    Ok(Data { data: service.assign_org(Actor::from(&principal), &tenant, id, org).await? })
}

/// Remove a domain from this tenant.
///
/// Requires `kernel.domains.manage`. The domain becomes `revoked`: requests to it no longer reach
/// the tenant (other servers follow within `AUTOXIDE_TENANCY_CACHE_SECONDS`) and no new TLS
/// certificate is issued for it. Removing it again changes nothing.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/domains/{id}",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["kernel.domains.manage"]))),
    operation_id = "remove_tenant_domain",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the domain.")),
    responses(
        (status = 204, description = "The domain is removed."),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.domains.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "The tenant has no such domain; code `tenant_domain_not_found`.", body = Problem),
    )
)]
async fn remove_domain(
    State(service): State<Arc<TenantDomains>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&DOMAINS_MANAGE)?;
    service.revoke(Actor::from(&principal), &tenant, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Tell whether a domain may get a TLS certificate.
///
/// No sign-in needed. For the `ask` URL of a reverse proxy that issues certificates on demand
/// (Caddy `on_demand_tls`): `200` when the domain is a verified domain of an active tenant,
/// `404` otherwise. Answers from a short cache; counted by the per-address rate limit.
#[utoipa::path(
    get,
    path = "/domains/allowed",
    extensions(("x-public" = json!(true))),
    operation_id = "check_tenant_domain_allowed",
    params(AllowedQuery),
    responses(
        (status = 200, description = "The domain is verified and its tenant is active.", body = Data<Allowed>),
        (status = 422, description = "The `domain` parameter is missing; code `validation_failed`, the field in `errors`.", body = Problem),
        (status = 404, description = "Not a verified domain of an active tenant; code `tenant_domain_not_found`.", body = Problem),
        (status = 429, description = "Too many requests from this address; code `rate_limited`.", body = Problem),
    )
)]
async fn domain_allowed(
    State(service): State<Arc<TenantDomains>>,
    Query(query): Query<AllowedQuery>,
) -> Result<Data<Allowed>, Error> {
    // Không dùng extractor khách: câu hỏi đến từ proxy, không thuộc khách nào.
    if service.allowed(&query.domain).await? {
        let domain = super::normalize_domain(&query.domain).unwrap_or(query.domain);
        Ok(Data { data: Allowed { domain } })
    } else {
        Err(tenant_domain_not_found())
    }
}

/// Routes under `/api/v1/kernel`.
pub(crate) fn routes(service: Arc<TenantDomains>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_domains, add_domain))
        .routes(routes!(domain_allowed))
        .routes(routes!(verify_domain))
        .routes(routes!(update_domain, remove_domain))
        .with_state(service)
}

// Đầu vào của tool `verify_domain`.
/// Which domain to verify.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DomainRef {
    /// Id of the domain, from `kernel.list_domains` or `kernel.add_domain`.
    id: Uuid,
}

// Đầu vào của tool `update_domain`.
/// Which domain, and the node of the organization tree it belongs to.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DomainOrg {
    /// Id of the domain, from `kernel.list_domains` or `kernel.add_domain`.
    id: Uuid,
    /// Id of a node of the tenant's organization tree, or `null` for the tenant as a whole.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schema(required = true)]
    org_id: Option<Uuid>,
}

/// MCP tools; they call exactly the functions REST uses. There is no delete-domain tool (a destructive action, done through REST).
pub(crate) fn tools(service: Arc<TenantDomains>) -> Vec<Tool> {
    let (list, add, verify, update) = (service.clone(), service.clone(), service.clone(), service);
    vec![
        Tool::new(
            "list_domains",
            "List the current tenant's own domains, newest first, with their status (`pending`, \
             `verified`, `failed`) and the DNS TXT record that verifies each.",
            move |tenant: TenantContext, _principal: Principal, input: PageInput| {
                let list = list.clone();
                async move { list.list(&tenant, &input.page()?).await }
            },
        )
        .requires(DOMAINS_MANAGE)
        .read_only(),
        Tool::new(
            "add_domain",
            "Add a domain (e.g. `shop.example`) to the current tenant. Returns it `pending` with \
             the DNS TXT record to set; then call `kernel.verify_domain`. Adding a domain the \
             tenant already has returns it unchanged.",
            move |tenant: TenantContext, principal: Principal, input: NewDomain| {
                let add = add.clone();
                async move { add.add(Actor::from(&principal), &tenant, &input.domain).await }
            },
        )
        .requires(DOMAINS_MANAGE),
        Tool::new(
            "verify_domain",
            "Look up the DNS TXT record of one of the current tenant's domains now and return its \
             new status: `verified`, or still `pending` with the reason in `check_error`.",
            move |tenant: TenantContext, principal: Principal, input: DomainRef| {
                let verify = verify.clone();
                async move { verify.verify(Actor::from(&principal), &tenant, input.id).await }
            },
        )
        .requires(DOMAINS_MANAGE),
        Tool::new(
            "update_domain",
            "Attach one of the current tenant's domains to a node of its organization tree \
             (`org_id`, e.g. a store with its own site), or to the tenant as a whole \
             (`org_id: null`). Status and verification are not affected.",
            move |tenant: TenantContext, principal: Principal, input: DomainOrg| {
                let update = update.clone();
                async move {
                    let org = input.org_id.map(OrgId::from_uuid);
                    update.assign_org(Actor::from(&principal), &tenant, input.id, org).await
                }
            },
        )
        .requires(DOMAINS_MANAGE),
    ]
}
