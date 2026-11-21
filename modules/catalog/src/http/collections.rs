//! Bộ sưu tập sản phẩm (bước 55 mục 3): quản trị dưới `/collections`, đọc công khai dưới
//! `/public/collections/{slug}`.

use autoxide_kernel::time::Timestamp;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{Site, SiteQuery, TenantContext};
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::public::{PublicCardResponse, cached, cards};
use crate::collections::service::{
    CollectionDetail, CollectionPatch, CollectionText, NewCollection,
};
use crate::service::CatalogService;

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_collections, create_collection))
        .routes(routes!(get_collection, update_collection, delete_collection))
        .routes(routes!(get_collection_items, put_collection_items))
        .routes(routes!(get_public_collection))
        .with_state(service)
}

/// Texts of a collection in one language.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCollectionText)]
pub(crate) struct CollectionTextDto {
    /// 1 to 200 characters.
    title: String,
    /// Address part, unique in the language; omitted: made from the title.
    #[serde(default)]
    slug: Option<String>,
    /// Up to 5000 characters.
    #[serde(default)]
    description: String,
    /// Up to 200 characters.
    #[serde(default)]
    meta_title: String,
    /// Up to 500 characters.
    #[serde(default)]
    meta_description: String,
}

impl From<CollectionTextDto> for CollectionText {
    fn from(text: CollectionTextDto) -> Self {
        CollectionText {
            title: text.title,
            slug: text.slug,
            description: text.description,
            meta_title: text.meta_title,
            meta_description: text.meta_description,
        }
    }
}

/// A product collection.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogCollection)]
pub(crate) struct CollectionResponse {
    id: Uuid,
    /// Organization unit that owns it; the tenant's id for the whole tenant.
    org_id: Uuid,
    /// `manual` (chosen products, in order) or `rule`.
    kind: String,
    /// The rule of a `rule` collection; `{}` for `manual`.
    #[schema(value_type = Object)]
    rules: Value,
    /// `manual` (chosen order), `newest`, `price_asc`, `price_desc` or `best_selling`.
    sort: String,
    /// Picture (public media item).
    media_id: Option<Uuid>,
    /// `draft`, `active` or `archived`; only `active` ones in their time window are public.
    status: String,
    starts_at: Option<Timestamp>,
    ends_at: Option<Timestamp>,
    /// Texts by language.
    #[schema(value_type = BTreeMap<String, CollectionTextDto>)]
    texts: BTreeMap<String, CollectionTextDto>,
    /// Number of chosen products (`manual`).
    items: u64,
    /// The texts were written with an AI's suggestion; show an AI label next to them.
    ai_assisted: bool,
    created_at: Timestamp,
    updated_at: Timestamp,
}

