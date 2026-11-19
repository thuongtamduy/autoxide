//! Nhóm nút cho báo cáo (bước 38, D-232), dưới `/api/v1/organizations/groups`. Handler chỉ: lấy dữ
//! liệu vào → kiểm permission → gọi service → trả envelope.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::groups::{GroupChanges, GroupView, entity::member};
use crate::permissions::{GROUPS_MANAGE, UNITS_READ};
use crate::service::OrgService;

pub(crate) fn routes() -> OpenApiRouter<Arc<OrgService>> {
    OpenApiRouter::new()
        .routes(routes!(list_groups, create_group))
        .routes(routes!(get_group, update_group, delete_group))
        .routes(routes!(list_group_members, add_group_members))
        .routes(routes!(remove_group_member))
}

/// A unit group: any units of the tenant gathered to filter and group reports (e.g. "stores in
/// the north"). A group grants no access.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct GroupResponse {
    id: Uuid,
    #[schema(example = "Stores in the north")]
    name: String,
    description: Option<String>,
    /// Units of the group within your `organizations.units.read` scope; units outside it are not
    /// counted.
    member_count: u64,
    /// Units of the group outside your scope: only counted, never listed or named.
    excluded_org_count: u64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<GroupView> for GroupResponse {
    fn from(view: GroupView) -> Self {
        GroupResponse {
            id: view.row.id,
            name: view.row.name,
            description: view.row.description,
            member_count: view.member_count,
            excluded_org_count: view.excluded_org_count,
            created_at: view.row.created_at,
            updated_at: view.row.updated_at,
        }
    }
}

/// A unit in a group.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct GroupMemberResponse {
    /// Id of the unit (see `GET /api/v1/organizations/units/{id}`).
    org_id: Uuid,
    added_at: DateTime<Utc>,
}

impl From<member::Model> for GroupMemberResponse {
    fn from(row: member::Model) -> Self {
        GroupMemberResponse { org_id: row.org_id, added_at: row.added_at }
    }
}

/// Result of adding units to a group.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct AddedResponse {
    /// Units that were not in the group before.
    added: u64,
    /// Units in the group now (all of them, whatever your scope).
    member_count: u64,
}

/// Which groups to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct GroupsQuery {
    /// Words in the name; accents and case are ignored.
    pub q: Option<String>,
}

/// A new unit group.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateGroupRequest {
    /// 1 to 200 characters, unique in the tenant (accents and case ignored).
    #[schema(example = "Stores in the north")]
    pub name: String,
    /// Up to 1000 characters.
    pub description: Option<String>,
}

/// Changes to a unit group; omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateGroupRequest {
    /// New name, 1 to 200 characters.
    pub name: Option<String>,
    /// New description, up to 1000 characters; an empty string removes it.
    pub description: Option<String>,
}

impl From<UpdateGroupRequest> for GroupChanges {
    fn from(input: UpdateGroupRequest) -> Self {
        GroupChanges { name: input.name, description: input.description.map(Some) }
    }
}

/// Units to add to a group.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddMembersRequest {
    /// 1 to 200 unit ids of this tenant, from any branch. Units already in the group are skipped.
    pub org_ids: Vec<Uuid>,
}

