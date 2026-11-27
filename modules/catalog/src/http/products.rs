//! Endpoint quản trị sản phẩm và biến thể, dưới `/api/v1/catalog`. Handler chỉ: lấy dữ liệu vào
//! → kiểm permission → gọi service → trả envelope.

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::http::dto::{
    ComponentDto, CreateProductRequest, KindDto, OptionDto, ProductListItem, ProductMediaDto,
    ProductResponse, SaleModeDto, StatusDto, TextRequest, UpdateProductRequest, VariantRequest,
    VariantResponse, present,
};
use crate::permissions::{PRODUCTS_PUBLISH, PRODUCTS_READ, PRODUCTS_WRITE};
use crate::product_parts::MediaInput;
use crate::products::{NewProduct, ProductChanges, ProductFilter};
use crate::service::CatalogService;
use crate::variants::{OptionInput, OptionValueInput, VariantChanges};

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_products, create_product))
        .routes(routes!(get_product, update_product, delete_product))
        .routes(routes!(set_product_status))
        .routes(routes!(put_product_translation, delete_product_translation))
        .routes(routes!(put_product_options))
        .routes(routes!(put_product_categories))
        .routes(routes!(put_product_media))
        .routes(routes!(create_variant))
        .routes(routes!(find_variants))
        .routes(routes!(update_variant, delete_variant))
        .routes(routes!(put_variant_components))
        .with_state(service)
}

fn tagged(product: ProductResponse) -> Tagged<ProductResponse> {
    Tagged { version: product.updated_at, data: product }
}

/// Filters of the product list.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct ProductsQuery {
    /// Only products in this state: `draft`, `active`, `inactive` or `archived`.
    pub status: Option<StatusDto>,
    /// Only products of this kind: `physical`, `digital`, `service` or `bundle`.
    pub kind: Option<KindDto>,
    /// Only products of this unit.
    pub org_id: Option<Uuid>,
    /// Only products in this category or one below it.
    pub category_id: Option<Uuid>,
    /// `ready`, `missing_attributes`, `missing_documents` or `restricted`.
    pub readiness: Option<String>,
    /// Words in the name, SKU or short description, with or without Vietnamese accents; up to
    /// 200 characters.
    pub q: Option<String>,
}

impl ProductsQuery {
    pub(crate) fn filter(self) -> Result<ProductFilter, Error> {
        if self.q.as_deref().is_some_and(|q| q.chars().count() > 200) {
            return Err(Error::validation(vec![autoxide_kernel::error::FieldError::new(
                "q",
                "length",
                "must be at most 200 characters",
            )]));
        }
        Ok(ProductFilter {
            status: self.status.map(Into::into),
            kind: self.kind.map(Into::into),
            org_id: self.org_id,
            category_id: self.category_id,
            readiness: self.readiness,
            q: self.q,
        })
    }
}

/// List the products in your scope.
///
/// Requires `catalog.products.read`; its scope decides which units' products you see. Newest
/// first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/products",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "list_catalog_products",
    security(("bearer" = [])),
    params(ProductsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of products.", body = Page<ProductListItem>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_products(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ProductsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<ProductListItem>, Error> {
    principal.require(&PRODUCTS_READ)?;
    let page = PageRequest::try_from(page)?;
    let found = service.list_products(&tenant, &principal, &query.filter()?, &page).await?;
    Ok(found.map(ProductListItem::from))
}

/// Create a product.
///
/// Requires `catalog.products.write` on the unit. The product starts as a `draft` with one
/// variant; it counts towards the plan limit `catalog.products`. Retry safely with an
/// `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/products",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "create_catalog_product",
    security(("bearer" = [])),
    request_body = CreateProductRequest,
    responses(
        (status = 201, description = "Created.", body = Data<ProductResponse>, headers(("Location" = String, description = "Path of the product."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or not active (`org_frozen`), or the plan limit is reached (`limit_exceeded`).", body = Problem),
        (status = 404, description = "A category does not exist; code `catalog.category_not_found`.", body = Problem),
        (status = 409, description = "The SKU (`catalog.sku_taken`), barcode (`catalog.barcode_taken`) or slug (`catalog.slug_taken`) is used already.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), the language is not configured (`catalog.unsupported_locale`), or you work in several units and gave no `org_id` (`catalog.org_required`).", body = Problem),
    )
)]
async fn create_product(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateProductRequest>,
) -> Result<Created<ProductResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let input = NewProduct {
        org_id: input.org_id,
        kind: input.kind.into(),
        tax_category: input.tax_category,
        locale: input.locale,
        text: input.text.into(),
        warranty_months: input.warranty_months,
        lot_tracked: input.lot_tracked,
        serial_tracked: input.serial_tracked,
        category_ids: input.category_ids,
        variant: input.variant.into(),
    };
    let product = service.create_product(&tenant, &principal, input).await?;
    let response = ProductResponse::from(product);
    Ok(Created::new(format!("/catalog/products/{}", response.id), response))
}