pub(crate) fn collection_response(detail: CollectionDetail) -> CollectionResponse {
    let row = detail.collection;
    CollectionResponse {
        id: row.id,
        org_id: row.org_id,
        kind: row.kind,
        rules: serde_json::from_str(&row.rules).unwrap_or(Value::Object(Default::default())),
        sort: row.sort,
        media_id: row.media_id,
        status: row.status,
        starts_at: row.starts_at,
        ends_at: row.ends_at,
        texts: detail
            .texts
            .into_iter()
            .map(|text| {
                (
                    text.locale,
                    CollectionTextDto {
                        title: text.title,
                        slug: Some(text.slug),
                        description: text.description,
                        meta_title: text.meta_title,
                        meta_description: text.meta_description,
                    },
                )
            })
            .collect(),
        items: detail.items,
        ai_assisted: row.ai_assisted,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// A new collection.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCreateCollection)]
pub(crate) struct CreateCollectionRequest {
    /// Owning organization unit; omitted: the whole tenant.
    org_id: Option<Uuid>,
    /// `manual` or `rule`.
    kind: String,
    /// For `rule`: `{"category_ids": [...], "attributes": {"color": ["red"]}, "price_min": 100000,
    /// "price_max": 500000, "new_within_days": 30, "on_promotion": true, "match": "all"}` — every
    /// condition must hold, at most 10 conditions; prices in the smallest unit of the default
    /// price list's currency; `on_promotion` needs module `promotions`.
    #[schema(value_type = Option<Object>)]
    rules: Option<Value>,
    /// Omitted: `manual` for a manual collection, else `newest`.
    sort: Option<String>,
    media_id: Option<Uuid>,
    /// `draft` (default), `active` or `archived`.
    status: Option<String>,
    starts_at: Option<Timestamp>,
    ends_at: Option<Timestamp>,
    /// Texts by language, at least one.
    #[schema(value_type = BTreeMap<String, CollectionTextDto>)]
    texts: BTreeMap<String, CollectionTextDto>,
    /// The texts were written with an AI's suggestion (default `false`); the public page then
    /// carries an AI label.
    #[serde(default)]
    ai_assisted: bool,
}

// Trường JSON có ba trạng thái: không gửi (`None`), `null` (`Some(None)`), có giá trị.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Changes to a collection: fields left out stay; `null` clears; `texts` replaces the languages
/// given.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogUpdateCollection)]
pub(crate) struct UpdateCollectionRequest {
    #[schema(value_type = Option<Object>)]
    rules: Option<Value>,
    sort: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>)]
    media_id: Option<Option<Uuid>>,
    status: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Timestamp>)]
    starts_at: Option<Option<Timestamp>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Timestamp>)]
    ends_at: Option<Option<Timestamp>>,
    #[serde(default)]
    #[schema(value_type = BTreeMap<String, CollectionTextDto>)]
    texts: BTreeMap<String, CollectionTextDto>,
    /// The texts were written with an AI's suggestion; omitted: unchanged.
    ai_assisted: Option<bool>,
}

fn texts(texts: BTreeMap<String, CollectionTextDto>) -> BTreeMap<String, CollectionText> {
    texts.into_iter().map(|(locale, text)| (locale, text.into())).collect()
}

/// Which collections to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct CollectionsQuery {
    /// Only `draft`, `active` or `archived`.
    status: Option<String>,
}

/// List product collections.
///
/// Requires `catalog.products.read`; only collections of units within reach.
#[utoipa::path(
    get,
    path = "/collections",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "list_catalog_collections",
    security(("bearer" = [])),
    params(CollectionsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of collections.", body = Page<CollectionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `catalog.products.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_collections(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<CollectionsQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<CollectionResponse>, Error> {
    let page = PageRequest::try_from(paging)?;
    let found =
        service.list_collections(&tenant, &principal, query.status.as_deref(), &page).await?;
    Ok(found.map(collection_response))
}

/// Create a product collection.
///
/// Requires `catalog.collections.manage` for its unit. Retry safely with an `Idempotency-Key`.
#[utoipa::path(
    post,
    path = "/collections",
    extensions(("x-permissions" = json!(["catalog.collections.manage"]))),
    operation_id = "create_catalog_collection",
    security(("bearer" = [])),
    request_body = CreateCollectionRequest,
    responses(
        (status = 201, description = "Created.", body = Data<CollectionResponse>, headers(("ETag" = String, description = "Version of the collection."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `catalog.collections.manage` for the unit; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 409, description = "A slug is taken in that language; code `catalog.slug_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; a bad rule gives `catalog.rule_invalid`, `on_promotion` without module `promotions` gives `catalog.rule_needs_module`; see `errors`.", body = Problem),
    )
)]
async fn create_collection(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateCollectionRequest>,
) -> Result<Created<CollectionResponse>, Error> {
    let new = NewCollection {
        org_id: input.org_id,
        kind: input.kind,
        rules: input.rules,
        sort: input.sort,
        media_id: input.media_id,
        status: input.status,
        starts_at: input.starts_at,
        ends_at: input.ends_at,
        texts: texts(input.texts),
        ai_assisted: input.ai_assisted,
    };
    let detail = service.create_collection(&tenant, &principal, new).await?;
    let version = detail.collection.updated_at;
    let id = detail.collection.id;
    Ok(Created::new(format!("/catalog/collections/{id}"), collection_response(detail))
        .tagged(version))
}

