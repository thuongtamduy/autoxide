//! Đọc công khai vị trí banner (bước 55, D-236): không cần đăng nhập; khách theo tên miền /
//! header, website theo `site`. `Cache-Control: public, max-age=<tới mốc gần nhất>` và `ETag` (→ 304).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{Site, SiteQuery, TenantContext};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::{IntoParams, ToSchema};

use super::placements::Ratio;
use super::public::{PublicMedia, media};
use crate::banners::public::{PageContext, PublicPlacements, ReadyBanner, ReadyPlacement};
use crate::service::ContentService;

/// A placement ready to draw, with its banners for the page asked about.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicPlacement {
    #[schema(example = "home_hero")]
    key: String,
    /// `single`, `carousel`, `grid`, `stack`, `strip` or `popup`; draw it as `docs/CONTENT.md`
    /// says (a popup always has a close button; no autoplay when the viewer asks for reduced
    /// motion).
    display: String,
    #[schema(value_type = Object)]
    display_settings: Value,
    /// `image`, `text` or `any`.
    accepts: String,
    ratio: Option<Ratio>,
    mobile_ratio: Option<Ratio>,
    /// `all`, `desktop` or `mobile`.
    devices: String,
    /// Banners to show, in this order (never shuffle them).
    banners: Vec<PublicBanner>,
}

/// A banner to show.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicBanner {
    id: uuid::Uuid,
    /// `image` or `text`.
    kind: String,
    title: String,
    subtitle: String,
    cta_label: String,
    /// Text banners: a colour token of the theme.
    tone: Option<String>,
    /// Image banners: the picture, with smaller copies and WebP for `srcset`.
    image: Option<PublicMedia>,
    /// Picture for phones, when it has one.
    mobile_image: Option<PublicMedia>,
    /// Where it links to: a path on the website or an address; `null`: no link.
    href: Option<String>,
    /// The text was written from an AI suggestion: label it.
    ai_assisted: bool,
}

/// Several placements of one page.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicPlacementsResponse {
    /// The language returned.
    locale: String,
    /// The placements asked for that exist and are on, in the order asked.
    placements: Vec<PublicPlacement>,
}

/// The page being shown and its language.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PageQuery {
    /// Omitted: the website's default language.
    #[param(example = "vi")]
    locale: Option<String>,
    /// Kind of the page: `home`, `category`, `collection`, `product`, `search`, `cart`,
    /// `content_entry`, `content_type` or `landing`. Omitted: only banners not limited to some
    /// pages.
    #[param(example = "category")]
    page_kind: Option<String>,
    /// The page itself, as `<kind>:<id>` of the object directory (e.g.
    /// `catalog.category:0190…`): banners chosen for that page show too.
    page_ref: Option<String>,
}

/// Which placements to read.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct KeysQuery {
    /// Keys of the placements, comma-separated, at most 10.
    #[param(example = "top_strip,home_hero,home_middle")]
    keys: String,
}

fn page(query: &PageQuery) -> PageContext {
    PageContext {
        kind: query.page_kind.clone().filter(|kind| !kind.is_empty()),
        reference: query.page_ref.as_deref().and_then(|raw| {
            let (kind, id) = raw.rsplit_once(':')?;
            Some((kind.to_owned(), id.to_owned()))
        }),
        locale: query.locale.clone(),
    }
}

fn banner(ready: ReadyBanner) -> PublicBanner {
    let row = ready.row;
    PublicBanner {
        id: row.id,
        image: media(ready.media, &row.alt),
        mobile_image: media(ready.mobile_media, &row.alt),
        href: ready.href,
        kind: row.kind,
        title: row.title,
        subtitle: row.subtitle,
        cta_label: row.cta_label,
        tone: row.tone,
        ai_assisted: row.ai_assisted,
    }
}

fn placement(ready: ReadyPlacement) -> PublicPlacement {
    let row = ready.placement;
    let ratio = |w: Option<i32>, h: Option<i32>| Some(Ratio::new(w?, h?));
    PublicPlacement {
        ratio: ratio(row.ratio_w, row.ratio_h),
        mobile_ratio: ratio(row.mobile_ratio_w, row.mobile_ratio_h),
        display_settings: serde_json::from_str(&row.display_settings)
            .unwrap_or_else(|_| Value::Object(Map::new())),
        banners: ready.banners.into_iter().map(banner).collect(),
        key: row.key,
        display: row.display,
        accepts: row.accepts,
        devices: row.devices,
    }
}

