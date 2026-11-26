//! Endpoint công khai (không cần đăng nhập) cho website bán hàng.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest};
use autoxide_kernel::response::Problem;
use autoxide_kernel::tenancy::{Site, SiteQuery, TenantContext};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::for_product::ProductCode;
use crate::public::{PublicFlashSale, PublicPromotion};
use crate::service::PromotionsService;

pub(crate) fn routes(service: Arc<PromotionsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_public_promotions))
        .routes(routes!(list_public_flash_sales))
        .routes(routes!(codes_for_product))
        .with_state(service)
}

/// Language of the texts.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PublicQuery {
    /// Language of `title` and `terms`, e.g. `vi`. Omitted or not offered: the first language
    /// the promotion has.
    #[param(example = "vi")]
    pub locale: Option<String>,
}

/// List the promotions running now on this website (public).
///
/// No sign-in. Promotions marked `public` of the website's unit and of the units above it, with
/// their full conditions, the notices recorded with the Departments of Industry and Trade
/// (authority and reference) and the largest discount the law in force today allows. Oldest
/// first; follow `pagination.next_cursor` for more.
#[utoipa::path(
    get,
    path = "/public/promotions",
    extensions(("x-public" = json!(true))),
    operation_id = "list_public_promotions",
    security((), ("bearer" = [])),
    params(PublicQuery, PageQuery, SiteQuery),
    responses(
        (status = 200, description = "One page of running promotions.", body = Page<PublicPromotion>, headers(("ETag" = String, description = "Fingerprint of the whole answer; send it in `If-None-Match` to get 304."))),
        (status = 404, description = "The `site` is not a unit of this tenant; code `site_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_public_promotions(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    site: Site,
    Query(query): Query<PublicQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Response, Error> {
    let page = PageRequest::try_from(paging)?;
    let locale = locale_of(query.locale.as_deref());
    let found = service.public_promotions(&tenant, &site, &locale, &page).await?;
    cached(&tenant, found)
}

/// The variant and language asked for.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ForProductQuery {
    /// The variant shown (required: prices are per variant).
    pub variant_id: uuid::Uuid,
    /// Language of `title` and `terms`, e.g. `vi`.
    #[param(example = "vi")]
    pub locale: Option<String>,
}

/// List the public coupons that lower the price of a product (public).
///
/// No sign-in. Coupons of running public promotions of this website that apply to the variant,
/// each with the price of one unit with the coupon, its conditions and the uses left (only when
/// there is a real limit). Cheapest first; empty when none applies. The listed price stays the
/// price without a coupon: show these as "X with code …", not as a discounted price. Discounts
/// that need no coupon are already in the catalog price. Paged with `limit` / `cursor`
/// (`order=desc` puts the dearest first).
#[utoipa::path(
    get,
    path = "/public/promotions/for-product/{product_id}",
    extensions(("x-public" = json!(true))),
    operation_id = "list_public_codes_for_product",
    security((), ("bearer" = [])),
    params(("product_id" = Uuid, Path, description = "Id of the product."), ForProductQuery, SiteQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the coupons, cheapest first.", body = Page<ProductCode>),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
        (status = 404, description = "The variant is not of this product (`promotions.product_not_found`) or the `site` is not a unit of this tenant (`site_not_found`).", body = Problem),
    )
)]
async fn codes_for_product(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    site: Site,
    Path(product_id): Path<uuid::Uuid>,
    Query(query): Query<ForProductQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<ProductCode>, Error> {
    let page = PageRequest::try_from(page)?;
    let locale = locale_of(query.locale.as_deref());
    service.codes_for_product(&tenant, &site, product_id, query.variant_id, &locale, &page).await
}

/// List the public flash sales running now or starting within a day (public).
///
/// No sign-in. Flash sales marked `public` of the website's unit and of the units above it, each
/// `running` or `upcoming`, with their products: the sale price and the units left at that price
/// (a real count, `null` when not limited). Oldest first; follow `pagination.next_cursor` for more.
#[utoipa::path(
    get,
    path = "/public/promotions/flash-sales",
    extensions(("x-public" = json!(true))),
    operation_id = "list_public_flash_sales",
    security((), ("bearer" = [])),
    params(PublicQuery, PageQuery, SiteQuery),
    responses(
        (status = 200, description = "One page of flash sales.", body = Page<PublicFlashSale>, headers(("ETag" = String, description = "Fingerprint of the whole answer; send it in `If-None-Match` to get 304."))),
        (status = 404, description = "The `site` is not a unit of this tenant; code `site_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_public_flash_sales(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    site: Site,
    Query(query): Query<PublicQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Response, Error> {
    let page = PageRequest::try_from(paging)?;
    let locale = locale_of(query.locale.as_deref());
    let found = service.public_flash_sales(&tenant, &site, &locale, &page).await?;
    cached(&tenant, found)
}

/// Ngôn ngữ hỏi, chuẩn hoá; sai hay không có thì chuỗi rỗng (lấy ngôn ngữ đầu của chương trình).
fn locale_of(raw: Option<&str>) -> String {
    raw.and_then(autoxide_kernel::i18n::canonical_locale).unwrap_or_default()
}

/// Trả `value` kèm `ETag` theo nội dung (kèm khách) và `Cache-Control` công khai: client đã có bản
/// này nhận 304 qua lớp HTTP của kernel.
fn cached<T: serde::Serialize + IntoResponse>(
    tenant: &TenantContext,
    value: T,
) -> Result<Response, Error> {
    let body = serde_json::to_string(&value).map_err(Error::internal)?;
    let digest = autoxide_kernel::secret::sha256(&format!("{}:{body}", tenant.id().as_uuid()));
    let etag = format!("\"{}\"", digest.get(..32).unwrap_or_default());
    let mut response = value.into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, no-cache"));
    Ok(response)
}
