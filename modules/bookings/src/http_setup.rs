//! Endpoint REST cấu hình đặt lịch, dưới `/api/v1/bookings`: dịch vụ, nhân viên / phòng / thiết
//! bị, lịch tuần, ngoại lệ. Handler chỉ: lấy dữ liệu vào → gọi service → trả envelope.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::entity::{exception, resource, schedule};
use crate::service::BookingsService;
use crate::setup::{
    Deposit, NewException, NewResource, NewService, ResourceChanges, ServiceChanges, ServiceView,
    WeeklyInput,
};

pub(crate) fn routes(service: Arc<BookingsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_services, create_service))
        .routes(routes!(get_service, update_service))
        .routes(routes!(list_resources, create_resource))
        .routes(routes!(get_resource, update_resource))
        .routes(routes!(get_schedule, replace_schedule))
        .routes(routes!(list_exceptions, create_exception))
        .routes(routes!(delete_exception))
        .with_state(service)
}

/// How much deposit a booking asks for, paid through the shop's payment partner.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BookingDeposit {
    /// `none`, `percent` (of the price) or `amount` (fixed, in the price's currency).
    #[schema(example = "percent")]
    kind: String,
    /// Percent 1–100, or the amount in minor units; `0` for `none`.
    #[serde(default)]
    #[schema(example = 30)]
    value: i64,
}

impl BookingDeposit {
    fn from_deposit(deposit: Deposit) -> Self {
        BookingDeposit { kind: deposit.kind().to_owned(), value: deposit.value() }
    }

    fn into_deposit(self) -> Result<Deposit, Error> {
        match self.kind.as_str() {
            "none" => Ok(Deposit::None),
            "percent" => Ok(Deposit::Percent(self.value)),
            "amount" => Ok(Deposit::Amount(self.value)),
            _ => Err(Error::validation(vec![FieldError::new(
                "deposit.kind",
                "invalid",
                "must be `none`, `percent` or `amount`",
            )])),
        }
    }
}

/// A new bookable service.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateBookingServiceRequest {
    /// The unit that offers it; you need `bookings.services.manage` there. Omitted: your only unit,
    /// or the tenant's root.
    org_id: Option<Uuid>,
    /// 1 to 200 characters.
    #[schema(example = "Haircut")]
    name: String,
    /// The name in other languages, e.g. `{"fr": "Coupe"}`.
    #[serde(default)]
    names: BTreeMap<String, String>,
    /// Up to 2000 characters. Never put health information here.
    #[serde(default)]
    description: String,
    /// 5 to 1440, in steps of 5.
    #[schema(example = 60)]
    duration_minutes: i32,
    /// Time the resource needs before the customer arrives (setup), 0 to 240 in steps of 5.
    #[serde(default)]
    buffer_before_minutes: i32,
    /// Time the resource needs after (cleaning), 0 to 240 in steps of 5.
    #[serde(default)]
    buffer_after_minutes: i32,
    price: Money,
    /// Omitted: no deposit.
    deposit: Option<BookingDeposit>,
    /// People per time slot: 1 for a haircut, more for a class. Default 1.
    #[serde(default = "one")]
    capacity: i32,
    /// Staff (or courts, rooms…) that can give the service; one of them is booked.
    performers: Vec<Uuid>,
    /// Rooms or equipment every booking also needs.
    #[serde(default)]
    required: Vec<Uuid>,
}

fn one() -> i32 {
    1
}

/// Một trường có mặt trong thân (kể cả `null`) → `Some(…)`; vắng → `None`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Changes to a service; omitted fields stay as they are.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateBookingServiceRequest {
    name: Option<String>,
    names: Option<BTreeMap<String, String>>,
    description: Option<String>,
    duration_minutes: Option<i32>,
    buffer_before_minutes: Option<i32>,
    buffer_after_minutes: Option<i32>,
    /// Same currency as before.
    price: Option<Money>,
    deposit: Option<BookingDeposit>,
    capacity: Option<i32>,
    /// `false` stops new bookings; existing bookings stay.
    active: Option<bool>,
    performers: Option<Vec<Uuid>>,
    required: Option<Vec<Uuid>>,
}