/// Get a product with its texts, options, variants, categories and images.
///
/// Requires `catalog.products.read` covering the product; a product outside your scope answers
/// 404.
#[utoipa::path(
    get,
    path = "/products/{id}",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "get_catalog_product",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product.")),
    responses(
        (status = 200, description = "The product.", body = Data<ProductResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No product with this id in your scope; code `catalog.product_not_found`.", body = Problem),
    )
)]
async fn get_product(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<ProductResponse>, Error> {
    principal.require(&PRODUCTS_READ)?;
    Ok(tagged(service.get_product(&tenant, &principal, id).await?.into()))
}

/// Change the general details of a product.
///
/// Requires `catalog.products.write` covering the product.
#[utoipa::path(
    patch,
    path = "/products/{id}",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "update_catalog_product",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the change is made only if nobody changed the record since; otherwise `412`.")),
    request_body = UpdateProductRequest,
    responses(
        (status = 200, description = "The product after the change.", body = Data<ProductResponse>, headers(("ETag" = String, description = "Version of the record."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write` (`permission_denied`), it does not cover this product (`org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 404, description = "No product with this id in your scope; code `catalog.product_not_found`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the record changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_product(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateProductRequest>,
) -> Result<Tagged<ProductResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let changes = ProductChanges {
        tax_category: input.tax_category,
        warranty_months: input.warranty_months,
        lot_tracked: input.lot_tracked,
        serial_tracked: input.serial_tracked,
        default_locale: input.default_locale,
    };
    let expected = if_match.expected()?;
    Ok(tagged(service.update_product(&tenant, &principal, id, changes, expected).await?.into()))
}

/// Delete a product that was never on sale.
///
/// Requires `catalog.products.write` covering the product. A product that was on sale once can
/// only be archived (`POST /products/{id}/status`).
#[utoipa::path(
    delete,
    path = "/products/{id}",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "delete_catalog_product",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write` (`permission_denied`), it does not cover this product (`org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 404, description = "No product with this id in your scope; code `catalog.product_not_found`.", body = Problem),
        (status = 409, description = "The product was on sale before; archive it instead; code `catalog.product_not_deletable`.", body = Problem),
    )
)]
async fn delete_product(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    service.delete_product(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The new sale status of a product.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogStatusChange)]
pub(crate) struct StatusRequest {
    /// `draft` → `active` or `archived`; `active` ↔ `inactive`; any → `archived`; `archived` →
    /// `draft`.
    pub status: StatusDto,
}

/// Change the sale status of a product.
///
/// Requires `catalog.products.publish` covering the product. Putting a product on sale checks it
/// has a tax category, a name, an active variant and `readiness = ready`.
#[utoipa::path(
    post,
    path = "/products/{id}/status",
    extensions(("x-permissions" = json!(["catalog.products.publish"]))),
    operation_id = "set_catalog_product_status",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read; otherwise `412` when it changed since.")),
    request_body = StatusRequest,
    responses(
        (status = 200, description = "The product after the change.", body = Data<ProductResponse>, headers(("ETag" = String, description = "Version of the record."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.publish` (`permission_denied`), it does not cover this product (`org_out_of_scope`), the unit is not active (`org_frozen`), or on a marketplace the seller (the owning unit or a unit above it, below the root) has not verified its identity (`compliance.kyc_required`) or identity verification is not enabled (`catalog.seller_verification_unavailable`).", body = Problem),
        (status = 404, description = "No product with this id in your scope; code `catalog.product_not_found`.", body = Problem),
        (status = 409, description = "The change is not allowed (`catalog.invalid_status_transition`), the goods are prohibited (`catalog.prohibited_goods`) or need valid documents (`catalog.documents_required`).", body = Problem),
        (status = 412, description = "`If-Match` does not match; code `precondition_failed`.", body = Problem),
        (status = 422, description = "The product is not ready to sell (see `errors`), required attributes are missing (`catalog.required_attributes_missing`), or a module refuses the listing, e.g. an unverified marketplace seller (`catalog.listing_denied`; the reason is in `detail`).", body = Problem),
    )
)]
async fn set_product_status(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<StatusRequest>,
) -> Result<Tagged<ProductResponse>, Error> {
    principal.require(&PRODUCTS_PUBLISH)?;
    let expected = if_match.expected()?;
    let product =
        service.set_status(&tenant, &principal, id, input.status.into(), expected).await?;
    Ok(tagged(product.into()))
}

/// Set the texts of a product in one language.
///
/// Requires `catalog.products.write` covering the product. An empty `slug` is made from the
/// name.
#[utoipa::path(
    put,
    path = "/products/{id}/translations/{locale}",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "put_catalog_product_translation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product."), ("locale" = String, Path, description = "Language, e.g. `vi`.")),
    request_body = TextRequest,
    responses(
        (status = 200, description = "The product after the change.", body = Data<ProductResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`, or it does not cover this product; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No product with this id in your scope; code `catalog.product_not_found`.", body = Problem),
        (status = 409, description = "The slug is used by another product in this language; code `catalog.slug_taken`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`) or the language is not configured (`catalog.unsupported_locale`).", body = Problem),
    )
)]
async fn put_product_translation(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, locale)): Path<(Uuid, String)>,
    ValidJson(input): ValidJson<TextRequest>,
) -> Result<Data<ProductResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let product = service.put_translation(&tenant, &principal, id, &locale, input.into()).await?;
    Ok(Data { data: product.into() })
}