/// Get a product collection.
///
/// Requires `catalog.products.read` covering its unit.
#[utoipa::path(
    get,
    path = "/collections/{id}",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "get_catalog_collection",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the collection.")),
    responses(
        (status = 200, description = "The collection.", body = Data<CollectionResponse>, headers(("ETag" = String, description = "Version: send it in `If-Match` to change the collection or its products."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `catalog.products.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No collection has this id within reach; code `catalog.collection_not_found`.", body = Problem),
    )
)]
async fn get_collection(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<CollectionResponse>, Error> {
    let detail = service.get_collection(&tenant, &principal, id).await?;
    Ok(Tagged { version: detail.collection.updated_at, data: collection_response(detail) })
}

/// Change a product collection.
///
/// Requires `catalog.collections.manage` for its unit. `If-Match` is required.
#[utoipa::path(
    patch,
    path = "/collections/{id}",
    extensions(("x-permissions" = json!(["catalog.collections.manage"]))),
    operation_id = "update_catalog_collection",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the collection."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    request_body = UpdateCollectionRequest,
    responses(
        (status = 200, description = "The collection after the change.", body = Data<CollectionResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `catalog.collections.manage` for the unit; code `permission_denied`.", body = Problem),
        (status = 404, description = "No collection has this id within reach; code `catalog.collection_not_found`.", body = Problem),
        (status = 409, description = "A slug is taken in that language; code `catalog.slug_taken`.", body = Problem),
        (status = 412, description = "Changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
async fn update_collection(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateCollectionRequest>,
) -> Result<Tagged<CollectionResponse>, Error> {
    let expected = if_match.required()?;
    let patch = CollectionPatch {
        rules: input.rules,
        sort: input.sort,
        media_id: input.media_id,
        status: input.status,
        starts_at: input.starts_at,
        ends_at: input.ends_at,
        texts: texts(input.texts),
        ai_assisted: input.ai_assisted,
    };
    let detail = service.update_collection(&tenant, &principal, id, patch, expected).await?;
    Ok(Tagged { version: detail.collection.updated_at, data: collection_response(detail) })
}

/// Delete a product collection.
///
/// Requires `catalog.collections.manage` for its unit. Banners and menu items pointing to it stop
/// showing. To keep it, archive it instead (`status` = `archived`).
#[utoipa::path(
    delete,
    path = "/collections/{id}",
    extensions(("x-permissions" = json!(["catalog.collections.manage"]))),
    operation_id = "delete_catalog_collection",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the collection.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `catalog.collections.manage` for the unit; code `permission_denied`.", body = Problem),
        (status = 404, description = "No collection has this id within reach; code `catalog.collection_not_found`.", body = Problem),
    )
)]
async fn delete_collection(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_collection(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The chosen products of a collection, in order.
#[derive(Debug, Serialize, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCollectionItems)]
pub(crate) struct ItemsRequest {
    /// Product ids, first shown first; at most 500.
    product_ids: Vec<Uuid>,
}

/// Get the chosen products of a collection.
///
/// Requires `catalog.products.read` covering its unit.
#[utoipa::path(
    get,
    path = "/collections/{id}/items",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "get_catalog_collection_items",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the collection.")),
    responses(
        (status = 200, description = "The chosen products, in order.", body = Data<ItemsRequest>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `catalog.products.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No collection has this id within reach; code `catalog.collection_not_found`.", body = Problem),
    )
)]
async fn get_collection_items(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ItemsRequest>, Error> {
    let product_ids = service.collection_items(&tenant, &principal, id).await?;
    Ok(Data { data: ItemsRequest { product_ids } })
}

/// Replace the chosen products of a collection.
///
/// Requires `catalog.collections.manage` for its unit. Only for `manual` collections; sets the
/// whole list and its order at once. `If-Match` is required (the collection's `ETag`).
#[utoipa::path(
    put,
    path = "/collections/{id}/items",
    extensions(("x-permissions" = json!(["catalog.collections.manage"]))),
    operation_id = "put_catalog_collection_items",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the collection."), ("If-Match" = String, Header, description = "Required: the collection's `ETag` (or `*`).")),
    request_body = ItemsRequest,
    responses(
        (status = 200, description = "The collection, with its new version.", body = Data<CollectionResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `catalog.collections.manage` for the unit; code `permission_denied`.", body = Problem),
        (status = 404, description = "No collection has this id within reach; code `catalog.collection_not_found`.", body = Problem),
        (status = 412, description = "Changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "Not a `manual` collection, an unknown or repeated product, or too many; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
async fn put_collection_items(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<ItemsRequest>,
) -> Result<Tagged<CollectionResponse>, Error> {
    let expected = if_match.required()?;
    let detail =
        service.set_collection_items(&tenant, &principal, id, input.product_ids, expected).await?;
    Ok(Tagged { version: detail.collection.updated_at, data: collection_response(detail) })
}

/// A collection page of the website.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogPublicCollection)]
pub(crate) struct PublicCollectionResponse {
    id: Uuid,
    locale: String,
    title: String,
    slug: String,
    description: String,
    meta_title: String,
    meta_description: String,
    /// Picture (public media item).
    media_id: Option<Uuid>,
    /// The texts were written with an AI's suggestion: show an AI label next to them.
    ai_assisted: bool,
    /// One page of its products, in the collection's order; same card as
    /// `GET /api/v1/catalog/public/products`.
    products: Vec<PublicCardResponse>,
    pagination: autoxide_kernel::pagination::Pagination,
}

/// The language of a collection page.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PublicCollectionQuery {
    /// Language of the page, e.g. `vi`.
    #[param(example = "vi")]
    locale: String,
}

/// Get a collection page (public).
///
/// No sign-in needed. Only `active` collections in their time window, of the website or a unit
/// above it, with a text in `locale`. Products are those still shown on the website, in the
/// collection's order (`manual`, `newest`, `price_asc`, `price_desc`, `best_selling`); follow
/// `pagination.next_cursor` for more.
#[utoipa::path(
    get,
    path = "/public/collections/{slug}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_catalog_collection",
    security((), ("bearer" = [])),
    params(("slug" = String, Path, description = "Slug of the collection in `locale`."), PublicCollectionQuery, PageQuery, SiteQuery),
    responses(
        (status = 200, description = "The collection and one page of its products.", body = Data<PublicCollectionResponse>, headers(("ETag" = String, description = "Fingerprint of the answer; send it in `If-None-Match` to get 304."))),
        (status = 404, description = "No public collection has this slug; code `catalog.collection_not_found`.", body = Problem),
        (status = 422, description = "The language is not supported, or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn get_public_collection(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    site: Site,
    Path(slug): Path<String>,
    Query(query): Query<PublicCollectionQuery>,
    Query(mut paging): Query<PageQuery>,
) -> Result<Response, Error> {
    // Thứ tự của bộ sưu tập quyết `order`: chọn tay và giá tăng thì tăng dần, còn lại giảm dần.
    let now = Timestamp::now();
    let found = service.find_public_collection(&tenant, &site, &query.locale, &slug, now).await?;
    let ascending = crate::collections::public::ascending(&found.0.sort);
    paging.order = Some(if ascending { "asc" } else { "desc" }.to_owned());
    let page = PageRequest::try_from(paging)?;
    let found = service.public_collection(&tenant, &site, found, &page, now).await?;
    let products = cards(&service, &tenant, &site, &query.locale, &found.products.data).await?;
    let text = found.text;
    let data = PublicCollectionResponse {
        id: found.collection.id,
        locale: text.locale,
        title: text.title,
        slug: text.slug,
        description: text.description,
        meta_title: text.meta_title,
        meta_description: text.meta_description,
        media_id: found.collection.media_id,
        ai_assisted: found.collection.ai_assisted,
        products,
        pagination: found.products.pagination,
    };
    cached(&tenant, Data { data })
}
