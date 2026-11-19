//! Hồ sơ công khai của nút (D-200, D-315): đọc / sửa (quản trị), danh sách cửa hàng công khai.

use std::sync::Arc;

use autoxide_address::api::{Address, AddressInput};
use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::permissions::UNITS_READ;
use crate::profile::{OpeningHours, Profile, ProfileChanges, SpecialDay};
use crate::service::OrgService;

pub(crate) fn routes() -> OpenApiRouter<Arc<OrgService>> {
    OpenApiRouter::new()
        .routes(routes!(get_public_profile, update_public_profile))
        .routes(routes!(list_public_stores))
        .routes(routes!(get_site_profile))
        .routes(routes!(get_public_storefront))
}

/// The public profile of an organization unit, as websites show it.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicProfileResponse {
    unit_id: Uuid,
    #[schema(example = "store")]
    unit_type: String,
    /// Shown on websites (`GET /api/v1/organizations/public/stores`).
    public: bool,
    /// Order among the public units (smaller first).
    position: i32,
    /// Name shown to visitors; defaults to the unit's name.
    display_name: String,
    address: Option<Address>,
    #[schema(inline)]
    opening_hours: Vec<OpeningHours>,
    #[schema(inline)]
    special_days: Vec<SpecialDay>,
    /// Customers can pick up orders here.
    pickup: bool,
    /// Cover image (id in the `media` library).
    cover_media_id: Option<Uuid>,
    /// `null` until the profile is first saved.
    updated_at: Option<DateTime<Utc>>,
}

impl From<Profile> for PublicProfileResponse {
    fn from(profile: Profile) -> Self {
        PublicProfileResponse {
            unit_id: profile.unit_id,
            unit_type: profile.unit_type,
            public: profile.public,
            position: profile.position,
            display_name: profile.display_name,
            address: profile.address,
            opening_hours: profile.opening_hours,
            special_days: profile.special_days,
            pickup: profile.pickup,
            cover_media_id: profile.cover_media_id,
            updated_at: profile.updated_at,
        }
    }
}

// Trường JSON có ba trạng thái: không gửi (`None`), `null` (`Some(None)`), có giá trị.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Changes to a public profile. Omitted fields stay as they are; `null` clears.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublicProfileRequest {
    public: Option<bool>,
    position: Option<i32>,
    /// 1 to 200 characters; `null` goes back to the unit's name.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    display_name: Option<Option<String>>,
    /// Checked against the official list of administrative units; `null` clears it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<AddressInput>, nullable)]
    address: Option<Option<AddressInput>>,
    /// Replaces every range.
    #[schema(inline)]
    opening_hours: Option<Vec<OpeningHours>>,
    /// Replaces every special day.
    #[schema(inline)]
    special_days: Option<Vec<SpecialDay>>,
    pickup: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    cover_media_id: Option<Option<Uuid>>,
}

