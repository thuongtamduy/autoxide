//! Sổ địa chỉ: người mua (`/me/addresses…`, chỉ địa chỉ của mình — mọi tài khoản đã đăng nhập, kể
//! cả tài khoản người mua vốn không có permission) và nhân viên chăm sóc khách
//! (`/buyers/{buyer_id}/addresses`, `shipping.addresses.read`).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::addresses::{AddressDraft, AddressPatch, PostalInput};
use crate::api::{Address, BuyerAddress};
use crate::permissions::ADDRESSES_READ;
use crate::service::ShippingService;

/// A postal address. In Vietnam: `province_code` and `commune_code` from the official list of
/// administrative units (`GET /api/v1/address/provinces`). Elsewhere: `province_code` is the
/// region (state, province…), `city` the city, with an optional `postal_code`.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[schema(as = ShippingPostalAddress)]
#[serde(deny_unknown_fields)]
pub(crate) struct PostalAddressRequest {
    /// ISO 3166-1 alpha-2 country code.
    #[schema(example = "VN")]
    pub country: String,
    /// Vietnam: code of the province. Elsewhere: the region.
    #[schema(example = "01")]
    pub province_code: String,
    /// Vietnam: code of the commune, ward or special zone (required).
    #[schema(example = "00004")]
    #[serde(default)]
    pub commune_code: Option<String>,
    /// Outside Vietnam: the city (required).
    #[serde(default)]
    pub city: Option<String>,
    /// House number, street, village… (1 to 300 characters).
    #[schema(example = "12 Trang Tien")]
    pub line: String,
    /// Postal code, mostly for addresses outside Vietnam.
    #[serde(default)]
    pub postal_code: Option<String>,
}

impl From<PostalAddressRequest> for PostalInput {
    fn from(input: PostalAddressRequest) -> Self {
        PostalInput {
            country: input.country,
            province_code: input.province_code,
            commune_code: input.commune_code,
            city: input.city,
            line: input.line,
            postal_code: input.postal_code,
        }
    }
}

/// A new address in the address book.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = NewShippingAddress)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewAddressRequest {
    /// Your name for it, e.g. `Home` (up to 100 characters).
    #[serde(default)]
    label: String,
    /// Who receives the parcel (1 to 200 characters).
    recipient_name: String,
    /// Phone number of the recipient, for the carrier.
    #[schema(example = "0912345678")]
    phone: String,
    address: PostalAddressRequest,
    /// Make it the default address. The first address is always the default.
    #[serde(default)]
    is_default: bool,
}

/// Fields to change; leave out what stays.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ShippingAddressChanges)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddressChangesRequest {
    label: Option<String>,
    recipient_name: Option<String>,
    phone: Option<String>,
    address: Option<PostalAddressRequest>,
}

/// An address of a buyer's address book.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingAddress)]
pub(crate) struct AddressResponse {
    id: Uuid,
    buyer_id: Uuid,
    label: String,
    recipient_name: String,
    phone: String,
    /// The address, with the names of the province and commune when it was saved. Outside
    /// Vietnam: `province_code` / `province_name` is the region, `commune_name` the city and
    /// `commune_code` is empty.
    address: Address,
    postal_code: Option<String>,
    is_default: bool,
    updated_at: Timestamp,
}

impl From<BuyerAddress> for AddressResponse {
    fn from(entry: BuyerAddress) -> Self {
        AddressResponse {
            id: entry.id,
            buyer_id: entry.buyer_id,
            label: entry.label,
            recipient_name: entry.recipient_name,
            phone: entry.phone,
            address: entry.address,
            postal_code: entry.postal_code,
            is_default: entry.is_default,
            updated_at: entry.updated_at,
        }
    }
}

fn page_of(page: Page<BuyerAddress>) -> Page<AddressResponse> {
    page.map(AddressResponse::from)
}

