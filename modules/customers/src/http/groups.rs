//! Endpoint nhóm khách: danh sách, tạo, đọc, sửa, xoá, xem trước số hồ sơ khớp, thêm / bớt thành viên
//! của nhóm tay.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::views::{CustomerGroupPreview, CustomerGroupResponse};
use crate::permissions::{READ, SEGMENTS_MANAGE};
use crate::segments::{GroupInput, GroupRule};
use crate::service::CustomersService;

/// A new customer group.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = CreateCustomerGroupRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateGroupRequest {
    /// 1 to 100 characters, unique in the shop.
    name: String,
    /// Up to 500 characters.
    description: Option<String>,
    /// `manual` (default: staff add members) or `rule` (members follow `rule`).
    kind: Option<String>,
    /// Required for a `rule` group.
    rule: Option<GroupRule>,
    /// Customer group code for price lists (`a-z`, `0-9`, `_`), e.g. `vip`.
    price_group: Option<String>,
    /// Unit of the group; only customers of that unit and below can match. Omitted: the root unit.
    org_id: Option<Uuid>,
}

/// List customer groups.
///
/// Requires `customers.segments.manage` or `customers.read`; you see the groups of the units in
/// your scope.
#[utoipa::path(
    get,
    path = "/groups",
    extensions(("x-permissions" = json!(["customers.read", "customers.segments.manage"]))),
    operation_id = "list_customer_groups",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of groups.", body = Page<CustomerGroupResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_groups(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<CustomerGroupResponse>, Error> {
    if !principal.has(&SEGMENTS_MANAGE) {
        principal.require(&READ)?;
    }
    let page = service.list_groups(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(CustomerGroupResponse::from))
}

