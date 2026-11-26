//! Tìm cho website (D-201): gợi ý khi gõ (gom mọi module), số đếm theo giá trị lọc (facet) và từ
//! đồng nghĩa theo khách.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::PageQuery;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::suggest::MAX_SUGGESTIONS;
use autoxide_kernel::tenancy::{Site, SiteQuery, TenantContext};
use axum::extract::{Query, State};
use axum::response::Response;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::facets::FacetValue;
use crate::http::public::{PublicProductsQuery, cached, parse_filter};
use crate::permissions::CATEGORIES_MANAGE;
use crate::public::QUERY_MAX;
use crate::service::CatalogService;

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(suggest))
        .routes(routes!(product_facets))
        .routes(routes!(get_synonyms, put_synonyms))
        .with_state(service)
}

/// What to suggest for.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SuggestQuery {
    /// What the shopper has typed so far, at most 200 characters.
    pub q: String,
    /// Language of the suggestions.
    #[param(example = "vi")]
    pub locale: String,
    /// At most this many suggestions, 1 to 10 (default 10; 8 suits a phone).
    pub limit: Option<u16>,
}

/// A search suggestion.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogSuggestion)]
pub(crate) struct SuggestionResponse {
    /// Type, `<module>.<type>`: `catalog.product`, `catalog.category`, `content.entry`,
    /// `organizations.unit`… (any module may add its own).
    pub kind: String,
    /// Text to show.
    pub label: String,
    /// Address on the website (no domain, no language prefix); `null`: open the record by `id`.
    pub path: Option<String>,
    pub id: Option<Uuid>,
    /// Matches with the exact Vietnamese accents (listed first); `false`: matches only once accents
    /// are ignored.
    pub exact: bool,
    /// Short context, e.g. the parent category; may be empty.
    pub context: String,
}

/// Suggest as the shopper types (public).
///
/// No sign-in needed. Products on sale on the website asking, categories, and what other modules
/// add (posts, stores…), gathered in one list: matches with the exact accents first, then matches
/// ignoring accents; the tenant's synonyms count. A module that fails is left out of the answer.
#[utoipa::path(
    get,
    path = "/public/search/suggest",
    extensions(("x-public" = json!(true))),
    operation_id = "suggest_public_catalog_search",
    security((), ("bearer" = [])),
    params(SuggestQuery, SiteQuery),
    responses(
        (status = 200, description = "Suggestions, best first.", body = Data<Vec<SuggestionResponse>>, headers(("ETag" = String, description = "Fingerprint of the whole answer; send it in `If-None-Match` to get 304."))),
        (status = 422, description = "`q` is empty or too long, `limit` is out of range, or the language is not supported; see `errors`.", body = Problem),
    )
)]
async fn suggest(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    site: Site,
    _principal: Option<Principal>,
    Query(query): Query<SuggestQuery>,
) -> Result<Response, Error> {
    let locale = service.locale(&tenant, &query.locale).await?;
    let mut problems = Vec::new();
    let length = query.q.trim().chars().count();
    if length == 0 || length > QUERY_MAX {
        problems.push(
            FieldError::new("q", "length", "must be 1 to {max} characters")
                .with_param("max", QUERY_MAX),
        );
    }
    let limit = query.limit.unwrap_or(MAX_SUGGESTIONS);
    if !(1..=MAX_SUGGESTIONS).contains(&limit) {
        problems.push(
            FieldError::new("limit", "range", "must be 1 to {max}")
                .with_param("max", MAX_SUGGESTIONS),
        );
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let found = service.suggestions.suggest(&tenant, &site, &locale, &query.q, limit).await;
    let data: Vec<SuggestionResponse> = found
        .into_iter()
        .map(|item| SuggestionResponse {
            kind: item.kind,
            label: item.label,
            path: item.path,
            id: item.id,
            exact: item.exact,
            context: item.context,
        })
        .collect();
    cached(&tenant, Data { data })
}

/// A filter value and how many products match it.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogFacetValue)]
pub(crate) struct FacetValueResponse {
    /// Value to send back in the filter: an option code, `true` / `false`, or a seller unit id.
    pub value: String,
    pub label: String,
    pub count: u64,
}

impl From<FacetValue> for FacetValueResponse {
    fn from(item: FacetValue) -> Self {
        FacetValueResponse { value: item.value, label: item.label, count: item.count }
    }
}

/// Counts of one filterable attribute.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogAttributeFacet)]
pub(crate) struct AttributeFacetResponse {
    /// Filter with `attr.<code>=<value>`.
    pub code: String,
    pub name: String,
    /// At most 50, most products first.
    pub values: Vec<FacetValueResponse>,
}

/// Filter counts of a public product list.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogFacets)]
pub(crate) struct FacetsResponse {
    /// Products matching every filter.
    pub total: u64,
    /// Filterable attributes of type option, options or boolean.
    pub attributes: Vec<AttributeFacetResponse>,
    /// Sellers (filter with `seller=<value>`), at most 50, most products first.
    pub sellers: Vec<FacetValueResponse>,
}

/// Count products per filter value (public).
///
/// No sign-in needed. Takes the same filters as `GET /public/products`. The counts of one filter
/// are taken with every other filter applied but not that one, so choosing another value of it
/// widens the list as the count says.
#[utoipa::path(
    get,
    path = "/public/products/facets",
    extensions(("x-public" = json!(true))),
    operation_id = "count_public_catalog_facets",
    security((), ("bearer" = [])),
    params(PublicProductsQuery, SiteQuery),
    responses(
        (status = 200, description = "The counts.", body = Data<FacetsResponse>, headers(("ETag" = String, description = "Fingerprint of the whole answer; send it in `If-None-Match` to get 304."))),
        (status = 422, description = "A filter is invalid; see `errors`.", body = Problem),
    )
)]
async fn product_facets(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    site: Site,
    _principal: Option<Principal>,
    Query(query): Query<PublicProductsQuery>,
    Query(raw): Query<Vec<(String, String)>>,
) -> Result<Response, Error> {
    let mut paging = PageQuery::default();
    let filter = parse_filter(&service, &tenant, query, raw, &mut paging).await?;
    let facets = service.public_facets(&tenant, &site, &filter).await?;
    let data = FacetsResponse {
        total: facets.total,
        attributes: facets
            .attributes
            .into_iter()
            .map(|attribute| AttributeFacetResponse {
                code: attribute.code,
                name: attribute.name,
                values: attribute.values.into_iter().map(Into::into).collect(),
            })
            .collect(),
        sellers: facets.sellers.into_iter().map(Into::into).collect(),
    };
    cached(&tenant, Data { data })
}

