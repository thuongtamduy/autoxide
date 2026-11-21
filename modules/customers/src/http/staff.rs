//! Endpoint của nhân viên về hồ sơ khách: danh sách, đăng ký tại quầy, một hồ sơ, sửa, liên hệ đầy
//! đủ (có audit), lịch sử mua.

use autoxide_kernel::time::Date;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Map, Value};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::views::{CustomerContact, CustomerOrder, CustomerResponse, try_page};
use crate::permissions::{CONTACT_REVEAL, MANAGE, READ};
use crate::profiles::{Details, ProfileFilter, Registration};
use crate::service::CustomersService;

/// Filters of the customer list; all optional and combined with AND.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct CustomerQuery {
    /// A name (accents do not matter), a phone number (any common form) or an email.
    q: Option<String>,
    /// Only members of this customer group.
    group_id: Option<Uuid>,
    /// Only this membership tier code.
    tier: Option<String>,
    /// Only customers with one of these tags; comma-separated tag ids.
    tags: Option<String>,
    /// Only customers of this unit (not the units below it).
    org_id: Option<Uuid>,
}

impl CustomerQuery {
    fn filter(self) -> Result<ProfileFilter, Error> {
        let tag_ids = match self.tags.as_deref().filter(|text| !text.is_empty()) {
            Some(text) => text
                .split(',')
                .map(|part| {
                    part.trim().parse::<Uuid>().map_err(|_| {
                        Error::validation(vec![autoxide_kernel::error::FieldError::new(
                            "tags",
                            "invalid",
                            "must be tag ids separated by commas",
                        )])
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
        };
        Ok(ProfileFilter {
            q: self.q,
            group_id: self.group_id,
            tier: self.tier,
            tag_ids,
            org_id: self.org_id,
        })
    }
}

/// List the customers in your scope.
///
/// Requires `customers.read`; its scope decides which units' customers you see. Phone and email are
/// masked. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/",
    extensions(("x-permissions" = json!(["customers.read"]))),
    operation_id = "list_customers",
    security(("bearer" = [])),
    params(CustomerQuery, PageQuery),
    responses(
        (status = 200, description = "One page of customers.", body = Page<CustomerResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_customers(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<CustomerQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<CustomerResponse>, Error> {
    principal.require(&READ)?;
    let filter = query.filter()?;
    let page = service.list(&tenant, &principal, &filter, &PageRequest::try_from(page)?).await?;
    try_page(page, CustomerResponse::new)
}

/// A customer registered at the counter.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = RegisterCustomerRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegisterRequest {
    /// Phone number, e.g. `0912345678` or `+84912345678`.
    phone: String,
    /// Unit where the customer registers; you need `customers.manage` there. Omitted: the root
    /// unit.
    org_id: Option<Uuid>,
    /// Up to 200 characters.
    full_name: Option<String>,
    email: Option<String>,
    birthday: Option<Date>,
    /// `female`, `male` or `other`.
    gender: Option<String>,
    /// Values of the shop's custom fields for customers.
    #[schema(value_type = Option<Object>)]
    custom_fields: Option<Map<String, Value>>,
}

/// Register a customer at the counter by phone number.
///
/// Requires `customers.manage` at the unit. The profile belongs to no account until the owner of
/// the number signs in and proves it.
#[utoipa::path(
    post,
    path = "/",
    extensions(("x-permissions" = json!(["customers.manage"]))),
    operation_id = "register_customer",
    security(("bearer" = [])),
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "Registered.", body = Data<CustomerResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.manage` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or not active (`org_frozen`).", body = Problem),
        (status = 409, description = "The phone number already has a profile; code `customers.phone_taken` with its `id`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn register_customer(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<RegisterRequest>,
) -> Result<Created<CustomerResponse>, Error> {
    principal.require(&MANAGE)?;
    let registration = Registration {
        phone: input.phone,
        org_id: input.org_id,
        details: Details {
            full_name: input.full_name,
            email: input.email,
            birthday: input.birthday,
            gender: input.gender,
            custom_fields: input.custom_fields,
        },
    };
    let row = service.register(&tenant, &principal, registration).await?;
    let version = row.updated_at;
    let mut created = Created::new(format!("/customers/{}", row.id), CustomerResponse::new(row)?);
    created.version = Some(version);
    Ok(created)
}

/// Get a customer.
///
/// Requires `customers.read` covering the customer's unit. Phone and email are masked.
#[utoipa::path(
    get,
    path = "/{id}",
    extensions(("x-permissions" = json!(["customers.read"]))),
    operation_id = "get_customer",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the customer profile.")),
    responses(
        (status = 200, description = "The customer, with an `ETag` for `If-Match`.", body = Data<CustomerResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No customer with this id in your scope; code `customers.profile_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_customer(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<CustomerResponse>, Error> {
    principal.require(&READ)?;
    let row = service.get(&tenant, &principal, id).await?;
    Ok(Tagged { version: row.updated_at, data: CustomerResponse::new(row)? })
}

/// Changes to a customer. Omitted fields stay as they are; an empty string clears a text.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = UpdateCustomerProfileRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateRequest {
    full_name: Option<String>,
    email: Option<String>,
    birthday: Option<Date>,
    /// `female`, `male`, `other` or an empty string.
    gender: Option<String>,
    /// Custom field values to set; `null` clears a field.
    #[schema(value_type = Option<Object>)]
    custom_fields: Option<Map<String, Value>>,
}

/// Change a customer.
///
/// Requires `customers.manage` covering the customer's unit. Send `If-Match` with the `ETag` of
/// the last read to avoid overwriting someone else's change.
#[utoipa::path(
    put,
    path = "/{id}",
    extensions(("x-permissions" = json!(["customers.manage"]))),
    operation_id = "update_customer",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the customer profile.")),
    request_body = UpdateRequest,
    responses(
        (status = 200, description = "Changed.", body = Data<CustomerResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No customer with this id in your scope; code `customers.profile_not_found`.", body = Problem),
        (status = 412, description = "Changed by someone else since your `If-Match` version; code `version_mismatch`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn update_customer(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateRequest>,
) -> Result<Tagged<CustomerResponse>, Error> {
    principal.require(&MANAGE)?;
    let details = Details {
        full_name: input.full_name,
        email: input.email,
        birthday: input.birthday,
        gender: input.gender,
        custom_fields: input.custom_fields,
    };
    let row = service.update(&tenant, &principal, id, details, if_match.expected()?).await?;
    Ok(Tagged { version: row.updated_at, data: CustomerResponse::new(row)? })
}

/// Why the contact details are needed.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct RevealQuery {
    /// Optional reason kept in the audit log, up to 200 characters.
    reason: Option<String>,
}

/// Show the full phone number and email of a customer.
///
/// Requires `customers.contact.reveal` covering the customer's unit. Every call is written to the
/// audit log (who looked, which customer, the reason).
#[utoipa::path(
    get,
    path = "/{id}/contact",
    extensions(("x-permissions" = json!(["customers.contact.reveal"]))),
    operation_id = "reveal_customer_contact",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the customer profile."), RevealQuery),
    responses(
        (status = 200, description = "Full contact details.", body = Data<CustomerContact>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.contact.reveal`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No customer with this id in your scope; code `customers.profile_not_found`.", body = Problem),
    )
)]
pub(crate) async fn reveal_contact(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<RevealQuery>,
) -> Result<Data<CustomerContact>, Error> {
    principal.require(&CONTACT_REVEAL)?;
    let contact = service.reveal_contact(&tenant, &principal, id, query.reason).await?;
    Ok(Data { data: CustomerContact { phone: contact.phone, email: contact.email } })
}

/// List the orders of a customer.
///
/// Requires `customers.read` covering the customer's unit. Built from order events; newest first
/// unless `order=asc`.
#[utoipa::path(
    get,
    path = "/{id}/orders",
    extensions(("x-permissions" = json!(["customers.read"]))),
    operation_id = "list_customer_orders",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the customer profile."), PageQuery),
    responses(
        (status = 200, description = "One page of orders.", body = Page<CustomerOrder>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No customer with this id in your scope; code `customers.profile_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_customer_orders(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<CustomerOrder>, Error> {
    principal.require(&READ)?;
    let page = service.orders_of(&tenant, &principal, id, &PageRequest::try_from(page)?).await?;
    try_page(page, CustomerOrder::new)
}