/// Câu trả lời công khai: `ETag` là dấu vân tay của thân (cùng khách), `max-age` tới mốc gần nhất.
fn respond<T: Serialize>(
    tenant: &TenantContext,
    data: T,
    result_max_age: i64,
    headers: &axum::http::HeaderMap,
) -> Result<Response, Error> {
    let body = Data { data };
    let etag = fingerprint(tenant, &body)?;
    let mut response = if autoxide_kernel::version::if_none_match(headers, &etag) {
        axum::http::StatusCode::NOT_MODIFIED.into_response()
    } else {
        body.into_response()
    };
    let cache = if result_max_age > 0 {
        format!("public, max-age={result_max_age}")
    } else {
        "public, no-cache".to_owned()
    };
    let out = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&etag) {
        out.insert(header::ETAG, value);
    }
    if let Ok(value) = HeaderValue::from_str(&cache) {
        out.insert(header::CACHE_CONTROL, value);
    }
    Ok(response)
}

fn fingerprint<T: Serialize>(tenant: &TenantContext, value: &T) -> Result<String, Error> {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(value)
        .map_err(|error| Error::internal(format!("cannot serialize the banners: {error}")))?;
    let mut hasher = Sha256::new();
    hasher.update(tenant.id().as_uuid().as_bytes());
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let short = digest.get(..16).unwrap_or_default();
    Ok(format!("\"{}\"", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(short)))
}

/// Read several placements of a page (public).
///
/// No sign-in needed. One call for every placement of the page (at most 10 keys). Each
/// placement comes with how to draw it and its banners for this page: `active`, in their time
/// window now, of the website and the units above it, in the page's language, in a fixed order.
/// Banners whose picture or link target is no longer public are left out. Keys that do not exist
/// or are switched off are left out. `Cache-Control` lets the answer be reused only until the
/// next banner starts or ends (at most 60 seconds).
#[utoipa::path(
    get,
    path = "/public/placements",
    extensions(("x-public" = json!(true))),
    operation_id = "list_public_content_placements",
    security((), ("bearer" = [])),
    params(KeysQuery, PageQuery, SiteQuery),
    responses(
        (status = 200, description = "The placements.", body = Data<PublicPlacementsResponse>, headers(("ETag" = String, description = "Fingerprint of the answer; send it in `If-None-Match` to get 304."), ("Cache-Control" = String, description = "`public, max-age=<seconds>` until the next banner starts or ends, at most 60."))),
        (status = 304, description = "Not changed since the `ETag` sent in `If-None-Match`."),
        (status = 422, description = "More than 10 keys (code `content.placement_keys_too_many`), or a language or page kind that is not supported; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_public_placements(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    site: Site,
    headers: axum::http::HeaderMap,
    Query(keys): Query<KeysQuery>,
    Query(query): Query<PageQuery>,
) -> Result<Response, Error> {
    let keys: Vec<String> = keys.keys.split(',').map(str::to_owned).collect();
    let found =
        service.public_placements(&tenant, &site, &keys, &page(&query), Utc::now(), false).await?;
    let PublicPlacements { locale, placements, max_age } = found;
    let data = PublicPlacementsResponse {
        locale,
        placements: placements.into_iter().map(placement).collect(),
    };
    respond(&tenant, data, max_age, &headers)
}

/// Read one placement (public).
///
/// No sign-in needed. Same rules as `GET /api/v1/content/public/placements`, for one key; prefer
/// that endpoint to read every placement of a page in one call.
#[utoipa::path(
    get,
    path = "/public/placements/{key}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_content_placement",
    security((), ("bearer" = [])),
    params(("key" = String, Path, description = "Key of the placement, e.g. `home_hero`."), PageQuery, SiteQuery),
    responses(
        (status = 200, description = "The placement.", body = Data<PublicPlacement>, headers(("ETag" = String, description = "Fingerprint of the answer; send it in `If-None-Match` to get 304."), ("Cache-Control" = String, description = "`public, max-age=<seconds>` until the next banner starts or ends, at most 60."))),
        (status = 304, description = "Not changed since the `ETag` sent in `If-None-Match`."),
        (status = 404, description = "No active placement has this key; code `content.placement_not_found`.", body = Problem),
        (status = 422, description = "A language or page kind that is not supported; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn get_public_placement(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    site: Site,
    headers: axum::http::HeaderMap,
    Path(key): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<Response, Error> {
    let found =
        service.public_placements(&tenant, &site, &[key], &page(&query), Utc::now(), true).await?;
    let max_age = found.max_age;
    let ready = found
        .placements
        .into_iter()
        .next()
        .ok_or_else(|| Error::internal("a strict read returned no placement"))?;
    respond(&tenant, placement(ready), max_age, &headers)
}