/// Remove the texts of a product in one language.
///
/// Requires `catalog.products.write` covering the product. The default language cannot be
/// removed.
#[utoipa::path(
    delete,
    path = "/products/{id}/translations/{locale}",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "delete_catalog_product_translation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product."), ("locale" = String, Path, description = "Language, e.g. `en`.")),
    responses(
        (status = 200, description = "The product after the change.", body = Data<ProductResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`, or it does not cover this product; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No product with this id in your scope; code `catalog.product_not_found`.", body = Problem),
        (status = 422, description = "The language is the product's default (see `errors`) or not configured (`catalog.unsupported_locale`).", body = Problem),
    )
)]
async fn delete_product_translation(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, locale)): Path<(Uuid, String)>,
) -> Result<Data<ProductResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    Ok(Data { data: service.delete_translation(&tenant, &principal, id, &locale).await?.into() })
}

/// The whole set of options of a product.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogOptions)]
pub(crate) struct OptionsRequest {
    /// Up to 3 options, each with 1 to 100 values.
    pub options: Vec<OptionDto>,
}

/// Replace the options of a product (colour, size…).
///
/// Requires `catalog.products.write` covering the product. Keep the ids of existing options and
/// values that variants use; every variant must still hold one value of each option.
#[utoipa::path(
    put,
    path = "/products/{id}/options",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "put_catalog_product_options",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product.")),
    request_body = OptionsRequest,
    responses(
        (status = 200, description = "The product after the change.", body = Data<ProductResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`, or it does not cover this product; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No product with this id in your scope; code `catalog.product_not_found`.", body = Problem),
        (status = 409, description = "A variant still uses a removed value or lacks a value of a new option; code `catalog.option_value_in_use`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`) or a language is not configured (`catalog.unsupported_locale`).", body = Problem),
    )
)]
async fn put_product_options(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<OptionsRequest>,
) -> Result<Data<ProductResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let options = input
        .options
        .into_iter()
        .map(|option| OptionInput {
            id: option.id,
            names: option.names,
            values: option
                .values
                .into_iter()
                .map(|value| OptionValueInput { id: value.id, labels: value.labels })
                .collect(),
        })
        .collect();
    Ok(Data { data: service.put_options(&tenant, &principal, id, options).await?.into() })
}

/// The whole set of categories of a product.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogProductCategories)]
pub(crate) struct CategoriesRequest {
    /// The first is the main category.
    pub category_ids: Vec<Uuid>,
}

