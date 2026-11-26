//! Đầu trang, chân trang và khung công khai của website (bước 67, D-234). Logic ở `site/`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{Site, SiteQuery, TenantContext};
use autoxide_kernel::time::Timestamp;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::IntoParams;
use autoxide_spike_macros::Schema as ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::service::{ContentService, now};
use crate::site::{RegionKey, SiteRegion};

/// A website header or footer.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct SiteRegionResponse {
    id: Uuid,
    org_id: Uuid,
    /// `header` or `footer`.
    region: String,
    locale: String,
    /// The theme variant it belongs to; `null`: the base theme.
    variant_id: Option<Uuid>,
    /// The published layout, shown on the website; `null`: never published.
    #[schema(value_type = Option<Object>)]
    layout: Option<Value>,
    /// The draft being edited; publish it with `POST /site-regions/{id}/publish`.
    #[schema(value_type = Option<Object>)]
    draft_layout: Option<Value>,
    #[schema(value_type = Option<String>, format = DateTime)]
    published_at: Option<Timestamp>,
    #[schema(value_type = String, format = DateTime)]
    updated_at: Timestamp,
}

fn response(row: SiteRegion) -> SiteRegionResponse {
    let parse = |text: Option<String>| text.and_then(|text| serde_json::from_str(&text).ok());
    SiteRegionResponse {
        id: row.id,
        org_id: row.org_id,
        region: row.region,
        locale: row.locale,
        variant_id: Uuid::parse_str(&row.variant_slot).ok(),
        layout: parse(row.layout),
        draft_layout: parse(row.draft_layout),
        published_at: row.published_at,
        updated_at: row.updated_at,
    }
}

/// Which header or footer.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct RegionQuery {
    /// The website (organization unit).
    org_id: Uuid,
    /// `header` or `footer`.
    region: String,
    /// A language of the website.
    locale: String,
    /// A theme variant with its own header or footer; left out: the base theme's.
    variant_id: Option<Uuid>,
}

impl From<RegionQuery> for RegionKey {
    fn from(query: RegionQuery) -> Self {
        RegionKey {
            org_id: query.org_id,
            region: query.region,
            locale: query.locale,
            variant_id: query.variant_id,
        }
    }
}

/// A new draft for a header or footer.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SiteRegionRequest {
    /// A block layout, checked like a page layout (`content.logo`, `menus.menu`,
    /// `content.search_box`, `content.announcement_bar`, `content.placement`, `newsletter.signup` suit
    /// headers and footers). The legal footer is added below the footer by itself.
    #[schema(value_type = Object)]
    layout: Value,
}

