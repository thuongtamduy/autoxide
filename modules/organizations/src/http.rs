//! Endpoint REST của module `organizations`, dưới `/api/v1/organizations`. Handler chỉ: lấy dữ
//! liệu vào → kiểm permission → gọi service → trả envelope. Phạm vi theo nút do service kiểm.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest, page_of};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::permissions::{SETTINGS_WRITE, TYPES_MANAGE, UNITS_MANAGE, UNITS_READ, UNITS_WRITE};
use crate::service::{
    NewUnit, NewUnitType, OrgService, UnitChanges, UnitFilter, UnitRow, UnitTypeRow,
};

pub(crate) fn routes(service: Arc<OrgService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_units, create_unit))
        .routes(routes!(get_unit, update_unit))
        .routes(routes!(list_ancestors))
        .routes(routes!(move_unit))
        .routes(routes!(change_unit_status))
        .routes(routes!(list_unit_types, create_unit_type))
        .routes(routes!(update_unit_type, delete_unit_type))
        .routes(routes!(list_settings))
        .routes(routes!(get_unit_settings))
        .routes(routes!(set_unit_setting, clear_unit_setting))
        .merge(crate::http_profile::routes())
        .merge(crate::http_groups::routes())
        .with_state(service)
}

/// Status of a unit. Only an `active` unit whose units above are all `active` can be written to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UnitStatusValue {
    Pending,
    Active,
    Suspended,
    Archived,
}