/// Replace the categories of a product.
///
/// Requires `catalog.products.write` covering the product.
#[utoipa::path(
    put,
    path = "/products/{id}/categories",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "put_catalog_product_categories",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product.")),
    request_body = CategoriesRequest,
    responses(
        (status = 200, description = "The product after the change.", body = Data<ProductResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`, or it does not cover this product; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No such product in your scope (`catalog.product_not_found`) or no such category (`catalog.category_not_found`).", body = Problem),
        (status = 422, description = "The body is invalid; see `errors`.", body = Problem),
    )
)]
async fn put_product_categories(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CategoriesRequest>,
) -> Result<Data<ProductResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let product = service.put_categories(&tenant, &principal, id, input.category_ids).await?;
    Ok(Data { data: product.into() })
}

/// The whole set of images of a product.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogProductImages)]
pub(crate) struct MediaRequest {
    /// Up to 50 images, in display order.
    pub media: Vec<ProductMediaDto>,
}

/// Replace the images of a product.
///
/// Requires `catalog.products.write` covering the product. Every image must be public media of
/// the `media` module.
#[utoipa::path(
    put,
    path = "/products/{id}/media",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "put_catalog_product_media",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product.")),
    request_body = MediaRequest,
    responses(
        (status = 200, description = "The product after the change.", body = Data<ProductResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`, or it does not cover this product; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No such product in your scope (`catalog.product_not_found`) or the variant is not of this product (`catalog.variant_not_found`).", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`) or an image is not public media (`catalog.media_not_public`).", body = Problem),
    )
)]
async fn put_product_media(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<MediaRequest>,
) -> Result<Data<ProductResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let items = input
        .media
        .into_iter()
        .map(|item| MediaInput {
            media_id: item.media_id,
            variant_id: item.variant_id,
            alt: item.alt,
        })
        .collect();
    Ok(Data { data: service.put_media(&tenant, &principal, id, items).await?.into() })
}

/// Add a variant to a product.
///
/// Requires `catalog.products.write` covering the product and the plan feature
/// `catalog.variants`.
#[utoipa::path(
    post,
    path = "/products/{id}/variants",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "create_catalog_variant",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product.")),
    request_body = VariantRequest,
    responses(
        (status = 201, description = "Created.", body = Data<VariantResponse>, headers(("Location" = String, description = "Path of the variant."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write` or the plan feature (`feature_not_enabled`), or it does not cover this product (`org_out_of_scope`, `org_frozen`).", body = Problem),
        (status = 404, description = "No product with this id in your scope; code `catalog.product_not_found`.", body = Problem),
        (status = 409, description = "The SKU (`catalog.sku_taken`), barcode (`catalog.barcode_taken`) or option combination (`catalog.option_combination_taken`) is used already.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors` (codes include `catalog.preorder_date_invalid`, `catalog.preorder_limit_invalid`).", body = Problem),
    )
)]
async fn create_variant(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<VariantRequest>,
) -> Result<Created<VariantResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let row = service.create_variant(&tenant, &principal, id, input.into()).await?;
    Ok(Created::new(format!("/catalog/variants/{}", row.id), VariantResponse::from(&row)))
}

/// Find variants by code.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct VariantsQuery {
    /// SKU (any case, spaces ignored) or barcode.
    pub code: String,
}

/// Find variants by SKU or barcode.
///
/// Requires `catalog.products.read`; only variants of products in your scope are returned.
#[utoipa::path(
    get,
    path = "/variants",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "find_catalog_variants",
    security(("bearer" = [])),
    params(VariantsQuery),
    responses(
        (status = 200, description = "The matching variants (at most one per SKU and one per barcode).", body = Data<Vec<VariantResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "`code` is missing; see `errors`.", body = Problem),
    )
)]
async fn find_variants(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<VariantsQuery>,
) -> Result<Data<Vec<VariantResponse>>, Error> {
    principal.require(&PRODUCTS_READ)?;
    let rows = service.find_variants_by_code(&tenant, &principal, &query.code).await?;
    Ok(Data { data: rows.iter().map(VariantResponse::from).collect() })
}

/// Changes to a variant. Omitted fields stay as they are; `null` clears an optional value.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogUpdateVariant)]
pub(crate) struct UpdateVariantRequest {
    pub sku: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub barcode: Option<Option<String>>,
    pub option_value_ids: Option<Vec<Uuid>>,
    pub active: Option<bool>,
    #[schema(inline)]
    pub sale_mode: Option<SaleModeDto>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Date>, nullable)]
    pub preorder_available_on: Option<Option<Date>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i64>, nullable)]
    pub preorder_limit: Option<Option<i64>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Timestamp>, nullable)]
    pub preorder_ends_at: Option<Option<Timestamp>>,
    pub preorder_messages: Option<BTreeMap<String, String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i64>, nullable)]
    pub weight_grams: Option<Option<i64>>,
    /// Net content, with `net_unit`; `null` on both removes it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i64>, nullable)]
    pub net_quantity: Option<Option<i64>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(inline, value_type = Option<crate::http::dto::NetUnitDto>, nullable)]
    pub net_unit: Option<Option<crate::http::dto::NetUnitDto>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i64>, nullable)]
    pub length_mm: Option<Option<i64>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i64>, nullable)]
    pub width_mm: Option<Option<i64>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i64>, nullable)]
    pub height_mm: Option<Option<i64>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    pub image_media_id: Option<Option<Uuid>>,
    /// Display order among the product's variants.
    pub position: Option<i32>,
}