/// List the unit groups of the tenant.
///
/// Requires `organizations.units.read` (any scope). `member_count` only counts the units within
/// your scope.
#[utoipa::path(
    get,
    path = "/groups",
    extensions(("x-permissions" = json!(["organizations.units.read"]))),
    operation_id = "list_unit_groups",
    security(("bearer" = [])),
    params(GroupsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of groups.", body = Page<GroupResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.units.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; code `validation_failed`.", body = Problem),
    )
)]
async fn list_groups(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<GroupsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<GroupResponse>, Error> {
    principal.require(&UNITS_READ)?;
    let page = PageRequest::try_from(page)?;
    let found = service.list_groups(&tenant, &principal, filter.q.as_deref(), &page).await?;
    Ok(found.map(GroupResponse::from))
}

/// Create a unit group.
///
/// Requires `organizations.groups.manage` (granted for the whole tenant only). A group only filters
/// and groups reports (`group_ids`, `group_by=group` of `GET /api/v1/reporting/summary`); it grants
/// no access.
#[utoipa::path(
    post,
    path = "/groups",
    extensions(("x-permissions" = json!(["organizations.groups.manage"]))),
    operation_id = "create_unit_group",
    security(("bearer" = [])),
    request_body = CreateGroupRequest,
    responses(
        (status = 201, description = "Created.", body = Data<GroupResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.groups.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "Another group has this name; code `organizations.group_name_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn create_group(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateGroupRequest>,
) -> Result<Created<GroupResponse>, Error> {
    principal.require(&GROUPS_MANAGE)?;
    let row = service
        .create_group(&tenant, &principal, &input.name, input.description.as_deref())
        .await?;
    let location = format!("/organizations/groups/{}", row.id);
    Ok(Created::new(location, GroupView { row, member_count: 0, excluded_org_count: 0 }.into()))
}

/// Get a unit group.
///
/// Requires `organizations.units.read` (any scope); `member_count` only counts units within it.
#[utoipa::path(
    get,
    path = "/groups/{id}",
    extensions(("x-permissions" = json!(["organizations.units.read"]))),
    operation_id = "get_unit_group",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the group.")),
    responses(
        (status = 200, description = "The group.", body = Data<GroupResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.units.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such group in this tenant; code `organizations.group_not_found`.", body = Problem),
    )
)]
async fn get_group(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<GroupResponse>, Error> {
    principal.require(&UNITS_READ)?;
    let view = service.get_group(&tenant, &principal, id).await?;
    Ok(Tagged { version: view.row.updated_at, data: view.into() })
}

/// Rename a unit group or change its description.
///
/// Requires `organizations.groups.manage`.
#[utoipa::path(
    patch,
    path = "/groups/{id}",
    extensions(("x-permissions" = json!(["organizations.groups.manage"]))),
    operation_id = "update_unit_group",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the group."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the change is made only if nobody changed the record since; otherwise `412`.")),
    request_body = UpdateGroupRequest,
    responses(
        (status = 200, description = "The group after the change.", body = Data<GroupResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.groups.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such group in this tenant; code `organizations.group_not_found`.", body = Problem),
        (status = 409, description = "Another group has this name; code `organizations.group_name_taken`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the record changed since you read it. Get it again and retry; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_group(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateGroupRequest>,
) -> Result<Tagged<GroupResponse>, Error> {
    principal.require(&GROUPS_MANAGE)?;
    let expected = if_match.expected()?;
    service.update_group(&tenant, &principal, id, input.into(), expected).await?;
    let view = service.get_group(&tenant, &principal, id).await?;
    Ok(Tagged { version: view.row.updated_at, data: view.into() })
}

/// Delete a unit group.
///
/// Requires `organizations.groups.manage`. The units and the report facts stay; only the group and
/// its member list go.
#[utoipa::path(
    delete,
    path = "/groups/{id}",
    extensions(("x-permissions" = json!(["organizations.groups.manage"]))),
    operation_id = "delete_unit_group",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the group.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.groups.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such group in this tenant; code `organizations.group_not_found`.", body = Problem),
    )
)]
async fn delete_group(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&GROUPS_MANAGE)?;
    service.delete_group(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List the units of a group that are within your scope.
///
/// Requires `organizations.units.read`. Units outside your scope are never listed; their number is
/// `excluded_org_count` of `GET /groups/{id}`.
#[utoipa::path(
    get,
    path = "/groups/{id}/members",
    extensions(("x-permissions" = json!(["organizations.units.read"]))),
    operation_id = "list_unit_group_members",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the group."), PageQuery),
    responses(
        (status = 200, description = "One page of the units within your scope.", body = Page<GroupMemberResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.units.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such group in this tenant; code `organizations.group_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; code `validation_failed`.", body = Problem),
    )
)]
async fn list_group_members(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<GroupMemberResponse>, Error> {
    principal.require(&UNITS_READ)?;
    let page = PageRequest::try_from(page)?;
    let found = service.list_members(&tenant, &principal, id, &page).await?;
    Ok(found.map(GroupMemberResponse::from))
}

/// Add units to a group.
///
/// Requires `organizations.groups.manage`. Units already in the group are skipped, so calling again
/// is safe. A group has at most 1000 units.
#[utoipa::path(
    post,
    path = "/groups/{id}/members",
    extensions(("x-permissions" = json!(["organizations.groups.manage"]))),
    operation_id = "add_unit_group_members",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the group.")),
    request_body = AddMembersRequest,
    responses(
        (status = 200, description = "How many units were added and how many the group has now.", body = Data<AddedResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.groups.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such group in this tenant; code `organizations.group_not_found`.", body = Problem),
        (status = 422, description = "A unit is not in this tenant (`organizations.group_member_invalid`), the group would pass 1000 units (`organizations.group_too_large`), or `org_ids` is empty or too long (`validation_failed`).", body = Problem),
    )
)]
async fn add_group_members(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AddMembersRequest>,
) -> Result<Data<AddedResponse>, Error> {
    principal.require(&GROUPS_MANAGE)?;
    let added = service.add_members(&tenant, &principal, id, &input.org_ids).await?;
    Ok(Data { data: AddedResponse { added: added.added, member_count: added.member_count } })
}

/// Remove a unit from a group.
///
/// Requires `organizations.groups.manage`. Removing a unit that is not in the group also answers
/// 204.
#[utoipa::path(
    delete,
    path = "/groups/{id}/members/{org_id}",
    extensions(("x-permissions" = json!(["organizations.groups.manage"]))),
    operation_id = "remove_unit_group_member",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the group."), ("org_id" = Uuid, Path, description = "Id of the unit.")),
    responses(
        (status = 204, description = "The unit is not in the group (any more)."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.groups.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such group in this tenant; code `organizations.group_not_found`.", body = Problem),
    )
)]
async fn remove_group_member(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, org_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, Error> {
    principal.require(&GROUPS_MANAGE)?;
    service.remove_member(&tenant, &principal, id, org_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