/// The tenant's search synonyms.
#[derive(Debug, Serialize, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogSynonyms)]
pub(crate) struct SynonymsBody {
    /// Groups of words meaning the same thing, e.g. `[["áo thun", "áo phông", "tee"]]`: searching
    /// any word of a group also finds the others. At most 500 groups of 2 to 20 words; words are
    /// stored without accents, in lower case.
    pub groups: Vec<Vec<String>>,
}

/// List search synonyms.
///
/// Requires `catalog.categories.manage`.
#[utoipa::path(
    get,
    path = "/synonyms",
    extensions(("x-permissions" = json!(["catalog.categories.manage"]))),
    operation_id = "get_catalog_synonyms",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every synonym group.", body = Data<SynonymsBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.categories.manage`; code `permission_denied`.", body = Problem),
    )
)]
async fn get_synonyms(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<SynonymsBody>, Error> {
    principal.require(&CATEGORIES_MANAGE)?;
    let groups = service.synonym_groups(&tenant).await?;
    Ok(Data { data: SynonymsBody { groups } })
}

/// Replace search synonyms.
///
/// Requires `catalog.categories.manage`. Replaces every group; send the whole set. Public search
/// and suggestions use them at once.
#[utoipa::path(
    put,
    path = "/synonyms",
    extensions(("x-permissions" = json!(["catalog.categories.manage"]))),
    operation_id = "put_catalog_synonyms",
    security(("bearer" = [])),
    request_body = SynonymsBody,
    responses(
        (status = 200, description = "The groups as stored (without accents, lower case).", body = Data<SynonymsBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.categories.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Too many groups, or a group with fewer than 2 or more than 20 words, or a word that is empty or too long; see `errors`.", body = Problem),
    )
)]
async fn put_synonyms(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(body): ValidJson<SynonymsBody>,
) -> Result<Data<SynonymsBody>, Error> {
    let groups = service.put_synonym_groups(&tenant, &principal, body.groups).await?;
    Ok(Data { data: SynonymsBody { groups } })
}