impl From<UpdateVariantRequest> for VariantChanges {
    fn from(input: UpdateVariantRequest) -> Self {
        VariantChanges {
            sku: input.sku,
            barcode: input.barcode,
            option_value_ids: input.option_value_ids,
            active: input.active,
            sale_mode: input.sale_mode.map(Into::into),
            preorder_available_on: input.preorder_available_on,
            preorder_limit: input.preorder_limit,
            preorder_ends_at: input.preorder_ends_at,
            preorder_messages: input.preorder_messages,
            weight_grams: input.weight_grams,
            net_quantity: input.net_quantity,
            net_unit: input
                .net_unit
                .map(|unit| unit.map(|unit| crate::http::dto::NetUnitDto::as_str(unit).to_owned())),
            length_mm: input.length_mm,
            width_mm: input.width_mm,
            height_mm: input.height_mm,
            image_media_id: input.image_media_id,
            position: input.position,
        }
    }
}

/// Change a variant.
///
/// Requires `catalog.products.write` covering its product. Moving the date of a pre-order also
/// emits `catalog.preorder_date_changed`.
#[utoipa::path(
    patch,
    path = "/variants/{id}",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "update_catalog_variant",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the variant."), ("If-Match" = Option<String>, Header, description = "The `updated_at` version you read; otherwise `412` when it changed since.")),
    request_body = UpdateVariantRequest,
    responses(
        (status = 200, description = "The variant after the change.", body = Data<VariantResponse>, headers(("ETag" = String, description = "Version of the record."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`, or it does not cover the product; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No variant with this id in your scope; code `catalog.variant_not_found`.", body = Problem),
        (status = 409, description = "The SKU, barcode or option combination is used already (`catalog.sku_taken`, `catalog.barcode_taken`, `catalog.option_combination_taken`).", body = Problem),
        (status = 412, description = "`If-Match` does not match; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_variant(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateVariantRequest>,
) -> Result<Tagged<VariantResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let expected = if_match.expected()?;
    let row = service.update_variant(&tenant, &principal, id, input.into(), expected).await?;
    Ok(Tagged { version: row.updated_at, data: VariantResponse::from(&row) })
}

/// Delete a variant.
///
/// Requires `catalog.products.write` covering its product. A product keeps at least one variant.
#[utoipa::path(
    delete,
    path = "/variants/{id}",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "delete_catalog_variant",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the variant.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`, or it does not cover the product; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No variant with this id in your scope; code `catalog.variant_not_found`.", body = Problem),
        (status = 409, description = "It is the last variant of the product; code `catalog.last_variant`.", body = Problem),
    )
)]
async fn delete_variant(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    service.delete_variant(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The whole set of components of a bundle variant.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogComponents)]
pub(crate) struct ComponentsRequest {
    /// 1 to 50 components, each variant once.
    pub components: Vec<ComponentDto>,
}

/// Replace the components of a bundle variant.
///
/// Requires `catalog.products.write` covering the bundle and the plan feature
/// `catalog.bundles`. A bundle cannot contain a bundle, itself or a service.
#[utoipa::path(
    put,
    path = "/variants/{id}/components",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "put_catalog_variant_components",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the bundle variant.")),
    request_body = ComponentsRequest,
    responses(
        (status = 200, description = "The components.", body = Data<Vec<ComponentDto>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write` or the plan feature (`feature_not_enabled`), or it does not cover the product (`org_out_of_scope`, `org_frozen`).", body = Problem),
        (status = 404, description = "No such variant in your scope; code `catalog.variant_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`) or the bundle is invalid (`catalog.bundle_invalid`).", body = Problem),
    )
)]
async fn put_variant_components(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ComponentsRequest>,
) -> Result<Data<Vec<ComponentDto>>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let components = input.components.into_iter().map(Into::into).collect();
    let saved = service.put_components(&tenant, &principal, id, components).await?;
    Ok(Data { data: saved.into_iter().map(ComponentDto::from).collect() })
}
