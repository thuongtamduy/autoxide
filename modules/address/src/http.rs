//! REST công khai của `/api/v1/address` (không cần đăng nhập): `ETag` = phiên bản bộ dữ liệu,
//! `Cache-Control: public, max-age=86400` (lớp chung của kernel trả 304 khi `If-None-Match` khớp).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::http::{
    Params, Path, PublicCache, Query, Routes, Schema, State, ValidJson, Validate, endpoint,
};
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::routes;
use serde::{Deserialize, Serialize};

use crate::api::{Address, AddressInput, AdministrativeUnit};
use crate::service::AddressService;

pub(crate) fn routes(service: Arc<AddressService>) -> Routes {
    routes![list_provinces, list_communes, search_units, get_unit, validate_address]
        .with_state(service)
}

/// Dữ liệu công khai giữ trong cache một ngày.
const CACHE_SECONDS: u32 = 86_400;

// Thêm `ETag` (phiên bản bộ dữ liệu của `country`) và `Cache-Control` cho dữ liệu công khai.
async fn cached<T>(
    service: &AddressService,
    country: &str,
    body: T,
) -> Result<PublicCache<T>, Error> {
    let version = service.version(&service.country(country)?).await?;
    Ok(PublicCache::new(body, CACHE_SECONDS).with_etag(version))
}

fn default_country() -> String {
    "VN".to_owned()
}

#[derive(Debug, Deserialize, Params)]
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
/// No sign-in needed. In code order (the order of the official list) unless `order=desc`; paged
/// with `limit` / `cursor`. Cached for a day; the `ETag` changes when a new list is imported.
#[endpoint(
    get,
    path = "/provinces",
    public,
    security((), ("bearer" = [])),
    params(ListQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the provinces in force, in code order.", body = Page<AdministrativeUnit>),
        (status = 304, description = "Not changed since the `ETag` sent in `If-None-Match`."),
        (status = 422, description = "No list for this country (`address.country_unsupported`), `as_of` is not a date, or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_provinces(
    State(service): State<Arc<AddressService>>,
    Query(query): Query<ListQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<PublicCache<Page<AdministrativeUnit>>, Error> {
    let page = PageRequest::try_from(page)?;
    let units = service.provinces(&query.country, query.as_of.as_deref(), &page).await?;
    cached(&service, &query.country, units).await
}

/// List the communes, wards and special zones of a province.
///
/// No sign-in needed. In code order unless `order=desc`; paged with `limit` / `cursor`; cached like
/// the provinces.
#[endpoint(
    get,
    path = "/provinces/{code}/communes",
    public,
    security((), ("bearer" = [])),
    params(("code" = String, Path, description = "Code of the province, e.g. `01`."), ListQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the communes in force, in code order.", body = Page<AdministrativeUnit>),
        (status = 304, description = "Not changed since the `ETag` sent in `If-None-Match`."),
        (status = 404, description = "No province with this code; code `address.unit_not_found`.", body = Problem),
        (status = 422, description = "No list for this country, `as_of` is not a date, or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_communes(
    State(service): State<Arc<AddressService>>,
    Path(code): Path<String>,
    Query(query): Query<ListQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<PublicCache<Page<AdministrativeUnit>>, Error> {
    let page = PageRequest::try_from(page)?;
    let units = service.communes(&query.country, &code, query.as_of.as_deref(), &page).await?;
    cached(&service, &query.country, units).await
}

#[derive(Debug, Deserialize, Validate, Params)]
#[into_params(parameter_in = Query)]
pub(crate) struct SearchQuery {
    /// ISO country code; only `VN` for now. Defaults to `VN`.
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
#[endpoint(
    get,
    path = "/units",
    public,
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
    search.check()?;
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
#[derive(Debug, Serialize, Deserialize, Schema)]
pub(crate) struct AdministrativeUnitDetail {
    #[serde(flatten)]
    unit: AdministrativeUnit,
    /// The province, for a commune.
    parent: Option<AdministrativeUnit>,
}

#[derive(Debug, Deserialize, Params)]
#[into_params(parameter_in = Query)]
pub(crate) struct CountryQuery {
    /// ISO country code; only `VN` for now. Defaults to `VN`.
    #[serde(default = "default_country")]
    #[param(example = "VN")]
    country: String,
}

/// Get an administrative unit by code.
///
/// No sign-in needed.
#[endpoint(
    get,
    path = "/units/{code}",
    public,
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
) -> Result<PublicCache<Data<AdministrativeUnitDetail>>, Error> {
    let (unit, parent) = service.get(&query.country, &code).await?;
    cached(&service, &query.country, Data { data: AdministrativeUnitDetail { unit, parent } }).await
}

/// An address to check.
#[derive(Debug, Deserialize, Validate, Schema)]
pub(crate) struct ValidateRequest {
    #[serde(flatten)]
    address: AddressInput,
}

/// Check an address and fill in the names.
///
/// No sign-in needed. The commune must be in force and belong to the province.
#[endpoint(
    post,
    path = "/validate",
    public,
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
