//! Endpoint công khai (không cần đăng nhập) cho website bán hàng.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::Problem;
use autoxide_kernel::tenancy::{Site, SiteQuery, TenantContext};
use axum::extract::{Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::public::PublicPromotion;
use crate::service::PromotionsService;

pub(crate) fn routes(service: Arc<PromotionsService>) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(list_public_promotions)).with_state(service)
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
    let locale = query
        .locale
        .as_deref()
        .and_then(autoxide_kernel::i18n::canonical_locale)
        .unwrap_or_default();
    let found = service.public_promotions(&tenant, &site, &locale, &page).await?;
    let body = serde_json::to_string(&found).map_err(Error::internal)?;
    // ETag theo nội dung (kèm khách): client đã có bản này nhận 304 qua lớp HTTP của kernel.
    let etag = format!(
        "\"{}\"",
        autoxide_kernel::secret::sha256(&format!("{}:{body}", tenant.id().as_uuid()))
            .get(..32)
            .unwrap_or_default()
    );
    let mut response = found.into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, no-cache"));
    Ok(response)
}