/// A unit of the organization tree of the tenant (head office, region, store, warehouse…).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct UnitResponse {
    /// Id of the unit. The root unit has the id of the tenant.
    id: Uuid,
    /// The unit above; `null` only for the root.
    parent_id: Option<Uuid>,
    /// Code of the unit type (see `GET /api/v1/organizations/unit-types`); `tenant` for the root.
    #[schema(example = "store")]
    unit_type: String,
    #[schema(example = "Store District 1")]
    name: String,
    /// Internal code, unique in the tenant when set.
    #[schema(example = "HCM-001")]
    code: Option<String>,
    #[schema(value_type = UnitStatusValue)]
    status: String,
    /// Why the status was last changed, when given.
    status_reason: Option<String>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<UnitRow> for UnitResponse {
    fn from(row: UnitRow) -> Self {
        UnitResponse {
            id: row.id,
            parent_id: row.parent_id,
            unit_type: row.unit_type,
            name: row.name,
            code: row.code,
            status: row.status,
            status_reason: row.status_reason,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A kind of unit the tenant uses, e.g. `store` or `warehouse`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct UnitTypeResponse {
    #[schema(example = "store")]
    code: String,
    #[schema(example = "Store")]
    name: String,
    description: String,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<UnitTypeRow> for UnitTypeResponse {
    fn from(row: UnitTypeRow) -> Self {
        UnitTypeResponse {
            code: row.code,
            name: row.name,
            description: row.description,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Which units to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct UnitsQuery {
    /// Only the units directly below this unit.
    pub parent_id: Option<Uuid>,
    /// Only units of this type (code).
    #[serde(rename = "type")]
    #[param(rename = "type")]
    pub unit_type: Option<String>,
    /// Only units with this status.
    #[param(value_type = Option<UnitStatusValue>, inline)]
    pub status: Option<String>,
    /// Words in the name; accents and case are ignored (`cafe` also finds `Café`).
    pub q: Option<String>,
}

impl From<UnitsQuery> for UnitFilter {
    fn from(query: UnitsQuery) -> Self {
        UnitFilter {
            parent_id: query.parent_id,
            unit_type: query.unit_type,
            status: query.status,
            q: query.q,
        }
    }
}

/// A new unit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateUnitRequest {
    /// The unit to place it under; you need `organizations.units.write` there.
    pub parent_id: Uuid,
    /// Code of an existing unit type (not `tenant`).
    #[schema(example = "store")]
    pub unit_type: String,
    /// 1 to 200 characters.
    #[schema(example = "Store District 1")]
    pub name: String,
    /// Internal code, 1 to 64 characters, unique in the tenant.
    pub code: Option<String>,
}

impl From<CreateUnitRequest> for NewUnit {
    fn from(input: CreateUnitRequest) -> Self {
        NewUnit {
            parent_id: input.parent_id,
            unit_type: input.unit_type,
            name: input.name,
            code: input.code.filter(|code| !code.trim().is_empty()),
        }
    }
}

/// Changes to a unit; omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateUnitRequest {
    /// New name, 1 to 200 characters.
    pub name: Option<String>,
    /// New internal code, 1 to 64 characters; an empty string removes the code.
    pub code: Option<String>,
}

impl From<UpdateUnitRequest> for UnitChanges {
    fn from(input: UpdateUnitRequest) -> Self {
        UnitChanges {
            name: input.name,
            code: input.code.map(|code| Some(code).filter(|code| !code.trim().is_empty())),
        }
    }
}

/// Where to move a unit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MoveUnitRequest {
    /// The new parent; you need `organizations.units.write` there too.
    pub parent_id: Uuid,
}

/// A new status for a unit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UnitStatusRequest {
    #[schema(value_type = UnitStatusValue)]
    pub status: String,
    /// Why, up to 500 characters; kept with the unit and in the audit log.
    pub reason: Option<String>,
}

/// A new unit type.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateUnitTypeRequest {
    /// A lower-case letter, then up to 31 lower-case letters, digits or `_`. `tenant` is reserved.
    #[schema(example = "store")]
    pub code: String,
    /// 1 to 100 characters.
    #[schema(example = "Store")]
    pub name: String,
    /// Up to 1000 characters.
    #[serde(default)]
    pub description: String,
}

impl From<CreateUnitTypeRequest> for NewUnitType {
    fn from(input: CreateUnitTypeRequest) -> Self {
        NewUnitType { code: input.code, name: input.name, description: input.description }
    }
}

/// Changes to a unit type; omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateUnitTypeRequest {
    /// 1 to 100 characters.
    pub name: Option<String>,
    /// Up to 1000 characters.
    pub description: Option<String>,
}

/// List the organization units in your scope.
///
/// Requires `organizations.units.read`; you see the units your scope covers (your unit, the units
/// below it, or the whole tenant). Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/units",
    extensions(("x-permissions" = json!(["organizations.units.read"]))),
    operation_id = "list_units",
    security(("bearer" = [])),
    params(UnitsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of units.", body = Page<UnitResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.units.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid, e.g. `parent_id` is not a UUID; code `validation_failed`, every problem in `errors`.", body = Problem),
    )
)]
async fn list_units(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<UnitsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<UnitResponse>, Error> {
    principal.require(&UNITS_READ)?;
    let page =
        service.list(&tenant, &principal, &filter.into(), &PageRequest::try_from(page)?).await?;
    Ok(page.map(UnitResponse::from))
}

/// Create an organization unit.
///
/// Requires `organizations.units.write` at the parent. The unit is `active` when you also have
/// `organizations.units.manage` at the parent; otherwise it is `pending` until someone who has
/// approves it (`POST /units/{id}/status`).
#[utoipa::path(
    post,
    path = "/units",
    extensions(("x-permissions" = json!(["organizations.units.write"]))),
    operation_id = "create_unit",
    security(("bearer" = [])),
    request_body = CreateUnitRequest,
    responses(
        (status = 201, description = "Created.", body = Data<UnitResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.units.write` (`permission_denied`), the parent is outside your scope (`org_out_of_scope`), or the parent is not active (`org_frozen`).", body = Problem),
        (status = 404, description = "The parent does not exist (`org_not_found`) or the unit type does not exist (`organizations.unit_type_not_found`).", body = Problem),
        (status = 409, description = "Another unit has this code; code `organizations.unit_code_taken`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or the tree would be too deep (`org_depth_exceeded`).", body = Problem),
    )
)]
async fn create_unit(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateUnitRequest>,
) -> Result<Created<UnitResponse>, Error> {
    principal.require(&UNITS_WRITE)?;
    let row = service.create(&tenant, &principal, input.into()).await?;
    Ok(Created::new(format!("/organizations/units/{}", row.id), row.into()))
}

/// Get an organization unit.
///
/// Requires `organizations.units.read` covering the unit; a unit outside your scope answers 404.
#[utoipa::path(
    get,
    path = "/units/{id}",
    extensions(("x-permissions" = json!(["organizations.units.read"]))),
    operation_id = "get_unit",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the unit.")),
    responses(
        (status = 200, description = "The unit.", body = Data<UnitResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.units.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such unit in your scope; code `organizations.unit_not_found`.", body = Problem),
    )
)]
async fn get_unit(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<UnitResponse>, Error> {
    principal.require(&UNITS_READ)?;
    let row = service.get(&tenant, &principal, id).await?;
    Ok(Tagged { version: row.updated_at, data: row.into() })
}

/// Rename an organization unit or change its code.
///
/// Requires `organizations.units.write` covering the unit; the unit and every unit above it must
/// be active.
#[utoipa::path(
    patch,
    path = "/units/{id}",
    extensions(("x-permissions" = json!(["organizations.units.write"]))),
    operation_id = "update_unit",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the unit."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the change is made only if nobody changed the record since; otherwise `412`.")),
    request_body = UpdateUnitRequest,
    responses(
        (status = 200, description = "The unit after the change.", body = Data<UnitResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.units.write` (`permission_denied`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 404, description = "No such unit in your scope; code `organizations.unit_not_found`.", body = Problem),
        (status = 409, description = "Another unit has this code; code `organizations.unit_code_taken`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the record changed since you read it. Get it again and retry; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_unit(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateUnitRequest>,
) -> Result<Tagged<UnitResponse>, Error> {
    principal.require(&UNITS_WRITE)?;
    let expected = if_match.expected()?;
    let row = service.update(&tenant, &principal, id, input.into(), expected).await?;
    Ok(Tagged { version: row.updated_at, data: row.into() })
}

/// List the units above an organization unit.
///
/// Requires `organizations.units.read` covering the unit. Nearest first, ending with the root
/// (`order=desc`: from the root down); paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/units/{id}/ancestors",
    extensions(("x-permissions" = json!(["organizations.units.read"]))),
    operation_id = "list_unit_ancestors",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the unit."), NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the units above, nearest first; empty for the root.", body = Page<UnitResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.units.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such unit in your scope; code `organizations.unit_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_ancestors(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<UnitResponse>, Error> {
    principal.require(&UNITS_READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.ancestors(&tenant, &principal, id).await?;
    // Chuỗi tổ tiên có trần theo độ sâu của cây (không lớn dần theo dữ liệu) và thứ tự là khoảng cách
    // tới nút, không phải cột nào: phân trang trong bộ nhớ, khoá là khoảng cách (D-20261125-list-pages mục 4).
    let rows: Vec<(String, UnitRow)> =
        rows.into_iter().enumerate().map(|(step, row)| (format!("{step:05}"), row)).collect();
    Ok(page_of(rows, |(step, _)| step.clone(), &page)?.map(|(_, row)| UnitResponse::from(row)))
}

/// Move an organization unit, with everything below it, under another parent.
///
/// Requires `organizations.units.write` covering the unit and the new parent; both must be active.
#[utoipa::path(
    post,
    path = "/units/{id}/move",
    extensions(("x-permissions" = json!(["organizations.units.write"]))),
    operation_id = "move_unit",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the unit.")),
    request_body = MoveUnitRequest,
    responses(
        (status = 200, description = "The unit after the move.", body = Data<UnitResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission (`permission_denied`), the new parent is outside your scope (`org_out_of_scope`), or a unit is not active (`org_frozen`).", body = Problem),
        (status = 404, description = "No such unit in your scope (`organizations.unit_not_found`), or the new parent does not exist (`org_not_found`).", body = Problem),
        (status = 409, description = "The root cannot move; code `org_root_immutable`.", body = Problem),
        (status = 422, description = "The new parent is the unit itself or below it (`org_cycle`), or the tree would be too deep (`org_depth_exceeded`).", body = Problem),
    )
)]
async fn move_unit(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<MoveUnitRequest>,
) -> Result<Data<UnitResponse>, Error> {
    principal.require(&UNITS_WRITE)?;
    let row = service.relocate(&tenant, &principal, id, input.parent_id).await?;
    Ok(Data { data: row.into() })
}

/// Approve, suspend, archive or reactivate an organization unit.
///
/// Requires `organizations.units.manage` covering the unit. Allowed: `pending → active`,
/// `active ↔ suspended`, `active` or `suspended → archived`, `archived → active`. While a unit is
/// not active, nothing can be written to it or to any unit below it.
#[utoipa::path(
    post,
    path = "/units/{id}/status",
    extensions(("x-permissions" = json!(["organizations.units.manage"]))),
    operation_id = "change_unit_status",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the unit.")),
    request_body = UnitStatusRequest,
    responses(
        (status = 200, description = "The unit after the change.", body = Data<UnitResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.units.manage` (`permission_denied`), or it does not cover this unit (`org_out_of_scope`).", body = Problem),
        (status = 404, description = "No such unit in your scope; code `organizations.unit_not_found`.", body = Problem),
        (status = 409, description = "The change is not allowed from the current status (`organizations.invalid_transition`), or the unit is the root (`org_root_immutable`).", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn change_unit_status(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<UnitStatusRequest>,
) -> Result<Data<UnitResponse>, Error> {
    principal.require(&UNITS_MANAGE)?;
    let row = service.change_status(&tenant, &principal, id, &input.status, input.reason).await?;
    Ok(Data { data: row.into() })
}

/// List the unit types of the tenant.
///
/// Requires `organizations.units.read`. By code (in the database's text order) unless
/// `order=desc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/unit-types",
    extensions(("x-permissions" = json!(["organizations.units.read"]))),
    operation_id = "list_unit_types",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the unit types of the tenant, by code.", body = Page<UnitTypeResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.units.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_unit_types(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<UnitTypeResponse>, Error> {
    principal.require(&UNITS_READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.list_types(&tenant, &principal, &page).await?;
    Ok(rows.map(UnitTypeResponse::from))
}

/// Create a unit type.
///
/// Requires `organizations.types.manage`.
#[utoipa::path(
    post,
    path = "/unit-types",
    extensions(("x-permissions" = json!(["organizations.types.manage"]))),
    operation_id = "create_unit_type",
    security(("bearer" = [])),
    request_body = CreateUnitTypeRequest,
    responses(
        (status = 201, description = "Created.", body = Data<UnitTypeResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.types.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "A unit type with this code exists; code `organizations.unit_type_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn create_unit_type(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateUnitTypeRequest>,
) -> Result<Created<UnitTypeResponse>, Error> {
    principal.require(&TYPES_MANAGE)?;
    let row = service.create_type(&tenant, &principal, input.into()).await?;
    Ok(Created::new(format!("/organizations/unit-types/{}", row.code), row.into()))
}

/// Rename a unit type or change its description.
///
/// Requires `organizations.types.manage`.
#[utoipa::path(
    put,
    path = "/unit-types/{code}",
    extensions(("x-permissions" = json!(["organizations.types.manage"]))),
    operation_id = "update_unit_type",
    security(("bearer" = [])),
    params(("code" = String, Path, description = "Code of the unit type.")),
    request_body = UpdateUnitTypeRequest,
    responses(
        (status = 200, description = "The unit type after the change.", body = Data<UnitTypeResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.types.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such unit type; code `organizations.unit_type_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_unit_type(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(code): Path<String>,
    ValidJson(input): ValidJson<UpdateUnitTypeRequest>,
) -> Result<Data<UnitTypeResponse>, Error> {
    principal.require(&TYPES_MANAGE)?;
    let row =
        service.update_type(&tenant, &principal, &code, input.name, input.description).await?;
    Ok(Data { data: row.into() })
}

/// Delete a unit type.
///
/// Requires `organizations.types.manage`. A type still used by a unit cannot be deleted.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/unit-types/{code}",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["organizations.types.manage"]))),
    operation_id = "delete_unit_type",
    security(("bearer" = [])),
    params(("code" = String, Path, description = "Code of the unit type.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.types.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such unit type; code `organizations.unit_type_not_found`.", body = Problem),
        (status = 409, description = "Units still use this type (`organizations.unit_type_in_use`), or it is the type of the root (`organizations.unit_type_reserved`).", body = Problem),
    )
)]
async fn delete_unit_type(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(code): Path<String>,
) -> Result<StatusCode, Error> {
    principal.require(&TYPES_MANAGE)?;
    service.delete_type(&tenant, &principal, &code).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A setting that units can set and the units below them inherit.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct SettingResponse {
    /// `<module>.<name>`, e.g. `organizations.timezone`.
    #[schema(example = "organizations.timezone")]
    key: String,
    /// What it is for and which values it takes.
    description: String,
    /// The value when no unit sets it; `null`: none.
    default: Option<serde_json::Value>,
    /// The shape of the value as a JSON Schema (`type`: `string`, `boolean`, `integer`, `number`,
    /// `array` or `object`, with `enum`, `pattern`, `items`… where they apply), for building the
    /// right input; `null` when the module does not say. The server checks every value anyway.
    #[schema(value_type = Option<Object>, example = json!({"type": "string", "enum": ["en", "vi"]}))]
    schema: Option<serde_json::Value>,
}

/// The value of a setting at a unit.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct UnitSettingResponse {
    /// Key of the setting.
    #[schema(example = "organizations.timezone")]
    key: String,
    /// The value in force; `null` when nothing sets it and it has no default.
    #[schema(example = "Asia/Ho_Chi_Minh")]
    value: Option<serde_json::Value>,
    /// The unit that set this value (this unit or one above it); `null`: the default.
    source_unit_id: Option<Uuid>,
    /// `true` when the value comes from a unit above or the default, not from this unit.
    inherited: bool,
}

impl From<crate::api::EffectiveSetting> for UnitSettingResponse {
    fn from(setting: crate::api::EffectiveSetting) -> Self {
        UnitSettingResponse {
            key: setting.key,
            value: setting.value,
            source_unit_id: setting.source_unit_id.map(|org| org.as_uuid()),
            inherited: setting.inherited,
        }
    }
}

/// A new value for a setting.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingValueRequest {
    /// The value, checked by the setting (see its description).
    #[schema(example = "Asia/Ho_Chi_Minh")]
    value: serde_json::Value,
}

/// List the settings units can set.
///
/// Any signed-in user. Modules declare them; each unit inherits the value of the nearest unit
/// above that sets it, or the default. By key unless `order=desc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/settings",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_org_settings",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the settings, by key.", body = Page<SettingResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_settings(
    State(service): State<Arc<OrgService>>,
    _principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<SettingResponse>, Error> {
    let page = PageRequest::try_from(page)?;
    // Sổ cài đặt do module khai trong code: phân trang trong bộ nhớ theo khoá (D-20261125-list-pages mục 4).
    let data: Vec<SettingResponse> = service
        .catalog
        .iter()
        .map(|setting| SettingResponse {
            key: setting.key().to_owned(),
            description: setting.description().to_owned(),
            default: setting.default_value(),
            schema: setting.schema(),
        })
        .collect();
    page_of(data, |setting| setting.key.clone(), &page)
}

/// Get the settings in force at an organization unit.
///
/// Requires `organizations.units.read` covering the unit. Each value says where it comes from:
/// this unit, a unit above, or the default. By key unless `order=desc`; paged with `limit` /
/// `cursor`.
#[utoipa::path(
    get,
    path = "/units/{id}/settings",
    extensions(("x-permissions" = json!(["organizations.units.read"]))),
    operation_id = "get_unit_settings",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the unit."), NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the settings at this unit, by key.", body = Page<UnitSettingResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.units.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such unit in your scope; code `organizations.unit_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn get_unit_settings(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<UnitSettingResponse>, Error> {
    principal.require(&UNITS_READ)?;
    let page = PageRequest::try_from(page)?;
    unit_settings_page(&service, &tenant, &principal, id, &page).await
}

/// Một trang cài đặt hiệu lực ở nút `id` (REST và tool `organizations.get_settings`). Không nằm trong
/// database: mỗi cài đặt module khai (trong code) một dòng, giá trị gộp theo chuỗi tổ tiên — phân trang
/// trong bộ nhớ theo khoá (D-20261125-list-pages mục 4).
pub(crate) async fn unit_settings_page(
    service: &OrgService,
    tenant: &TenantContext,
    principal: &Principal,
    id: Uuid,
    page: &PageRequest,
) -> Result<Page<UnitSettingResponse>, Error> {
    let found = service.effective_settings(tenant, principal, id).await?;
    Ok(page_of(found, |setting| setting.key.clone(), page)?.map(UnitSettingResponse::from))
}

/// Set a setting of an organization unit.
///
/// Requires `organizations.settings.write` covering the unit; the unit must be active. Units
/// below it follow, unless they set their own value.
#[utoipa::path(
    put,
    path = "/units/{id}/settings/{key}",
    extensions(("x-permissions" = json!(["organizations.settings.write"]))),
    operation_id = "set_unit_setting",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the unit."), ("key" = String, Path, description = "Key of the setting, e.g. `organizations.timezone`.")),
    request_body = SettingValueRequest,
    responses(
        (status = 200, description = "The setting at this unit after the change.", body = Data<UnitSettingResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission (`permission_denied`), it does not cover this unit (`org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 404, description = "No such unit in your scope (`organizations.unit_not_found`), or no such setting (`organizations.setting_unknown`).", body = Problem),
        (status = 422, description = "The value is not accepted by the setting; code `organizations.setting_invalid`.", body = Problem),
    )
)]
async fn set_unit_setting(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, key)): Path<(Uuid, String)>,
    ValidJson(input): ValidJson<SettingValueRequest>,
) -> Result<Data<UnitSettingResponse>, Error> {
    principal.require(&SETTINGS_WRITE)?;
    let setting = service.set_setting(&tenant, &principal, id, &key, Some(input.value)).await?;
    Ok(Data { data: setting.into() })
}

/// Clear a setting of an organization unit, so it inherits again.
///
/// Requires `organizations.settings.write` covering the unit; the unit must be active. Clearing a
/// setting the unit does not set changes nothing.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/units/{id}/settings/{key}",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["organizations.settings.write"]))),
    operation_id = "clear_unit_setting",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the unit."), ("key" = String, Path, description = "Key of the setting.")),
    responses(
        (status = 200, description = "The setting at this unit, now inherited.", body = Data<UnitSettingResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission (`permission_denied`), it does not cover this unit (`org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 404, description = "No such unit in your scope (`organizations.unit_not_found`), or no such setting (`organizations.setting_unknown`).", body = Problem),
    )
)]
async fn clear_unit_setting(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, key)): Path<(Uuid, String)>,
) -> Result<Data<UnitSettingResponse>, Error> {
    principal.require(&SETTINGS_WRITE)?;
    let setting = service.set_setting(&tenant, &principal, id, &key, None).await?;
    Ok(Data { data: setting.into() })
}
