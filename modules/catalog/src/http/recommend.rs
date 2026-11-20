//! Gợi ý trên trang sản phẩm (công khai): sản phẩm tương tự và thường mua cùng (B-c14).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::response::{Data, Problem};
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

/// Số thẻ mặc định và tối đa mỗi lần.
const LIMIT_DEFAULT: u8 = 8;
const LIMIT_MAX: u8 = 12;

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
    /// How many products, 1 to 12 (default 8).
    #[param(minimum = 1, maximum = 12)]
    pub limit: Option<u8>,
}

async fn recommend(
    service: &CatalogService,
    tenant: &TenantContext,
    site: &Site,
    id: Uuid,
    query: RecommendationQuery,
    kind: Recommendation,
) -> Result<Response, Error> {
    let locale = service.locale(&query.locale)?;
    let limit = query.limit.unwrap_or(LIMIT_DEFAULT);
    if !(1..=LIMIT_MAX).contains(&limit) {
        return Err(Error::validation(vec![FieldError::new(
            "limit",
            "range",
            "must be between 1 and 12",
        )]));
    }
    let rows = service.recommendations(tenant, site, id, &locale, kind, usize::from(limit)).await?;
    let data = cards(service, tenant, site, &locale, &rows).await?;
    cached(tenant, Data { data })
}

/// List products similar to a product (public).
///
/// No sign-in needed. Products on sale on the website asking whose texts mean something close
/// to this product's (semantic search of the AI module), most similar first. When there is no
/// such list yet (AI embeddings off or not indexed), products of the same categories, best
/// sellers first.
#[utoipa::path(
    get,
    path = "/public/products/{id}/similar",
    extensions(("x-public" = json!(true))),
    operation_id = "list_similar_catalog_products",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "ID of the product."), RecommendationQuery, SiteQuery),
    responses(
        (status = 200, description = "Similar products (maybe none).", body = Data<Vec<PublicCardResponse>>, headers(("ETag" = String, description = "Fingerprint of the whole answer; send it in `If-None-Match` to get 304."))),
        (status = 404, description = "The product is not on sale on this website; code `catalog.product_not_found`. The `site` is not a unit of this tenant; code `site_not_found`.", body = Problem),
        (status = 422, description = "The language is not supported (`catalog.unsupported_locale`) or `limit` is out of range; see `errors`.", body = Problem),
    )
)]
async fn similar_products(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    site: Site,
    _principal: Option<Principal>,
    Path(id): Path<Uuid>,
    Query(query): Query<RecommendationQuery>,
) -> Result<Response, Error> {
    recommend(&service, &tenant, &site, id, query, Recommendation::Similar).await
}

/// List products frequently bought together with a product (public).
///
/// No sign-in needed. Products on sale on the website asking that were in the same completed
/// orders as this product over the last 90 days, most orders first.
#[utoipa::path(
    get,
    path = "/public/products/{id}/frequently-bought-together",
    extensions(("x-public" = json!(true))),
    operation_id = "list_catalog_products_bought_together",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "ID of the product."), RecommendationQuery, SiteQuery),
    responses(
        (status = 200, description = "Products bought together (maybe none).", body = Data<Vec<PublicCardResponse>>, headers(("ETag" = String, description = "Fingerprint of the whole answer; send it in `If-None-Match` to get 304."))),
        (status = 404, description = "The product is not on sale on this website; code `catalog.product_not_found`. The `site` is not a unit of this tenant; code `site_not_found`.", body = Problem),
        (status = 422, description = "The language is not supported (`catalog.unsupported_locale`) or `limit` is out of range; see `errors`.", body = Problem),
    )
)]
async fn bought_together(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    site: Site,
    _principal: Option<Principal>,
    Path(id): Path<Uuid>,
    Query(query): Query<RecommendationQuery>,
) -> Result<Response, Error> {
    recommend(&service, &tenant, &site, id, query, Recommendation::BoughtTogether).await
}