/// A bookable service.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BookingService {
    id: Uuid,
    org_id: Uuid,
    name: String,
    names: BTreeMap<String, String>,
    description: String,
    duration_minutes: i32,
    buffer_before_minutes: i32,
    buffer_after_minutes: i32,
    price: Money,
    deposit: BookingDeposit,
    capacity: i32,
    active: bool,
    performers: Vec<Uuid>,
    required: Vec<Uuid>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl BookingService {
    pub(crate) fn from_view(view: ServiceView) -> Result<Self, Error> {
        let price = view.price()?;
        let deposit = BookingDeposit::from_deposit(view.deposit());
        let names = view.names();
        let row = view.row;
        Ok(BookingService {
            id: row.id,
            org_id: row.org_id,
            name: row.name,
            names,
            description: row.description,
            duration_minutes: row.duration_minutes,
            buffer_before_minutes: row.buffer_before_minutes,
            buffer_after_minutes: row.buffer_after_minutes,
            price,
            deposit,
            capacity: row.capacity,
            active: row.active,
            performers: view.performers,
            required: view.required,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// A new staff member, room or equipment.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateBookingResourceRequest {
    /// The unit it belongs to; you need `bookings.services.manage` there.
    org_id: Option<Uuid>,
    /// `staff`, `room` or `equipment`. Staff need a weekly schedule to be bookable; rooms and
    /// equipment without one are always open.
    #[schema(example = "staff")]
    kind: String,
    /// 1 to 200 characters, shown to customers.
    #[schema(example = "Lan")]
    name: String,
    /// Staff only: the user account of this person, so they see their own appointments with
    /// `bookings.staff`.
    user_id: Option<Uuid>,
}

/// Changes to a resource.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateBookingResourceRequest {
    name: Option<String>,
    /// Staff only; `null` unlinks the account.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    user_id: Option<Option<Uuid>>,
    /// `false`: not offered for new bookings.
    active: Option<bool>,
}

/// A staff member, room or equipment.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BookingResource {
    id: Uuid,
    org_id: Uuid,
    kind: String,
    name: String,
    user_id: Option<Uuid>,
    active: bool,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<resource::Model> for BookingResource {
    fn from(row: resource::Model) -> Self {
        BookingResource {
            id: row.id,
            org_id: row.org_id,
            kind: row.kind,
            name: row.name,
            user_id: row.user_id,
            active: row.active,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A weekly working range, in the local time of the unit (setting `organizations.timezone`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BookingWeeklyRange {
    /// 1 (Monday) to 7 (Sunday).
    #[schema(example = 1)]
    weekday: i16,
    /// Minutes after local midnight, in steps of 5: 540 is 09:00.
    #[schema(example = 540)]
    start_minute: i32,
    /// Minutes after local midnight, up to 1440: 1020 is 17:00.
    #[schema(example = 1020)]
    end_minute: i32,
}

impl From<schedule::Model> for BookingWeeklyRange {
    fn from(row: schedule::Model) -> Self {
        BookingWeeklyRange {
            weekday: row.weekday,
            start_minute: row.start_minute,
            end_minute: row.end_minute,
        }
    }
}

/// The whole weekly schedule of a resource.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplaceBookingScheduleRequest {
    /// At most 70 ranges; ranges of the same day must not overlap. Empty: never available (staff).
    ranges: Vec<BookingWeeklyRange>,
}

/// The weekly schedule of a resource.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BookingSchedule {
    resource_id: Uuid,
    ranges: Vec<BookingWeeklyRange>,
}

/// Time off or extra working hours of a resource.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateBookingExceptionRequest {
    resource_id: Uuid,
    /// `closed` (time off, a holiday) or `open` (an extra shift).
    #[schema(example = "closed")]
    kind: String,
    starts_at: Timestamp,
    /// After `starts_at`, at most 366 days later.
    ends_at: Timestamp,
    /// Up to 200 characters, e.g. "Lunar New Year holiday".
    #[serde(default)]
    reason: String,
}

/// Time off or extra working hours.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BookingException {
    id: Uuid,
    resource_id: Uuid,
    kind: String,
    starts_at: Timestamp,
    ends_at: Timestamp,
    reason: String,
    created_at: Timestamp,
}

impl From<exception::Model> for BookingException {
    fn from(row: exception::Model) -> Self {
        BookingException {
            id: row.id,
            resource_id: row.resource_id,
            kind: row.kind,
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            reason: row.reason,
            created_at: row.created_at,
        }
    }
}

/// Which exceptions to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ExceptionsQuery {
    /// The resource.
    resource_id: Uuid,
    /// From this moment; default now.
    from: Option<Timestamp>,
    /// Until this moment; default 90 days after `from`.
    to: Option<Timestamp>,
}

/// List bookable services.
///
/// Requires `bookings.services.manage`. Every service in your units, active or not.
#[utoipa::path(
    get,
    path = "/services",
    extensions(("x-permissions" = json!(["bookings.services.manage"]))),
    operation_id = "list_booking_services",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of services.", body = Page<BookingService>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.services.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "The paging parameters are invalid; see `errors`.", body = Problem),
    )
)]
async fn list_services(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<BookingService>, Error> {
    // Quyền trước tham số trang: thiếu quyền là 403, không phải 422 (luật `permission-first` của xtask).
    principal.require(&crate::permissions::SERVICES_MANAGE)?;
    let page = PageRequest::try_from(query)?;
    let page = service.list_services(&tenant, &principal, &page).await?;
    let data = page.data.into_iter().map(BookingService::from_view).collect::<Result<_, _>>()?;
    Ok(Page { data, pagination: page.pagination })
}

/// Create a bookable service.
///
/// Requires `bookings.services.manage` at the unit. Every problem with the input is reported at once.
#[utoipa::path(
    post,
    path = "/services",
    extensions(("x-permissions" = json!(["bookings.services.manage"]))),
    operation_id = "create_booking_service",
    security(("bearer" = [])),
    request_body = CreateBookingServiceRequest,
    responses(
        (status = 201, description = "Created; `Location` is the new service.", body = Data<BookingService>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.services.manage` at the unit (`permission_denied`, `org_out_of_scope`).", body = Problem),
        (status = 422, description = "Invalid values; see `errors`. Code `bookings.org_required` when you work in several units and gave no `org_id`.", body = Problem),
    )
)]
async fn create_service(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateBookingServiceRequest>,
) -> Result<Created<BookingService>, Error> {
    let deposit = input.deposit.map_or(Ok(Deposit::None), BookingDeposit::into_deposit)?;
    let new = NewService {
        org_id: input.org_id,
        name: input.name,
        names: input.names,
        description: input.description,
        duration_minutes: input.duration_minutes,
        buffer_before_minutes: input.buffer_before_minutes,
        buffer_after_minutes: input.buffer_after_minutes,
        price: input.price,
        deposit,
        capacity: input.capacity,
        performers: input.performers,
        required: input.required,
    };
    let view = service.create_service(&tenant, &principal, new).await?;
    Ok(Created::new(
        format!("/bookings/services/{}", view.row.id),
        BookingService::from_view(view)?,
    ))
}

/// Get a bookable service.
///
/// Requires `bookings.services.manage` at the service's unit.
#[utoipa::path(
    get,
    path = "/services/{id}",
    extensions(("x-permissions" = json!(["bookings.services.manage"]))),
    operation_id = "get_booking_service",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the service.")),
    responses(
        (status = 200, description = "The service.", body = Data<BookingService>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.services.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such service in your units; code `bookings.service_not_found`.", body = Problem),
    )
)]
async fn get_service(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BookingService>, Error> {
    let view = service.get_service(&tenant, &principal, id).await?;
    Ok(Data { data: BookingService::from_view(view)? })
}

/// Change a bookable service.
///
/// Requires `bookings.services.manage` at the service's unit. Bookings already made keep their
/// time, price and deposit.
#[utoipa::path(
    patch,
    path = "/services/{id}",
    extensions(("x-permissions" = json!(["bookings.services.manage"]))),
    operation_id = "update_booking_service",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the service.")),
    request_body = UpdateBookingServiceRequest,
    responses(
        (status = 200, description = "The service as saved.", body = Data<BookingService>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.services.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such service in your units; code `bookings.service_not_found`.", body = Problem),
        (status = 422, description = "Invalid values; see `errors`.", body = Problem),
    )
)]
async fn update_service(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<UpdateBookingServiceRequest>,
) -> Result<Data<BookingService>, Error> {
    let changes = ServiceChanges {
        name: input.name,
        names: input.names,
        description: input.description,
        duration_minutes: input.duration_minutes,
        buffer_before_minutes: input.buffer_before_minutes,
        buffer_after_minutes: input.buffer_after_minutes,
        price: input.price,
        deposit: input.deposit.map(BookingDeposit::into_deposit).transpose()?,
        capacity: input.capacity,
        active: input.active,
        performers: input.performers,
        required: input.required,
    };
    let view = service.update_service(&tenant, &principal, id, changes).await?;
    Ok(Data { data: BookingService::from_view(view)? })
}

/// List staff, rooms and equipment.
///
/// Requires `bookings.services.manage`.
#[utoipa::path(
    get,
    path = "/resources",
    extensions(("x-permissions" = json!(["bookings.services.manage"]))),
    operation_id = "list_booking_resources",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of resources.", body = Page<BookingResource>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.services.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "The paging parameters are invalid; see `errors`.", body = Problem),
    )
)]
async fn list_resources(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<BookingResource>, Error> {
    // Quyền trước tham số trang: thiếu quyền là 403, không phải 422 (luật `permission-first` của xtask).
    principal.require(&crate::permissions::SERVICES_MANAGE)?;
    let page = PageRequest::try_from(query)?;
    Ok(service.list_resources(&tenant, &principal, &page).await?.map(BookingResource::from))
}

/// Add a staff member, room or equipment.
///
/// Requires `bookings.services.manage` at the unit.
#[utoipa::path(
    post,
    path = "/resources",
    extensions(("x-permissions" = json!(["bookings.services.manage"]))),
    operation_id = "create_booking_resource",
    security(("bearer" = [])),
    request_body = CreateBookingResourceRequest,
    responses(
        (status = 201, description = "Created; `Location` is the new resource.", body = Data<BookingResource>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.services.manage` at the unit.", body = Problem),
        (status = 422, description = "Invalid values; see `errors`.", body = Problem),
    )
)]
async fn create_resource(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateBookingResourceRequest>,
) -> Result<Created<BookingResource>, Error> {
    let new = NewResource {
        org_id: input.org_id,
        kind: input.kind,
        name: input.name,
        user_id: input.user_id,
    };
    let row = service.create_resource(&tenant, &principal, new).await?;
    Ok(Created::new(format!("/bookings/resources/{}", row.id), BookingResource::from(row)))
}