/// Create a customer group.
///
/// Requires `customers.segments.manage` at the unit. A rule group gets its members shortly after.
#[utoipa::path(
    post,
    path = "/groups",
    extensions(("x-permissions" = json!(["customers.segments.manage"]))),
    operation_id = "create_customer_group",
    security(("bearer" = [])),
    request_body = CreateGroupRequest,
    responses(
        (status = 201, description = "Created.", body = Data<CustomerGroupResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.segments.manage` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or not active (`org_frozen`).", body = Problem),
        (status = 409, description = "A group with this name exists; code `customers.group_name_taken`.", body = Problem),
        (status = 422, description = "A value or the rule is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_group(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateGroupRequest>,
) -> Result<Created<CustomerGroupResponse>, Error> {
    principal.require(&SEGMENTS_MANAGE)?;
    let input = GroupInput {
        name: Some(input.name),
        description: input.description,
        kind: input.kind,
        rule: input.rule,
        price_group: input.price_group,
        org_id: input.org_id,
    };
    let row = service.create_group(&tenant, &principal, input).await?;
    Ok(Created::new(format!("/customers/groups/{}", row.id), row.into()))
}

/// Get a customer group with its number of members.
///
/// Requires `customers.segments.manage` covering the group's unit.
#[utoipa::path(
    get,
    path = "/groups/{id}",
    extensions(("x-permissions" = json!(["customers.segments.manage"]))),
    operation_id = "get_customer_group",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the group.")),
    responses(
        (status = 200, description = "The group.", body = Data<CustomerGroupResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.segments.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No group with this id in your scope; code `customers.group_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_group(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<CustomerGroupResponse>, Error> {
    principal.require(&SEGMENTS_MANAGE)?;
    Ok(Data { data: service.group(&tenant, &principal, id).await?.into() })
}

/// Changes to a customer group. Omitted fields stay as they are; the kind and unit never change.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = UpdateCustomerGroupRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateGroupRequest {
    name: Option<String>,
    /// An empty string clears it.
    description: Option<String>,
    /// New rule of a rule group; members follow it shortly after.
    rule: Option<GroupRule>,
    /// An empty string clears it.
    price_group: Option<String>,
}

/// Change a customer group.
///
/// Requires `customers.segments.manage` covering the group's unit.
#[utoipa::path(
    put,
    path = "/groups/{id}",
    extensions(("x-permissions" = json!(["customers.segments.manage"]))),
    operation_id = "update_customer_group",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the group.")),
    request_body = UpdateGroupRequest,
    responses(
        (status = 200, description = "Changed.", body = Data<CustomerGroupResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.segments.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No group with this id in your scope; code `customers.group_not_found`.", body = Problem),
        (status = 409, description = "A group with this name exists; code `customers.group_name_taken`.", body = Problem),
        (status = 422, description = "A value or the rule is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn update_group(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<UpdateGroupRequest>,
) -> Result<Data<CustomerGroupResponse>, Error> {
    principal.require(&SEGMENTS_MANAGE)?;
    let input = GroupInput {
        name: input.name,
        description: input.description,
        kind: None,
        rule: input.rule,
        price_group: input.price_group,
        org_id: None,
    };
    Ok(Data { data: service.update_group(&tenant, &principal, id, input).await?.into() })
}

/// Delete a customer group and its memberships.
///
/// Requires `customers.segments.manage` covering the group's unit. Price lists and promotions
/// that name the group stop matching anyone.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/groups/{id}",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["customers.segments.manage"]))),
    operation_id = "delete_customer_group",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the group.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.segments.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No group with this id in your scope; code `customers.group_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_group(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&SEGMENTS_MANAGE)?;
    service.delete_group(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Count the customers a group matches now.
///
/// Requires `customers.segments.manage` or `customers.read` covering the group's unit. For a rule
/// group: customers matching the rule now (members follow shortly after a change); for a manual group: its members.
#[utoipa::path(
    get,
    path = "/groups/{id}/preview",
    extensions(("x-permissions" = json!(["customers.read", "customers.segments.manage"]))),
    operation_id = "preview_customer_group",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the group.")),
    responses(
        (status = 200, description = "Number of matching customers.", body = Data<CustomerGroupPreview>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No group with this id in your scope; code `customers.group_not_found`.", body = Problem),
    )
)]
pub(crate) async fn preview_group(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<CustomerGroupPreview>, Error> {
    if !principal.has(&SEGMENTS_MANAGE) {
        principal.require(&READ)?;
    }
    let matching = service.preview_group(&tenant, &principal, id).await?;
    Ok(Data { data: CustomerGroupPreview { matching } })
}

/// Add a customer to a manual group.
///
/// Requires `customers.segments.manage` covering the group's unit and `customers.read` covering
/// the customer. Adding a member twice does nothing.
#[utoipa::path(
    post,
    path = "/groups/{id}/members/{profile_id}",
    extensions(("x-permissions" = json!(["customers.segments.manage"]))),
    operation_id = "add_customer_group_member",
    security(("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Id of the group."),
        ("profile_id" = Uuid, Path, description = "Id of the customer profile."),
    ),
    responses(
        (status = 204, description = "The customer is a member."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.segments.manage` or `customers.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such group (`customers.group_not_found`) or customer (`customers.profile_not_found`) in your scope.", body = Problem),
        (status = 409, description = "The group follows a rule; code `customers.group_not_manual`.", body = Problem),
    )
)]
pub(crate) async fn add_member(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, profile_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, Error> {
    principal.require(&SEGMENTS_MANAGE)?;
    service.add_member(&tenant, &principal, id, profile_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Remove a customer from a manual group.
///
/// Requires `customers.segments.manage` covering the group's unit and `customers.read` covering
/// the customer. Removing a non-member does nothing.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/groups/{id}/members/{profile_id}",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["customers.segments.manage"]))),
    operation_id = "remove_customer_group_member",
    security(("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Id of the group."),
        ("profile_id" = Uuid, Path, description = "Id of the customer profile."),
    ),
    responses(
        (status = 204, description = "The customer is not a member."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.segments.manage` or `customers.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such group (`customers.group_not_found`) or customer (`customers.profile_not_found`) in your scope.", body = Problem),
        (status = 409, description = "The group follows a rule; code `customers.group_not_manual`.", body = Problem),
    )
)]
pub(crate) async fn remove_member(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, profile_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, Error> {
    principal.require(&SEGMENTS_MANAGE)?;
    service.remove_member(&tenant, &principal, id, profile_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