/// Get the public profile of an organization unit.
///
/// Requires `organizations.units.read` covering the unit.
#[utoipa::path(
    get,
    path = "/units/{id}/public-profile",
    extensions(("x-permissions" = json!(["organizations.units.read"]))),
    operation_id = "get_unit_public_profile",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the unit.")),
    responses(
        (status = 200, description = "The profile (defaults when never saved).", body = Data<PublicProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `organizations.units.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such unit in your scope; code `organizations.unit_not_found`.", body = Problem),
    )
)]
async fn get_public_profile(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PublicProfileResponse>, Error> {
    principal.require(&UNITS_READ)?;
    Ok(Data { data: service.profile(&tenant, &principal, id).await?.into() })
}

/// Change the public profile of an organization unit.
///
/// Requires `organizations.profile.manage` covering the unit; the unit must be active. Websites
/// see the change at once (`GET /api/v1/organizations/public/stores`).
#[utoipa::path(
    patch,
    path = "/units/{id}/public-profile",
    extensions(("x-permissions" = json!(["organizations.profile.manage"]))),
    operation_id = "update_unit_public_profile",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the unit.")),
    request_body = PublicProfileRequest,
    responses(
        (status = 200, description = "The profile after the change.", body = Data<PublicProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or not active (`org_frozen`).", body = Problem),
        (status = 404, description = "No such unit in your scope; code `organizations.unit_not_found`.", body = Problem),
        (status = 422, description = "Opening hours are invalid (`organizations.invalid_opening_hours`), or another value, e.g. the address, is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_public_profile(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<PublicProfileRequest>,
) -> Result<Data<PublicProfileResponse>, Error> {
    let changes = ProfileChanges {
        public: input.public,
        position: input.position,
        display_name: input.display_name,
        address: input.address,
        opening_hours: input.opening_hours,
        special_days: input.special_days,
        pickup: input.pickup,
        cover_media_id: input.cover_media_id,
    };
    Ok(Data { data: service.update_profile(&tenant, &principal, id, changes).await?.into() })
}

/// Get the profile of the website being visited.
///
/// No sign-in needed. The website is the organization unit the domain of the request is bound to
/// (see `PATCH /api/v1/kernel/domains/{id}`), or the one named with `site` when the page is not
/// served on its own domain; otherwise the whole tenant (its root unit). A store's own domain thus
/// shows the store's name, address and hours, not the head office's. Use `unit_id` as `site` in
/// other public calls (menus, content) when they are not made on that domain.
#[utoipa::path(
    get,
    path = "/public/site",
    extensions(("x-public" = json!(true))),
    operation_id = "get_site_profile",
    security((), ("bearer" = [])),
    params(autoxide_kernel::tenancy::SiteQuery),
    responses(
        (status = 200, description = "The public profile of the website's unit.", body = Data<PublicProfileResponse>),
        (status = 400, description = "`site` is not a UUID; code `invalid_site`.", body = Problem),
        (status = 404, description = "No active unit `site` in this tenant; code `site_not_found`.", body = Problem),
    )
)]
async fn get_site_profile(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    site: autoxide_kernel::tenancy::Site,
) -> Result<Data<PublicProfileResponse>, Error> {
    Ok(Data { data: service.site_profile(&tenant, &site).await?.into() })
}

/// List the stores of this tenant shown on its websites.
///
/// No sign-in needed; the tenant comes from the domain (or the header `Autoxide-Tenant`). Only
/// active units whose profile is public, in the order the tenant set. For "stores near you" and
/// choosing where to pick up an order.
#[utoipa::path(
    get,
    path = "/public/stores",
    extensions(("x-public" = json!(true))),
    operation_id = "list_public_stores",
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "The public stores.", body = Data<Vec<PublicProfileResponse>>),
        (status = 304, description = "Not changed since the `ETag` sent in `If-None-Match`."),
    )
)]
async fn list_public_stores(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
) -> Result<Response, Error> {
    let data: Vec<PublicProfileResponse> =
        service.public_stores(&tenant).await?.into_iter().map(Into::into).collect();
    let body = serde_json::to_vec(&data).map_err(Error::internal)?;
    let etag =
        format!("\"{}\"", &autoxide_kernel::secret::sha256(&String::from_utf8_lossy(&body))[..22]);
    let mut response = Data { data }.into_response();
    if let Ok(value) = HeaderValue::from_str(&etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=300"));
    Ok(response)
}

/// The brand of a website, inherited down the organization tree.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct BrandResponse {
    /// Brand name (setting `organizations.brand_name`, else the unit's display name).
    name: String,
    /// Logo: a public media item; load it from `/api/v1/media/{id}/content`.
    logo_media_id: Option<Uuid>,
    /// Site icon (favicon): a public media item.
    icon_media_id: Option<Uuid>,
    phone: Option<String>,
    email: Option<String>,
    #[schema(inline)]
    social_links: Vec<crate::storefront::SocialLink>,
}

/// What a website shows about its store: the brand and the location.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct StorefrontResponse {
    brand: BrandResponse,
    /// The unit's public profile: address, opening hours, special days, pickup, cover picture.
    location: PublicProfileResponse,
    /// IANA time zone of the unit; opening hours are local times in it.
    #[schema(example = "Asia/Ho_Chi_Minh")]
    timezone: String,
    /// Open right now, in the unit's time zone (special days first); `null` when no opening
    /// hours are set.
    open_now: Option<bool>,
}

/// Get the store information of a website (public).
///
/// No sign-in needed. The brand (name, logo, site icon, phone, email, social profiles) is
/// inherited down the organization tree: a chain sets it, a store overrides what it wants (manage
/// it with `PUT /api/v1/organizations/units/{id}/settings/organizations.brand_name` and the other
/// `organizations.brand_*`, `organizations.contact_*`, `organizations.social_links` settings). The
/// location is the unit's public profile. For the legal footer of a Vietnamese shop, add the
/// owner information and the Ministry of Industry and Trade badge from
/// `GET /api/v1/compliance/public/owner` and `GET /api/v1/compliance/public/badge`.
#[utoipa::path(
    get,
    path = "/public/storefront",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_storefront",
    security((), ("bearer" = [])),
    params(autoxide_kernel::tenancy::SiteQuery),
    responses(
        (status = 200, description = "The store information.", body = Data<StorefrontResponse>, headers(("Cache-Control" = String, description = "`public, max-age=60`: `open_now` changes with time."))),
        (status = 400, description = "`site` is not a UUID; code `invalid_site`.", body = Problem),
        (status = 404, description = "No active unit `site` in this tenant; code `site_not_found`.", body = Problem),
    )
)]
async fn get_public_storefront(
    State(service): State<Arc<OrgService>>,
    tenant: TenantContext,
    site: autoxide_kernel::tenancy::Site,
) -> Result<Response, Error> {
    let found = service.public_storefront(&tenant, &site, Utc::now()).await?;
    let brand = found.brand;
    let data = StorefrontResponse {
        brand: BrandResponse {
            name: brand.name,
            logo_media_id: brand.logo_media_id,
            icon_media_id: brand.icon_media_id,
            phone: brand.phone,
            email: brand.email,
            social_links: brand.social_links,
        },
        location: found.profile.into(),
        timezone: found.timezone,
        open_now: found.open_now,
    };
    let mut response = Data { data }.into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=60"));
    Ok(response)
}
