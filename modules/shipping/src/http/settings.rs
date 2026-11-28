//! Cài đặt vận chuyển của cửa hàng: vùng, phương thức, bậc phí, điểm nhận tại cửa hàng. Mọi
//! endpoint cần `shipping.settings.manage` ở nút của bản ghi.

use autoxide_kernel::time::Timestamp;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::api::MethodKind;
use crate::entity::{method, pickup_point, rate};
use crate::permissions::SETTINGS_MANAGE;
use crate::service::ShippingService;
use crate::settings::{MethodFields, MethodView, PickupInput, RateInput, Region, ZoneView};

/// Một trường có mặt trong thân (kể cả `null`) → `Some(…)`; vắng → `None`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Filter of a settings list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct OrgFilter {
    /// Only records of this organization unit.
    org_id: Option<Uuid>,
}

/// One region of a zone: a country, or one province of it.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[schema(as = ShippingRegion)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegionBody {
    /// ISO 3166-1 alpha-2 country code.
    #[schema(example = "VN")]
    country: String,
    /// Code of the province; `null` for the whole country. A zone listing the province of the
    /// destination wins over a zone for the whole country.
    #[schema(example = "01")]
    #[serde(default)]
    province_code: Option<String>,
}

impl From<RegionBody> for Region {
    fn from(body: RegionBody) -> Self {
        Region { country: body.country, province_code: body.province_code }
    }
}

impl From<Region> for RegionBody {
    fn from(region: Region) -> Self {
        RegionBody { country: region.country, province_code: region.province_code }
    }
}

/// A new shipping zone.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = NewShippingZone)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewZoneRequest {
    /// Organization unit owning the zone; defaults to where you work when that is one unit.
    org_id: Option<Uuid>,
    /// 1 to 200 characters, e.g. `Inner Hanoi`.
    name: String,
    #[serde(default)]
    regions: Vec<RegionBody>,
}

/// Changes of a zone; leave out what stays.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ShippingZoneChanges)]
#[serde(deny_unknown_fields)]
pub(crate) struct ZoneChangesRequest {
    name: Option<String>,
}

/// The full list of regions of a zone.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ShippingZoneRegions)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegionsRequest {
    regions: Vec<RegionBody>,
}

/// A shipping zone.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingZone)]
pub(crate) struct ZoneResponse {
    id: Uuid,
    org_id: Uuid,
    name: String,
    regions: Vec<RegionBody>,
    updated_at: Timestamp,
}

impl From<ZoneView> for ZoneResponse {
    fn from(view: ZoneView) -> Self {
        ZoneResponse {
            id: view.zone.id,
            org_id: view.zone.org_id,
            name: view.zone.name,
            regions: view.regions.into_iter().map(RegionBody::from).collect(),
            updated_at: view.zone.updated_at,
        }
    }
}