/// Get a staff member, room or equipment.
///
/// Requires `bookings.services.manage` at its unit.
#[utoipa::path(
    get,
    path = "/resources/{id}",
    extensions(("x-permissions" = json!(["bookings.services.manage"]))),
    operation_id = "get_booking_resource",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the resource.")),
    responses(
        (status = 200, description = "The resource.", body = Data<BookingResource>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.services.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such resource in your units; code `bookings.resource_not_found`.", body = Problem),
    )
)]
async fn get_resource(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BookingResource>, Error> {
    Ok(Data { data: service.get_resource(&tenant, &principal, id).await?.into() })
}

/// Change a staff member, room or equipment.
///
/// Requires `bookings.services.manage` at its unit.
#[utoipa::path(
    patch,
    path = "/resources/{id}",
    extensions(("x-permissions" = json!(["bookings.services.manage"]))),
    operation_id = "update_booking_resource",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the resource.")),
    request_body = UpdateBookingResourceRequest,
    responses(
        (status = 200, description = "The resource as saved.", body = Data<BookingResource>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.services.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such resource in your units; code `bookings.resource_not_found`.", body = Problem),
        (status = 422, description = "Invalid values; see `errors`.", body = Problem),
    )
)]
async fn update_resource(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<UpdateBookingResourceRequest>,
) -> Result<Data<BookingResource>, Error> {
    let changes =
        ResourceChanges { name: input.name, user_id: input.user_id, active: input.active };
    Ok(Data { data: service.update_resource(&tenant, &principal, id, changes).await?.into() })
}

