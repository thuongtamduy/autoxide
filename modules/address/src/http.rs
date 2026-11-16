//! REST công khai của `/api/v1/address` (không cần đăng nhập): `ETag` = phiên bản bộ dữ liệu,
//! `Cache-Control: public, max-age=86400` (lớp chung của kernel trả 304 khi `If-None-Match` khớp).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;

use crate::api::{Address, AddressInput, AdministrativeUnit};
use crate::service::AddressService;

pub(crate) fn routes(service: Arc<AddressService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_provinces))
        .routes(routes!(list_communes))
        .routes(routes!(search_units))
        .routes(routes!(get_unit))
        .routes(routes!(validate_address))
        .with_state(service)
}

// Thêm `ETag` (phiên bản bộ dữ liệu của `country`) và `Cache-Control` cho dữ liệu công khai.
async fn cached(
    service: &AddressService,
    country: &str,
    response: Response,
) -> Result<Response, Error> {
    let mut response = response;
    if let Some(version) = service.version(&service.country(country)?).await?
        && let Ok(value) = HeaderValue::from_str(&version)
    {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=86400"));
    Ok(response)
}

fn default_country() -> String {
    "VN".to_owned()
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ListQuery {
    /// ISO country code; only `VN` for now. Defaults to `VN`.
    #[serde(default = "default_country")]
    #[param(example = "VN")]
    country: String,
    /// The list in force on this day (`YYYY-MM-DD`); defaults to today.
    #[param(example = "2025-07-01")]
    as_of: Option<String>,
}

/// List the provinces.
///
/// No sign-in needed. In code order (the order of the official list). Cached for a day; the
/// `ETag` changes when a new list is imported.
#[utoipa::path(
    get,
    path = "/provinces",
    extensions(("x-public" = json!(true))),
    operation_id = "list_provinces",
    security((), ("bearer" = [])),
    params(ListQuery),
    responses(
        (status = 200, description = "The provinces in force.", body = Data<Vec<AdministrativeUnit>>),
        (status = 304, description = "Not changed since the `ETag` sent in `If-None-Match`."),
        (status = 422, description = "No list for this country (`address.country_unsupported`), or `as_of` is not a date.", body = Problem),
    )
)]
async fn list_provinces(
    State(service): State<Arc<AddressService>>,
    Query(query): Query<ListQuery>,
) -> Result<Response, Error> {
    let data = service.provinces(&query.country, query.as_of.as_deref()).await?;
    cached(&service, &query.country, Data { data }.into_response()).await
}

/// List the communes, wards and special zones of a province.
///
/// No sign-in needed. In code order; cached like the provinces.
#[utoipa::path(
    get,
    path = "/provinces/{code}/communes",
    extensions(("x-public" = json!(true))),
    operation_id = "list_communes",
    security((), ("bearer" = [])),
    params(("code" = String, Path, description = "Code of the province, e.g. `01`."), ListQuery),
    responses(
        (status = 200, description = "The communes in force.", body = Data<Vec<AdministrativeUnit>>),
        (status = 304, description = "Not changed since the `ETag` sent in `If-None-Match`."),
        (status = 404, description = "No province with this code; code `address.unit_not_found`.", body = Problem),
        (status = 422, description = "No list for this country, or `as_of` is not a date.", body = Problem),
    )
)]
async fn list_communes(
    State(service): State<Arc<AddressService>>,
    Path(code): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Response, Error> {
    let data = service.communes(&query.country, &code, query.as_of.as_deref()).await?;
    cached(&service, &query.country, Data { data }.into_response()).await
}

#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SearchQuery {
    #[serde(default = "default_country")]
    #[param(example = "VN")]
    country: String,
    /// Part of the name, with or without accents (`ha noi` also finds the accented name).
    #[validate(length(max = 100))]
    q: Option<String>,
    /// Only units of this province (and the province itself).
    province: Option<String>,
    /// `1`: provinces; `2`: communes.
    #[param(minimum = 1, maximum = 2)]
    level: Option<i32>,
}

/// Search administrative units by name.
///
/// No sign-in needed. Units in force today, in code order.
#[utoipa::path(
    get,
    path = "/units",
    extensions(("x-public" = json!(true))),
    operation_id = "search_administrative_units",
    security((), ("bearer" = [])),
    params(SearchQuery, PageQuery),
    responses(
        (status = 200, description = "One page of units.", body = Page<AdministrativeUnit>),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn search_units(
    State(service): State<Arc<AddressService>>,
    Query(search): Query<SearchQuery>,
    Query(query): Query<PageQuery>,
) -> Result<Page<AdministrativeUnit>, Error> {
    search.validate().map_err(Error::from)?;
    let page = PageRequest::try_from(query)?;
    service
        .search(
            &search.country,
            search.q.as_deref(),
            search.province.as_deref(),
            search.level,
            &page,
        )
        .await
}

/// An administrative unit with the unit above it.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct AdministrativeUnitDetail {
    #[serde(flatten)]
    unit: AdministrativeUnit,
    /// The province, for a commune.
    parent: Option<AdministrativeUnit>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct CountryQuery {
    #[serde(default = "default_country")]
    #[param(example = "VN")]
    country: String,
}

/// Get an administrative unit by code.
///
/// No sign-in needed.
#[utoipa::path(
    get,
    path = "/units/{code}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_administrative_unit",
    security((), ("bearer" = [])),
    params(("code" = String, Path, description = "Code of the unit."), CountryQuery),
    responses(
        (status = 200, description = "The unit and the unit above it.", body = Data<AdministrativeUnitDetail>),
        (status = 304, description = "Not changed since the `ETag` sent in `If-None-Match`."),
        (status = 404, description = "No unit in force with this code; code `address.unit_not_found`.", body = Problem),
    )
)]
async fn get_unit(
    State(service): State<Arc<AddressService>>,
    Path(code): Path<String>,
    Query(query): Query<CountryQuery>,
) -> Result<Response, Error> {
    let (unit, parent) = service.get(&query.country, &code).await?;
    cached(
        &service,
        &query.country,
        Data { data: AdministrativeUnitDetail { unit, parent } }.into_response(),
    )
    .await
}

/// An address to check.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct ValidateRequest {
    #[serde(flatten)]
    address: AddressInput,
}

/// Check an address and fill in the names.
///
/// No sign-in needed. The commune must be in force and belong to the province.
#[utoipa::path(
    post,
    path = "/validate",
    extensions(("x-public" = json!(true))),
    operation_id = "validate_address",
    security((), ("bearer" = [])),
    request_body = AddressInput,
    responses(
        (status = 200, description = "The address with the names of its units.", body = Data<Address>),
        (status = 422, description = "The country is not supported, a unit does not exist, or the commune is not in the province (`address.commune_not_in_province`); see `errors`.", body = Problem),
    )
)]
async fn validate_address(
    State(service): State<Arc<AddressService>>,
    ValidJson(input): ValidJson<ValidateRequest>,
) -> Result<Data<Address>, Error> {
    let data = service.check(input.address, "").await.map_err(Error::validation)?;
    Ok(Data { data })
}
