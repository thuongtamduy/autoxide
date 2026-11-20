//! Endpoint của người mua về hồ sơ của chính mình (`/me`): chỉ cần đăng nhập, không cần permission
//! (như `orders`); liên hệ không bị che.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Query, State};
use chrono::NaiveDate;
use serde::Deserialize;
use utoipa::ToSchema;
use validator::Validate;

use super::views::{CustomerOrder, MyCustomerProfile, try_page};
use crate::profiles::Details;
use crate::service::CustomersService;

/// Get your customer profile (created on first visit).
///
/// Any signed-in buyer. When your account has a verified phone number that the shop already knows
/// from orders or the counter, that history becomes yours.
#[utoipa::path(
    get,
    path = "/me",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_customer_profile",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Your profile.", body = Data<MyCustomerProfile>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
pub(crate) async fn get_me(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<MyCustomerProfile>, Error> {
    let row = service.me(&tenant, principal.user_id()).await?;
    Ok(Data { data: MyCustomerProfile::new(row)? })
}

/// Changes to your profile. Omitted fields stay as they are; an empty string clears a text.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = UpdateMyCustomerProfileRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateMeRequest {
    /// Up to 200 characters.
    full_name: Option<String>,
    email: Option<String>,
    birthday: Option<NaiveDate>,
    /// `female`, `male`, `other` or an empty string.
    gender: Option<String>,
}

/// Change your customer profile.
///
/// Any signed-in buyer. The phone number comes from your account's verified number and cannot be
/// changed here.
#[utoipa::path(
    put,
    path = "/me",
    extensions(("x-permissions" = json!([]))),
    operation_id = "update_my_customer_profile",
    security(("bearer" = [])),
    request_body = UpdateMeRequest,
    responses(
        (status = 200, description = "Changed.", body = Data<MyCustomerProfile>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn update_me(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<UpdateMeRequest>,
) -> Result<Data<MyCustomerProfile>, Error> {
    let details = Details {
        full_name: input.full_name,
        email: input.email,
        birthday: input.birthday,
        gender: input.gender,
        custom_fields: None,
    };
    let row = service.update_me(&tenant, principal.user_id(), details).await?;
    Ok(Data { data: MyCustomerProfile::new(row)? })
}

/// List your orders as the shop's customer history sees them.
///
/// Any signed-in buyer; newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/me/orders",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_customer_orders",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of your orders.", body = Page<CustomerOrder>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_my_orders(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<CustomerOrder>, Error> {
    let row = service.me(&tenant, principal.user_id()).await?;
    let page = service.order_page(&tenant, row.id, &PageRequest::try_from(page)?).await?;
    try_page(page, CustomerOrder::new)
}