/// Get the weekly schedule of a resource.
///
/// Requires `bookings.services.manage` at its unit. Times are local to the unit.
#[utoipa::path(
    get,
    path = "/resources/{id}/schedule",
    extensions(("x-permissions" = json!(["bookings.services.manage"]))),
    operation_id = "get_booking_schedule",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the resource.")),
    responses(
        (status = 200, description = "The weekly schedule.", body = Data<BookingSchedule>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.services.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such resource in your units; code `bookings.resource_not_found`.", body = Problem),
    )
)]
async fn get_schedule(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BookingSchedule>, Error> {
    let rows = service.schedule(&tenant, &principal, id).await?;
    Ok(Data {
        data: BookingSchedule {
            resource_id: id,
            ranges: rows.into_iter().map(Into::into).collect(),
        },
    })
}

/// Replace the weekly schedule of a resource.
///
/// Requires `bookings.services.manage` at its unit. Times are local to the unit (setting
/// `organizations.timezone`), so a change of time zone keeps "9:00 to 17:00" at 9:00 local time.
/// Bookings already made keep their time.
#[utoipa::path(
    put,
    path = "/resources/{id}/schedule",
    extensions(("x-permissions" = json!(["bookings.services.manage"]))),
    operation_id = "replace_booking_schedule",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the resource.")),
    request_body = ReplaceBookingScheduleRequest,
    responses(
        (status = 200, description = "The weekly schedule as saved.", body = Data<BookingSchedule>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.services.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such resource in your units; code `bookings.resource_not_found`.", body = Problem),
        (status = 422, description = "Invalid ranges; see `errors`.", body = Problem),
    )
)]
async fn replace_schedule(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ReplaceBookingScheduleRequest>,
) -> Result<Data<BookingSchedule>, Error> {
    let ranges = input
        .ranges
        .into_iter()
        .map(|range| WeeklyInput {
            weekday: range.weekday,
            start_minute: range.start_minute,
            end_minute: range.end_minute,
        })
        .collect();
    let rows = service.replace_schedule(&tenant, &principal, id, ranges).await?;
    Ok(Data {
        data: BookingSchedule {
            resource_id: id,
            ranges: rows.into_iter().map(Into::into).collect(),
        },
    })
}