/// List shipping zones.
///
/// Needs `shipping.settings.manage`; lists the zones of the units it covers.
#[utoipa::path(
    get,
    path = "/zones",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "list_shipping_zones",
    security(("bearer" = [])),
    params(OrgFilter, PageQuery),
    responses(
        (status = 200, description = "One page of zones.", body = Page<ZoneResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_zones(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<OrgFilter>,
    Query(page): Query<PageQuery>,
) -> Result<Page<ZoneResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let page = PageRequest::try_from(page)?;
    Ok(service.list_zones(&tenant, &principal, filter.org_id, &page).await?.map(ZoneResponse::from))
}

/// Create a shipping zone.
///
/// Needs `shipping.settings.manage` at the unit. A zone groups provinces (or whole countries) that
/// share the same rates.
#[utoipa::path(
    post,
    path = "/zones",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "create_shipping_zone",
    security(("bearer" = [])),
    request_body = NewZoneRequest,
    responses(
        (status = 201, description = "Created.", body = Data<ZoneResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage` at the unit (`permission_denied`, `org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`. Several units and no `org_id`: `shipping.org_required`.", body = Problem),
    )
)]
pub(crate) async fn create_zone(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<NewZoneRequest>,
) -> Result<Created<ZoneResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let regions = input.regions.into_iter().map(Region::from).collect();
    let zone = service.create_zone(&tenant, &principal, input.org_id, input.name, regions).await?;
    Ok(Created::new(format!("/shipping/zones/{}", zone.zone.id), zone.into()))
}

/// Get a shipping zone.
#[utoipa::path(
    get,
    path = "/zones/{id}",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "get_shipping_zone",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the zone.")),
    responses(
        (status = 200, description = "The zone.", body = Data<ZoneResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No zone with this id in your scope; code `shipping.zone_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_zone(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ZoneResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    Ok(Data { data: service.get_zone(&tenant, &principal, id).await?.into() })
}

/// Rename a shipping zone.
#[utoipa::path(
    patch,
    path = "/zones/{id}",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "update_shipping_zone",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the zone.")),
    request_body = ZoneChangesRequest,
    responses(
        (status = 200, description = "The zone after the change.", body = Data<ZoneResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage` at the unit, or the unit is not active; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No zone with this id in your scope; code `shipping.zone_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn update_zone(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ZoneChangesRequest>,
) -> Result<Data<ZoneResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let zone = service.update_zone(&tenant, &principal, id, input.name, None).await?;
    Ok(Data { data: zone.into() })
}

/// Delete a shipping zone.
///
/// Its rates in every method go with it.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/zones/{id}",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "delete_shipping_zone",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the zone.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage` at the unit, or the unit is not active; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No zone with this id in your scope; code `shipping.zone_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_zone(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    service.delete_zone(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Replace the regions of a shipping zone.
#[utoipa::path(
    put,
    path = "/zones/{id}/regions",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "set_shipping_zone_regions",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the zone.")),
    request_body = RegionsRequest,
    responses(
        (status = 200, description = "The zone after the change.", body = Data<ZoneResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage` at the unit, or the unit is not active; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No zone with this id in your scope; code `shipping.zone_not_found`.", body = Problem),
        (status = 422, description = "A region is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn set_zone_regions(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<RegionsRequest>,
) -> Result<Data<ZoneResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let regions = input.regions.into_iter().map(Region::from).collect();
    let zone = service.update_zone(&tenant, &principal, id, None, Some(regions)).await?;
    Ok(Data { data: zone.into() })
}

// ---- Phương thức -------------------------------------------------------------------------------

/// Days a delivery takes, as declared by the method.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema)]
#[schema(as = ShippingDays)]
#[serde(deny_unknown_fields)]
pub(crate) struct DaysBody {
    min: i16,
    max: i16,
}

/// Fields of a shipping method. On create `code`, `kind`, `names` and `currency` are required;
/// on change, leave out what stays (`code`, `kind`, `org_id`, `currency` cannot change).
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[schema(as = ShippingMethodFields)]
#[serde(deny_unknown_fields)]
pub(crate) struct MethodRequest {
    /// Organization unit owning the method; units below inherit it unless they have a method with
    /// the same `code`.
    org_id: Option<Uuid>,
    /// Stable code, lower-case letters, digits and `-`, e.g. `standard`.
    #[schema(example = "standard")]
    code: Option<String>,
    kind: Option<MethodKind>,
    /// Name per language, e.g. `{"vi": "Giao tieu chuan", "en": "Standard"}`.
    names: Option<BTreeMap<String, String>>,
    active: Option<bool>,
    /// Display order, smaller first.
    position: Option<i32>,
    /// `carrier` methods: the carrier account quoting the fee.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    carrier_account_id: Option<Option<Uuid>>,
    /// `carrier` methods: the carrier's service code; `null` for every service.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    service_code: Option<Option<String>>,
    /// ISO 4217 code of every amount of the method, e.g. `VND`.
    #[schema(example = "VND")]
    currency: Option<String>,
    /// Shipping is free when the order value (after discounts) reaches this amount; `null` never.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Money>, nullable)]
    free_over: Option<Option<Money>>,
    /// Added to every fee of the method (packing, insurance…); the fee of a pickup method.
    handling_fee: Option<Money>,
    /// Tax category of the shipping fee in the tax engine, default `vat.standard`.
    tax_category: Option<String>,
    /// Volumetric divisor: volumetric grams = length × width × height (mm³) / divisor. Default 6000.
    volumetric_divisor: Option<i32>,
    /// `table_rate` methods: weight of an item that has none, in grams; `null`: such items make the
    /// method unavailable (`shipping.weight_missing`).
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i64>, nullable)]
    default_item_weight_grams: Option<Option<i64>>,
    /// Countries shipped to (ISO codes), default `["VN"]`.
    countries: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<DaysBody>, nullable)]
    estimated_days: Option<Option<DaysBody>>,
    /// Delivery calendar (working days, holidays, cut-off times).
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    calendar_id: Option<Option<Uuid>>,
    /// Orders before this minute of the day (calendar time zone; 660 = 11:00) leave the same day.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i32>, nullable)]
    cutoff_minute: Option<Option<i32>>,
    /// Delivered the same day when ordered before the cut-off.
    same_day: Option<bool>,
    /// `pickup` methods: days the parcel waits for the buyer.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i16>, nullable)]
    hold_days: Option<Option<i16>>,
    /// `carrier` methods: book the carrier as soon as an order asks for a shipment (default off:
    /// staff check the goods first).
    auto_book: Option<bool>,
}

impl From<MethodRequest> for MethodFields {
    fn from(input: MethodRequest) -> Self {
        MethodFields {
            org_id: input.org_id,
            code: input.code,
            kind: input.kind,
            names: input.names,
            active: input.active,
            position: input.position,
            carrier_account_id: input.carrier_account_id,
            service_code: input.service_code,
            currency: input.currency,
            free_over: input.free_over,
            handling_fee: input.handling_fee,
            tax_category: input.tax_category,
            volumetric_divisor: input.volumetric_divisor,
            default_item_weight_grams: input.default_item_weight_grams,
            countries: input.countries,
            estimated_days: input.estimated_days.map(|days| days.map(|days| (days.min, days.max))),
            calendar_id: input.calendar_id,
            cutoff_minute: input.cutoff_minute,
            same_day: input.same_day,
            hold_days: input.hold_days,
            auto_book: input.auto_book,
        }
    }
}

/// A weight tier of a method in one zone.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[schema(as = ShippingRateTier)]
#[serde(deny_unknown_fields)]
pub(crate) struct RateBody {
    zone_id: Uuid,
    /// The tier applies from this chargeable weight (grams)…
    min_weight_grams: i64,
    /// …up to, not including, this weight; `null` for no upper limit.
    #[serde(default)]
    max_weight_grams: Option<i64>,
    /// Longest side allowed, in mm; `null` for any size.
    #[serde(default)]
    max_side_mm: Option<i64>,
    amount: Money,
    /// Each `step_grams` (or part) above `min_weight_grams` adds `step_amount`.
    #[serde(default)]
    step_grams: Option<i64>,
    #[serde(default)]
    step_amount: Option<Money>,
}

/// The full list of weight tiers of a method.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ShippingRates)]
#[serde(deny_unknown_fields)]
pub(crate) struct RatesRequest {
    rates: Vec<RateBody>,
}

/// A store where buyers collect their parcels.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[schema(as = ShippingPickupPoint)]
#[serde(deny_unknown_fields)]
pub(crate) struct PickupBody {
    /// The store: the method's unit or a unit below it.
    org_id: Uuid,
    /// Shown to the buyer, e.g. where to park.
    #[serde(default)]
    instructions: String,
    /// Hours to get the order ready (0 to 720).
    #[serde(default)]
    prep_hours: i32,
}

/// The full list of pickup points of a `pickup` method.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ShippingPickupPoints)]
#[serde(deny_unknown_fields)]
pub(crate) struct PickupPointsRequest {
    pickup_points: Vec<PickupBody>,
}

/// A shipping method.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingMethod)]
pub(crate) struct MethodResponse {
    id: Uuid,
    org_id: Uuid,
    code: String,
    kind: MethodKind,
    names: BTreeMap<String, String>,
    active: bool,
    position: i32,
    carrier_account_id: Option<Uuid>,
    service_code: Option<String>,
    currency: String,
    free_over: Option<Money>,
    handling_fee: Money,
    tax_category: String,
    volumetric_divisor: i32,
    default_item_weight_grams: Option<i64>,
    countries: Vec<String>,
    estimated_days: Option<DaysBody>,
    calendar_id: Option<Uuid>,
    cutoff_minute: Option<i32>,
    same_day: bool,
    hold_days: Option<i16>,
    auto_book: bool,
    /// Weight tiers (only in `GET /methods/{id}` and after changes).
    rates: Vec<RateBody>,
    pickup_points: Vec<PickupBody>,
    updated_at: Timestamp,
}

/// Tiền từ hai cột; mã tiền tệ hỏng (không xảy ra: đã kiểm lúc ghi) thì số 0 của cùng loại tiền
/// với phương thức không đọc được nữa — trả 0 VND thay vì làm hỏng cả danh sách.
fn money(minor: i64, currency: &str) -> Money {
    Money::from_columns(minor, currency)
        .unwrap_or_else(|_| Money::new(0, autoxide_kernel::money::Currency::VND))
}

fn method_response(
    row: method::Model,
    rates: Vec<rate::Model>,
    points: Vec<pickup_point::Model>,
) -> MethodResponse {
    let currency = row.currency.clone();
    MethodResponse {
        id: row.id,
        org_id: row.org_id,
        code: row.code,
        kind: MethodKind::parse(&row.kind).unwrap_or(MethodKind::TableRate),
        names: serde_json::from_str(&row.names).unwrap_or_default(),
        active: row.active,
        position: row.position,
        carrier_account_id: row.carrier_account_id,
        service_code: row.service_code,
        currency: row.currency.clone(),
        free_over: row.free_over_minor.map(|minor| money(minor, &currency)),
        handling_fee: money(row.handling_fee_minor, &currency),
        tax_category: row.tax_category,
        volumetric_divisor: row.volumetric_divisor,
        default_item_weight_grams: row.default_item_weight_grams,
        countries: serde_json::from_str(&row.countries).unwrap_or_default(),
        estimated_days: row
            .estimated_min_days
            .zip(row.estimated_max_days)
            .map(|(min, max)| DaysBody { min, max }),
        calendar_id: row.calendar_id,
        cutoff_minute: row.cutoff_minute,
        same_day: row.same_day,
        hold_days: row.hold_days,
        auto_book: row.auto_book,
        rates: rates
            .into_iter()
            .map(|tier| RateBody {
                zone_id: tier.zone_id,
                min_weight_grams: tier.min_weight_grams,
                max_weight_grams: tier.max_weight_grams,
                max_side_mm: tier.max_side_mm,
                amount: money(tier.amount_minor, &tier.currency),
                step_grams: tier.step_grams,
                step_amount: tier.step_amount_minor.map(|minor| money(minor, &tier.currency)),
            })
            .collect(),
        pickup_points: points
            .into_iter()
            .map(|point| PickupBody {
                org_id: point.org_id,
                instructions: point.instructions,
                prep_hours: point.prep_hours,
            })
            .collect(),
        updated_at: row.updated_at,
    }
}

impl From<MethodView> for MethodResponse {
    fn from(view: MethodView) -> Self {
        method_response(view.method, view.rates, view.pickup_points)
    }
}

/// List shipping methods.
///
/// Needs `shipping.settings.manage`; lists the methods of the units it covers (not inherited
/// ones: see `POST /quote` for what applies at a store).
#[utoipa::path(
    get,
    path = "/methods",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "list_shipping_methods",
    security(("bearer" = [])),
    params(OrgFilter, PageQuery),
    responses(
        (status = 200, description = "One page of methods, without tiers.", body = Page<MethodResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_methods(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<OrgFilter>,
    Query(page): Query<PageQuery>,
) -> Result<Page<MethodResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let page = PageRequest::try_from(page)?;
    let page = service.list_methods(&tenant, &principal, filter.org_id, &page).await?;
    Ok(page.map(|row| method_response(row, Vec::new(), Vec::new())))
}

/// Create a shipping method.
///
/// Needs `shipping.settings.manage` at the unit. Kinds: `table_rate` (your own weight tiers per
/// zone), `carrier` (the fee a carrier account quotes), `pickup` (collect in store).
#[utoipa::path(
    post,
    path = "/methods",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "create_shipping_method",
    security(("bearer" = [])),
    request_body = MethodRequest,
    responses(
        (status = 201, description = "Created.", body = Data<MethodResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage` at the unit (`permission_denied`, `org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 409, description = "The unit already has a method with this code; code `shipping.method_code_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_method(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<MethodRequest>,
) -> Result<Created<MethodResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let view = service.create_method(&tenant, &principal, input.into()).await?;
    Ok(Created::new(format!("/shipping/methods/{}", view.method.id), view.into()))
}

/// Get a shipping method with its tiers and pickup points.
#[utoipa::path(
    get,
    path = "/methods/{id}",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "get_shipping_method",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the method.")),
    responses(
        (status = 200, description = "The method.", body = Data<MethodResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No method with this id in your scope; code `shipping.method_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_method(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<MethodResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    Ok(Data { data: service.get_method(&tenant, &principal, id).await?.into() })
}

/// Change a shipping method.
#[utoipa::path(
    patch,
    path = "/methods/{id}",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "update_shipping_method",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the method.")),
    request_body = MethodRequest,
    responses(
        (status = 200, description = "The method after the change.", body = Data<MethodResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage` at the unit, or the unit is not active; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No method with this id in your scope; code `shipping.method_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid or cannot change; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn update_method(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<MethodRequest>,
) -> Result<Data<MethodResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    Ok(Data { data: service.update_method(&tenant, &principal, id, input.into()).await?.into() })
}

/// Delete a shipping method with its tiers and pickup points.
///
/// Shipments already created keep their copy of the method.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/methods/{id}",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "delete_shipping_method",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the method.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage` at the unit, or the unit is not active; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No method with this id in your scope; code `shipping.method_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_method(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    service.delete_method(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Replace the weight tiers of a shipping method.
///
/// Tiers of one zone must not overlap (`[min, max)` ranges with the same `max_side_mm`).
#[utoipa::path(
    put,
    path = "/methods/{id}/rates",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "set_shipping_method_rates",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the method.")),
    request_body = RatesRequest,
    responses(
        (status = 200, description = "The method after the change.", body = Data<MethodResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage` at the unit, or the unit is not active; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No method or zone with this id in your scope; codes `shipping.method_not_found`, `shipping.zone_not_found`.", body = Problem),
        (status = 422, description = "A tier is invalid (see `errors`), or tiers overlap (`shipping.rate_overlap`).", body = Problem),
    )
)]
pub(crate) async fn set_method_rates(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<RatesRequest>,
) -> Result<Data<MethodResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let rates = input
        .rates
        .into_iter()
        .map(|body| RateInput {
            zone_id: body.zone_id,
            min_weight_grams: body.min_weight_grams,
            max_weight_grams: body.max_weight_grams,
            max_side_mm: body.max_side_mm,
            amount: body.amount,
            step_grams: body.step_grams,
            step_amount: body.step_amount,
        })
        .collect();
    Ok(Data { data: service.put_rates(&tenant, &principal, id, rates).await?.into() })
}

/// Replace the pickup points of a `pickup` shipping method.
#[utoipa::path(
    put,
    path = "/methods/{id}/pickup-points",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "set_shipping_method_pickup_points",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the method.")),
    request_body = PickupPointsRequest,
    responses(
        (status = 200, description = "The method after the change.", body = Data<MethodResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage` at the unit, or the unit is not active; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No method with this id in your scope; code `shipping.method_not_found`.", body = Problem),
        (status = 409, description = "The method is not a `pickup` method; code `shipping.not_pickup_method`.", body = Problem),
        (status = 422, description = "A pickup point is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn set_method_pickup_points(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<PickupPointsRequest>,
) -> Result<Data<MethodResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let points = input
        .pickup_points
        .into_iter()
        .map(|body| PickupInput {
            org_id: body.org_id,
            instructions: body.instructions,
            prep_hours: body.prep_hours,
        })
        .collect();
    Ok(Data { data: service.put_pickup_points(&tenant, &principal, id, points).await?.into() })
}