/// List your saved delivery addresses.
///
/// Any signed-in account (customer accounts too); only your own addresses.
#[utoipa::path(
    get,
    path = "/me/addresses",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_shipping_addresses",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of your addresses.", body = Page<AddressResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_my_addresses(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<AddressResponse>, Error> {
    let page = PageRequest::try_from(page)?;
    Ok(page_of(service.list_addresses(&tenant, principal.user_id(), &page).await?))
}

/// Save a delivery address.
///
/// Any signed-in account. An address in Vietnam is checked against the official list of
/// administrative units (the commune must belong to the province and be in force). Retry safely
/// with an `Idempotency-Key`.
#[utoipa::path(
    post,
    path = "/me/addresses",
    extensions(("x-permissions" = json!([]))),
    operation_id = "create_my_shipping_address",
    security(("bearer" = [])),
    request_body = NewAddressRequest,
    responses(
        (status = 201, description = "Saved.", body = Data<AddressResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 409, description = "The address book is full; code `shipping.address_limit`.", body = Problem),
        (status = 422, description = "The address or contact is not valid; code `shipping.invalid_address`, see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_my_address(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<NewAddressRequest>,
) -> Result<Created<AddressResponse>, Error> {
    let draft = AddressDraft {
        label: input.label,
        recipient_name: input.recipient_name,
        phone: input.phone,
        address: input.address.into(),
        is_default: input.is_default,
    };
    let saved = service.create_address(&tenant, principal.user_id(), draft).await?;
    Ok(Created::new(format!("/shipping/me/addresses/{}", saved.id), saved.into()))
}

/// Get one of your saved addresses.
#[utoipa::path(
    get,
    path = "/me/addresses/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_shipping_address",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the address.")),
    responses(
        (status = 200, description = "The address.", body = Data<AddressResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no address with this id; code `shipping.address_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_my_address(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<AddressResponse>, Error> {
    let entry = service.get_address(&tenant, principal.user_id(), id).await?;
    Ok(Data { data: entry.into() })
}

/// Change one of your saved addresses.
///
/// Shipments already created keep the address they were created with.
#[utoipa::path(
    patch,
    path = "/me/addresses/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "update_my_shipping_address",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the address.")),
    request_body = AddressChangesRequest,
    responses(
        (status = 200, description = "The address after the change.", body = Data<AddressResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no address with this id; code `shipping.address_not_found`.", body = Problem),
        (status = 422, description = "The address or contact is not valid; code `shipping.invalid_address`, see `errors`.", body = Problem),
    )
)]
pub(crate) async fn update_my_address(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AddressChangesRequest>,
) -> Result<Data<AddressResponse>, Error> {
    let patch = AddressPatch {
        label: input.label,
        recipient_name: input.recipient_name,
        phone: input.phone,
        address: input.address.map(PostalInput::from),
    };
    let entry = service.update_address(&tenant, principal.user_id(), id, patch).await?;
    Ok(Data { data: entry.into() })
}

/// Delete one of your saved addresses.
///
/// Deleting the default address makes the most recently changed other address the default.
#[utoipa::path(
    delete,
    path = "/me/addresses/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "delete_my_shipping_address",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the address.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no address with this id; code `shipping.address_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_my_address(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_address(&tenant, principal.user_id(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Make one of your saved addresses the default.
///
/// Exactly one address is the default, also when two requests run at the same time.
#[utoipa::path(
    post,
    path = "/me/addresses/{id}/default",
    extensions(("x-permissions" = json!([]))),
    operation_id = "set_my_default_shipping_address",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the address.")),
    responses(
        (status = 200, description = "The new default address.", body = Data<AddressResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no address with this id; code `shipping.address_not_found`.", body = Problem),
    )
)]
pub(crate) async fn set_my_default_address(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<AddressResponse>, Error> {
    let entry = service.set_default_address(&tenant, principal.user_id(), id).await?;
    Ok(Data { data: entry.into() })
}

/// List the saved addresses of a buyer.
///
/// For customer care. Needs `shipping.addresses.read`.
#[utoipa::path(
    get,
    path = "/buyers/{buyer_id}/addresses",
    extensions(("x-permissions" = json!(["shipping.addresses.read"]))),
    operation_id = "list_buyer_shipping_addresses",
    security(("bearer" = [])),
    params(("buyer_id" = Uuid, Path, description = "Id of the buyer's account."), PageQuery),
    responses(
        (status = 200, description = "One page of the buyer's addresses.", body = Page<AddressResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.addresses.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_buyer_addresses(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(buyer_id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<AddressResponse>, Error> {
    principal.require(&ADDRESSES_READ)?;
    let page = PageRequest::try_from(page)?;
    Ok(page_of(service.list_addresses(&tenant, buyer_id, &page).await?))
}