/// List time off and extra hours of a resource.
///
/// Requires `bookings.services.manage` at its unit. Exceptions that touch the range, by start time
/// (earliest first) unless `order=desc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/exceptions",
    extensions(("x-permissions" = json!(["bookings.services.manage"]))),
    operation_id = "list_booking_exceptions",
    security(("bearer" = [])),
    params(ExceptionsQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the exceptions, by start time.", body = Page<BookingException>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.services.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such resource in your units; code `bookings.resource_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_exceptions(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ExceptionsQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<BookingException>, Error> {
    // Kiểm quyền trước khi đọc tham số trang: người không có quyền nhận 403, không phải 422 (D-20261126-list-pages-cluster-2).
    principal.require(&crate::permissions::SERVICES_MANAGE)?;
    let page = PageRequest::try_from(page)?;
    let from = query.from.unwrap_or_else(Timestamp::now);
    let to = query.to.unwrap_or(from + SignedDuration::from_hours(24 * 90));
    let rows = service.exceptions(&tenant, &principal, query.resource_id, from, to, &page).await?;
    Ok(rows.map(Into::into))
}

/// Add time off or extra hours.
///
/// Requires `bookings.services.manage` at the resource's unit. Bookings already made in the time
/// off stay; cancel or move them if needed.
#[utoipa::path(
    post,
    path = "/exceptions",
    extensions(("x-permissions" = json!(["bookings.services.manage"]))),
    operation_id = "create_booking_exception",
    security(("bearer" = [])),
    request_body = CreateBookingExceptionRequest,
    responses(
        (status = 201, description = "Created.", body = Data<BookingException>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.services.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such resource in your units; code `bookings.resource_not_found`.", body = Problem),
        (status = 422, description = "Invalid values; see `errors`.", body = Problem),
    )
)]
async fn create_exception(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateBookingExceptionRequest>,
) -> Result<Created<BookingException>, Error> {
    let new = NewException {
        resource_id: input.resource_id,
        kind: input.kind,
        starts_at: input.starts_at,
        ends_at: input.ends_at,
        reason: input.reason,
    };
    let row = service.create_exception(&tenant, &principal, new).await?;
    Ok(Created::new(format!("/bookings/exceptions/{}", row.id), BookingException::from(row)))
}

/// Remove time off or extra hours.
///
/// Requires `bookings.services.manage` at the resource's unit.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/exceptions/{id}",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["bookings.services.manage"]))),
    operation_id = "delete_booking_exception",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the exception.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.services.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such exception in your units; code `bookings.exception_not_found`.", body = Problem),
    )
)]
async fn delete_exception(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_exception(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