/// Get a website header or footer.
///
/// Requires `content.themes.manage` at the unit. `data` is `null` when none was made yet for this
/// unit, language and variant (the website then inherits the one of a unit above, or the base
/// theme's).
#[utoipa::path(
    get,
    path = "/site-regions",
    extensions(("x-permissions" = json!(["content.themes.manage"]))),
    operation_id = "get_content_site_region",
    security(("bearer" = [])),
    params(RegionQuery),
    responses(
        (status = 200, description = "The header or footer, or `null`.", body = Data<Option<SiteRegionResponse>>, headers(("ETag" = String, description = "Version: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.themes.manage` at the unit; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 404, description = "No unit or variant has this id; code `site_not_found` or `content.theme_variant_not_found`.", body = Problem),
        (status = 422, description = "The region or language is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn get_site_region(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<RegionQuery>,
) -> Result<Tagged<Option<SiteRegionResponse>>, Error> {
    let row = service.site_region(&tenant, &principal, &query.into()).await?;
    let version = row.as_ref().map_or(Timestamp::UNIX_EPOCH, |row| row.updated_at);
    Ok(Tagged { version, data: row.map(response) })
}

/// Save a draft of a website header or footer.
///
/// Requires `content.themes.manage` at the unit; `If-Match` is required (`*` for the first draft).
/// The website keeps showing the published layout until `POST /site-regions/{id}/publish`.
#[utoipa::path(
    put,
    path = "/site-regions",
    extensions(("x-permissions" = json!(["content.themes.manage"]))),
    operation_id = "put_content_site_region",
    security(("bearer" = [])),
    params(RegionQuery, ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`). Otherwise `412`.")),
    request_body = SiteRegionRequest,
    responses(
        (status = 200, description = "The header or footer with its new draft.", body = Data<SiteRegionResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.themes.manage` at the unit; code `permission_denied`.", body = Problem),
        (status = 404, description = "No unit or variant has this id; code `site_not_found` or `content.theme_variant_not_found`.", body = Problem),
        (status = 412, description = "It changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "The layout, region or language is invalid; see `errors` (layout paths start with `layout`).", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn put_site_region(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<RegionQuery>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<SiteRegionRequest>,
) -> Result<Tagged<SiteRegionResponse>, Error> {
    let expected = if_match.required()?;
    let row = service
        .put_site_region(&tenant, &principal, &query.into(), &input.layout, expected)
        .await?;
    Ok(Tagged { version: row.updated_at, data: response(row) })
}

/// Publish the draft of a website header or footer.
///
/// Requires `content.themes.manage` at the unit; `If-Match` is required. Every page of the website
/// shows it from now on (other servers within 30 seconds).
#[utoipa::path(
    post,
    path = "/site-regions/{id}/publish",
    extensions(("x-permissions" = json!(["content.themes.manage"]))),
    operation_id = "publish_content_site_region",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the header or footer."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    responses(
        (status = 200, description = "Published.", body = Data<SiteRegionResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.themes.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No header or footer has this id; code `content.site_region_not_found`.", body = Problem),
        (status = 409, description = "There is no draft; code `content.site_region_no_draft`.", body = Problem),
        (status = 412, description = "It changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn publish_site_region(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<Tagged<SiteRegionResponse>, Error> {
    let expected = if_match.required()?;
    let row = service.publish_site_region(&tenant, &principal, id, expected).await?;
    Ok(Tagged { version: row.updated_at, data: response(row) })
}

/// The language of the website frame.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PublicSiteQuery {
    /// A language of the website; left out: its default.
    locale: Option<String>,
}

/// The frame of every page of the website.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PublicSiteResponse {
    locale: String,
    /// Every theme token, flat (`color.primary` → set it as the CSS variable `--color-primary`): the
    /// website's theme with the running theme variant's overrides.
    #[schema(value_type = Object)]
    tokens: Value,
    /// Decoration of the running variant (`snow`, `petals`, `lanterns`, `fireworks`, `confetti`,
    /// `hearts`) or `null`. Load it after the page shows; never on cart, checkout, account or
    /// sign-in pages; off when the visitor asks for reduced motion (`docs/PAGES.md`).
    effect: Option<String>,
    effect_intensity: Option<String>,
    /// The running theme variant (`id`, `name`), or `null`.
    #[schema(value_type = Option<Object>)]
    variant: Option<Value>,
    /// When the look changes next (a variant starts or ends), within 7 days.
    #[schema(value_type = Option<String>, format = DateTime)]
    next_change_at: Option<String>,
    /// `{"layout", "blocks_data"}` like a page (`blocks_data` by block id), or `null`.
    #[schema(value_type = Option<Object>)]
    header: Option<Value>,
    #[schema(value_type = Option<Object>)]
    footer: Option<Value>,
    /// Owner information and ministry logo, drawn below the footer on every page; `null` when the
    /// website has no legal profile or the `compliance` module is off.
    #[schema(value_type = Option<Object>)]
    legal_footer: Option<Value>,
    /// Parts of other modules, by module name (e.g. cookie consent settings).
    #[schema(value_type = Object)]
    extensions: Value,
}

/// Get the frame of the website (public).
///
/// No sign-in needed. One call for what every page shares: theme tokens (with the running seasonal
/// variant), decoration, header, footer, legal footer and other modules' parts. Call it once and
/// reuse it for every page; `GET /public/resolve` does not repeat it. `Cache-Control: max-age` ends
/// exactly when the theme changes next (at most 300 seconds); `ETag` / `If-None-Match` give 304.
#[utoipa::path(
    get,
    path = "/public/site",
    extensions(("x-public" = json!(true))),
    operation_id = "get_content_public_site",
    security((), ("bearer" = [])),
    params(PublicSiteQuery, SiteQuery),
    responses(
        (status = 200, description = "The website frame.", body = Data<PublicSiteResponse>, headers(("ETag" = String, description = "Fingerprint of the answer; send it in `If-None-Match` to get 304."), ("Cache-Control" = String, description = "`public, max-age=<seconds until the theme changes, at most 300>`."))),
        (status = 304, description = "Not changed since the `ETag` sent in `If-None-Match`."),
        (status = 422, description = "The language is not one of the website; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn public_site(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    site: Site,
    headers: HeaderMap,
    Query(query): Query<PublicSiteQuery>,
) -> Result<Response, Error> {
    let at = now();
    let frame = service.public_site(&tenant, &site, query.locale.as_deref(), at).await?;
    let mut response = if autoxide_kernel::version::if_none_match(&headers, &frame.etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Data { data: &frame.body }.into_response()
    };
    let out = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&frame.etag) {
        out.insert(header::ETAG, value);
    }
    if let Ok(value) = HeaderValue::from_str(&format!("public, max-age={}", frame.max_age(at))) {
        out.insert(header::CACHE_CONTROL, value);
    }
    Ok(response)
}
