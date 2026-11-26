//! Gợi ý trên trang sản phẩm (công khai): sản phẩm tương tự và thường mua cùng (B-c14).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageRequest};
use autoxide_kernel::response::Problem;
use autoxide_kernel::tenancy::{Site, SiteQuery, TenantContext};
use axum::extract::{Path, Query, State};
use axum::response::Response;
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::http::public::{PublicCardResponse, cached, cards};
use crate::recommend::Recommendation;
use crate::service::CatalogService;

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(similar_products))
        .routes(routes!(bought_together))
        .with_state(service)
}

/// Parameters of the product page recommendations.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct RecommendationQuery {
    /// Language of the texts; only products written in it are returned.
    #[param(example = "vi")]
    pub locale: String,
}

async fn recommend(
    service: &CatalogService,
    tenant: &TenantContext,
    site: &Site,
    id: Uuid,
    query: RecommendationQuery,
    page: NaturalOrderPageQuery,
    kind: Recommendation,
) -> Result<Response, Error> {
    let page = PageRequest::try_from(page)?;
    let locale = service.locale(tenant, &query.locale).await?;
    let rows = service.recommendations(tenant, site, id, &locale, kind, &page).await?;
    let data = cards(service, tenant, site, &locale, &rows.data).await?;
    cached(tenant, Page { data, pagination: rows.pagination })
}

/// List products similar to a product (public).
///
/// No sign-in needed. Products on sale on the website asking whose texts mean something close
/// to this product's (semantic search of the AI module), most similar first. When there is no
/// such list yet (AI embeddings off or not indexed), products of the same categories, best
/// sellers first. A short ranked list (at most 12); paged with `limit` / `cursor` (`limit=8` suits
/// a product page).
#[utoipa::path(
    get,
    path = "/public/products/{id}/similar",
    extensions(("x-public" = json!(true))),
    operation_id = "list_similar_catalog_products",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "ID of the product."), RecommendationQuery, SiteQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of similar products (maybe none).", body = Page<PublicCardResponse>, headers(("ETag" = String, description = "Fingerprint of the whole answer; send it in `If-None-Match` to get 304."))),
        (status = 404, description = "The product is not on sale on this website; code `catalog.product_not_found`. The `site` is not a unit of this tenant; code `site_not_found`.", body = Problem),
        (status = 422, description = "The language is not supported (`catalog.unsupported_locale`) or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn similar_products(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    site: Site,
    _principal: Option<Principal>,
    Path(id): Path<Uuid>,
    Query(query): Query<RecommendationQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Response, Error> {
    recommend(&service, &tenant, &site, id, query, page, Recommendation::Similar).await
}

/// List products frequently bought together with a product (public).
///
/// No sign-in needed. Products on sale on the website asking that were in the same completed
/// orders as this product over the last 90 days, most orders first. A short ranked list (at most
/// 20); paged with `limit` / `cursor` (`limit=8` suits a product page).
#[utoipa::path(
    get,
    path = "/public/products/{id}/frequently-bought-together",
    extensions(("x-public" = json!(true))),
    operation_id = "list_catalog_products_bought_together",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "ID of the product."), RecommendationQuery, SiteQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of products bought together (maybe none).", body = Page<PublicCardResponse>, headers(("ETag" = String, description = "Fingerprint of the whole answer; send it in `If-None-Match` to get 304."))),
        (status = 404, description = "The product is not on sale on this website; code `catalog.product_not_found`. The `site` is not a unit of this tenant; code `site_not_found`.", body = Problem),
        (status = 422, description = "The language is not supported (`catalog.unsupported_locale`) or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn bought_together(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    site: Site,
    _principal: Option<Principal>,
    Path(id): Path<Uuid>,
    Query(query): Query<RecommendationQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Response, Error> {
    recommend(&service, &tenant, &site, id, query, page, Recommendation::BoughtTogether).await
}
