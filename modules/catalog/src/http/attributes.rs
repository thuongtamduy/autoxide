//! Endpoint quản trị thuộc tính, thuộc tính và quy tắc hàng cấm của danh mục, giá trị thuộc tính
//! và giấy tờ của sản phẩm, số liệu sản phẩm — dưới `/api/v1/catalog`.

use autoxide_kernel::time::{Date, Timestamp};
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::attributes::{AttributeDetail, CategoryAttributeInput, NewAttribute, OptionInput};
use crate::entity::{category_attribute, product_attribute, product_document, restriction_rule};
use crate::http::dto::present;
use crate::permissions::{CATEGORIES_MANAGE, DOCUMENTS_MANAGE, PRODUCTS_READ, PRODUCTS_WRITE};
use crate::restrictions::{AttributeChanges, AttributeValueInput, NewDocument};
use crate::service::{CatalogService, texts_of};

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_attributes, create_attribute))
        .routes(routes!(update_attribute))
        .routes(routes!(put_attribute_options))
        .routes(routes!(get_category_attributes, put_category_attributes))
        .routes(routes!(get_category_restrictions, put_category_restrictions))
        .routes(routes!(list_restriction_rules))
        .routes(routes!(get_product_attributes, put_product_attributes))
        .routes(routes!(list_product_documents, add_product_document))
        .routes(routes!(delete_product_document))
        .routes(routes!(count_products))
        .with_state(service)
}

/// A choosable value of an `option` / `options` attribute.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogAttributeOption)]
pub(crate) struct AttributeOptionDto {
    /// 1 to 64 lower-case letters, digits, `_` or `-`; unique in the attribute.
    pub code: String,
    /// Label per language, e.g. `{"en": "Cotton"}`.
    pub labels: BTreeMap<String, String>,
}

/// An attribute products can have (origin, material, size of the box…).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogAttribute)]
pub(crate) struct AttributeResponse {
    pub id: Uuid,
    pub code: String,
    /// `text`, `integer`, `decimal`, `boolean`, `date`, `option` or `options`.
    pub data_type: String,
    /// Decimal places of a `decimal` attribute.
    pub scale: i32,
    pub unit: Option<String>,
    pub names: BTreeMap<String, String>,
    /// Offered as a storefront filter.
    pub filterable: bool,
    /// Law the attribute comes from, for legally required label contents.
    pub legal_basis: Option<String>,
    pub options: Vec<AttributeOptionDto>,
    pub updated_at: Timestamp,
}

impl From<AttributeDetail> for AttributeResponse {
    fn from(detail: AttributeDetail) -> Self {
        let row = detail.attribute;
        AttributeResponse {
            id: row.id,
            code: row.code,
            data_type: row.data_type,
            scale: row.scale,
            unit: row.unit,
            names: texts_of(&row.names),
            filterable: row.filterable,
            legal_basis: row.legal_basis,
            options: detail
                .options
                .into_iter()
                .map(|option| AttributeOptionDto {
                    code: option.code,
                    labels: texts_of(&option.labels),
                })
                .collect(),
            updated_at: row.updated_at,
        }
    }
}

/// List every attribute of the tenant.
///
/// Requires `catalog.products.read`. By code (A→Z unless `order=desc`); paged with `limit` /
/// `cursor`.
#[utoipa::path(
    get,
    path = "/attributes",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "list_catalog_attributes",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the attributes, by code.", body = Page<AttributeResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_attributes(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<AttributeResponse>, Error> {
    principal.require(&PRODUCTS_READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.list_attributes_page(&tenant, &page).await?;
    Ok(rows.map(AttributeResponse::from))
}

/// A new attribute.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCreateAttribute)]
pub(crate) struct CreateAttributeRequest {
    /// 1 to 64 lower-case letters, digits or `_`; unique in the tenant
    /// (`catalog.attribute_code_taken`).
    #[schema(example = "origin")]
    pub code: String,
    /// `text`, `integer`, `decimal`, `boolean`, `date`, `option` or `options`.
    pub data_type: String,
    /// Decimal places of a `decimal` attribute, 0 to 6. Default 0.
    #[serde(default)]
    pub scale: i32,
    pub unit: Option<String>,
    /// Name per language.
    pub names: BTreeMap<String, String>,
    #[serde(default)]
    pub filterable: bool,
    pub legal_basis: Option<String>,
}

/// Create an attribute.
///
/// Requires `catalog.categories.manage` for the whole tenant.
#[utoipa::path(
    post,
    path = "/attributes",
    extensions(("x-permissions" = json!(["catalog.categories.manage"]))),
    operation_id = "create_catalog_attribute",
    security(("bearer" = [])),
    request_body = CreateAttributeRequest,
    responses(
        (status = 201, description = "Created.", body = Data<AttributeResponse>, headers(("Location" = String, description = "Path of the attribute."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.categories.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "The code is used already; code `catalog.attribute_code_taken`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`) or a language is not configured (`catalog.unsupported_locale`).", body = Problem),
    )
)]
async fn create_attribute(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateAttributeRequest>,
) -> Result<Created<AttributeResponse>, Error> {
    principal.require(&CATEGORIES_MANAGE)?;
    let input = NewAttribute {
        code: input.code,
        data_type: input.data_type,
        scale: input.scale,
        unit: input.unit,
        names: input.names,
        filterable: input.filterable,
        legal_basis: input.legal_basis,
    };
    let made = AttributeResponse::from(service.create_attribute(&tenant, &principal, input).await?);
    Ok(Created::new(format!("/catalog/attributes/{}", made.id), made))
}

/// Changes to an attribute; its code and type cannot change.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogUpdateAttribute)]
pub(crate) struct UpdateAttributeRequest {
    pub names: Option<BTreeMap<String, String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub unit: Option<Option<String>>,
    pub filterable: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub legal_basis: Option<Option<String>>,
}

/// Change an attribute.
///
/// Requires `catalog.categories.manage`.
#[utoipa::path(
    patch,
    path = "/attributes/{id}",
    extensions(("x-permissions" = json!(["catalog.categories.manage"]))),
    operation_id = "update_catalog_attribute",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the attribute.")),
    request_body = UpdateAttributeRequest,
    responses(
        (status = 200, description = "The attribute after the change.", body = Data<AttributeResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.categories.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such attribute; code `catalog.attribute_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_attribute(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<UpdateAttributeRequest>,
) -> Result<Data<AttributeResponse>, Error> {
    principal.require(&CATEGORIES_MANAGE)?;
    let changes = AttributeChanges {
        names: input.names,
        unit: input.unit,
        filterable: input.filterable,
        legal_basis: input.legal_basis,
    };
    let detail = service.update_attribute(&tenant, &principal, id, changes).await?;
    Ok(Data { data: detail.into() })
}

/// The whole set of options of an attribute.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogAttributeOptions)]
pub(crate) struct OptionsRequest {
    /// Up to 500 options, in display order. Options are matched by `code`.
    pub options: Vec<AttributeOptionDto>,
}

/// Replace the options of an `option` / `options` attribute.
///
/// Requires `catalog.categories.manage`. Removing an option products still use is refused.
#[utoipa::path(
    put,
    path = "/attributes/{id}/options",
    extensions(("x-permissions" = json!(["catalog.categories.manage"]))),
    operation_id = "put_catalog_attribute_options",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the attribute.")),
    request_body = OptionsRequest,
    responses(
        (status = 200, description = "The attribute after the change.", body = Data<AttributeResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.categories.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such attribute; code `catalog.attribute_not_found`.", body = Problem),
        (status = 409, description = "Products still use a removed option; code `catalog.attribute_option_in_use`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn put_attribute_options(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<OptionsRequest>,
) -> Result<Data<AttributeResponse>, Error> {
    principal.require(&CATEGORIES_MANAGE)?;
    let options = input
        .options
        .into_iter()
        .map(|option| OptionInput { code: option.code, labels: option.labels })
        .collect();
    Ok(Data { data: service.put_attribute_options(&tenant, &principal, id, options).await?.into() })
}

/// An attribute of a category.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCategoryAttribute)]
pub(crate) struct CategoryAttributeDto {
    pub attribute_id: Uuid,
    /// Products in the category cannot go on sale without a value.
    #[serde(default)]
    pub required: bool,
    /// Also for products of the subcategories.
    #[serde(default = "yes")]
    pub applies_to_descendants: bool,
    /// The rule applies from this day (e.g. when a new labelling rule takes effect).
    pub effective_from: Date,
    /// The rule stops applying on this day.
    pub effective_to: Option<Date>,
    /// Law the rule comes from.
    pub legal_basis: Option<String>,
}

fn yes() -> bool {
    true
}

impl From<category_attribute::Model> for CategoryAttributeDto {
    fn from(row: category_attribute::Model) -> Self {
        CategoryAttributeDto {
            attribute_id: row.attribute_id,
            required: row.required,
            applies_to_descendants: row.applies_to_descendants,
            effective_from: row.effective_from,
            effective_to: row.effective_to,
            legal_basis: row.legal_basis,
        }
    }
}

/// The whole set of attributes of a category.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCategoryAttributes)]
pub(crate) struct CategoryAttributesRequest {
    pub attributes: Vec<CategoryAttributeDto>,
}

/// List the attributes of a category.
///
/// Requires `catalog.products.read`. In the order of the last replace (`PUT`) unless `order=desc`;
/// paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/categories/{id}/attributes",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "get_catalog_category_attributes",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the category."), NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the attributes of the category.", body = Page<CategoryAttributeDto>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn get_category_attributes(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<CategoryAttributeDto>, Error> {
    principal.require(&PRODUCTS_READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.category_attributes_page(&tenant, id, &page).await?;
    Ok(rows.map(Into::into))
}

/// Replace the attributes of a category.
///
/// Requires `catalog.categories.manage`. The readiness of every product in the category and its
/// subcategories is computed again.
#[utoipa::path(
    put,
    path = "/categories/{id}/attributes",
    extensions(("x-permissions" = json!(["catalog.categories.manage"]))),
    operation_id = "put_catalog_category_attributes",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the category.")),
    request_body = CategoryAttributesRequest,
    responses(
        (status = 200, description = "The attributes of the category.", body = Data<Vec<CategoryAttributeDto>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.categories.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such category (`catalog.category_not_found`) or attribute (`catalog.attribute_not_found`).", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn put_category_attributes(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CategoryAttributesRequest>,
) -> Result<Data<Vec<CategoryAttributeDto>>, Error> {
    principal.require(&CATEGORIES_MANAGE)?;
    let items = input
        .attributes
        .into_iter()
        .map(|item| CategoryAttributeInput {
            attribute_id: item.attribute_id,
            required: item.required,
            applies_to_descendants: item.applies_to_descendants,
            effective_from: item.effective_from,
            effective_to: item.effective_to,
            legal_basis: item.legal_basis,
        })
        .collect();
    let rows = service.put_category_attributes(&tenant, &principal, id, items).await?;
    Ok(Data { data: rows.into_iter().map(Into::into).collect() })
}

/// The restricted-goods rules linked to a category.
#[derive(Debug, Deserialize, Serialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCategoryRestrictions)]
pub(crate) struct RestrictionsDto {
    /// Rule codes of `GET /restriction-rules`.
    pub rule_codes: Vec<String>,
}

/// List the restricted-goods rules of a category.
///
/// Requires `catalog.products.read`.
#[utoipa::path(
    get,
    path = "/categories/{id}/restrictions",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "get_catalog_category_restrictions",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the category.")),
    responses(
        (status = 200, description = "The rule codes.", body = Data<RestrictionsDto>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn get_category_restrictions(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<RestrictionsDto>, Error> {
    principal.require(&PRODUCTS_READ)?;
    Ok(Data {
        data: RestrictionsDto { rule_codes: service.category_restrictions(&tenant, id).await? },
    })
}

/// Link restricted-goods rules to a category.
///
/// Requires `catalog.categories.manage`. Products of a category with a `prohibited` rule cannot go
/// on sale; with a `conditional` rule they need valid documents. Readiness of the products in the
/// category and below is computed again.
#[utoipa::path(
    put,
    path = "/categories/{id}/restrictions",
    extensions(("x-permissions" = json!(["catalog.categories.manage"]))),
    operation_id = "put_catalog_category_restrictions",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the category.")),
    request_body = RestrictionsDto,
    responses(
        (status = 200, description = "The rule codes.", body = Data<RestrictionsDto>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.categories.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such category (`catalog.category_not_found`) or rule (`catalog.restriction_rule_not_found`).", body = Problem),
    )
)]
async fn put_category_restrictions(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<RestrictionsDto>,
) -> Result<Data<RestrictionsDto>, Error> {
    principal.require(&CATEGORIES_MANAGE)?;
    let codes =
        service.put_category_restrictions(&tenant, &principal, id, input.rule_codes).await?;
    Ok(Data { data: RestrictionsDto { rule_codes: codes } })
}

/// A rule of prohibited or conditional goods (shared data, read only).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogRestrictionRule)]
pub(crate) struct RestrictionRuleResponse {
    #[schema(example = "VN")]
    pub jurisdiction: String,
    #[schema(example = "alcohol")]
    pub code: String,
    /// `prohibited` or `conditional`.
    pub level: String,
    pub names: BTreeMap<String, String>,
    /// Document types a conditional product needs, valid on the day.
    pub required_documents: Vec<String>,
    pub legal_basis: String,
    pub source_url: String,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
    pub pack_version: String,
}

impl From<restriction_rule::Model> for RestrictionRuleResponse {
    fn from(row: restriction_rule::Model) -> Self {
        RestrictionRuleResponse {
            jurisdiction: row.jurisdiction,
            code: row.code,
            level: row.level,
            names: texts_of(&row.names),
            required_documents: serde_json::from_str(&row.required_documents).unwrap_or_default(),
            legal_basis: row.legal_basis,
            source_url: row.source_url,
            effective_from: row.effective_from,
            effective_to: row.effective_to,
            pack_version: row.pack_version,
        }
    }
}

/// List the rules of prohibited and conditional goods.
///
/// Requires `catalog.products.read`. Loaded by the operator with `autoxide task
/// catalog.load_restrictions`; check the legal basis of each rule against the source text. By
/// jurisdiction, code, then effective date (A→Z unless `order=desc`); paged with `limit` /
/// `cursor`.
#[utoipa::path(
    get,
    path = "/restriction-rules",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "list_catalog_restriction_rules",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the rules.", body = Page<RestrictionRuleResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_restriction_rules(
    State(service): State<Arc<CatalogService>>,
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<RestrictionRuleResponse>, Error> {
    principal.require(&PRODUCTS_READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.restriction_rules(&page).await?;
    Ok(rows.map(Into::into))
}

/// A value of an attribute on a product.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogAttributeValue)]
pub(crate) struct AttributeValueDto {
    pub attribute_id: Uuid,
    /// By the attribute's type: text, a whole number, a decimal written as text (`"12.5"`),
    /// `true` / `false`, a date `YYYY-MM-DD`, an option code, or a list of option codes.
    pub value: Value,
}

/// The whole set of attribute values of a product.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogProductAttributes)]
pub(crate) struct ProductAttributesRequest {
    pub attributes: Vec<AttributeValueDto>,
}

/// A stored attribute value (decimals are stored scaled by the attribute's `scale`).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogStoredAttributeValue)]
pub(crate) struct StoredValueResponse {
    pub attribute_id: Uuid,
    pub value_text: Option<String>,
    pub value_integer: Option<i64>,
    pub value_boolean: Option<bool>,
    pub value_date: Option<Date>,
    pub option_id: Option<Uuid>,
}

impl From<product_attribute::Model> for StoredValueResponse {
    fn from(row: product_attribute::Model) -> Self {
        StoredValueResponse {
            attribute_id: row.attribute_id,
            value_text: row.value_text,
            value_integer: row.value_integer,
            value_boolean: row.value_boolean,
            value_date: row.value_date,
            option_id: row.option_id,
        }
    }
}

/// List the attribute values of a product.
///
/// Requires `catalog.products.read` covering the product. In the order of the last replace
/// (`PUT`) unless `order=desc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/products/{id}/attributes",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "get_catalog_product_attributes",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product."), NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the values.", body = Page<StoredValueResponse>),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No product with this id in your scope; code `catalog.product_not_found`.", body = Problem),
    )
)]
async fn get_product_attributes(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<StoredValueResponse>, Error> {
    principal.require(&PRODUCTS_READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.get_product_attributes(&tenant, &principal, id, &page).await?;
    Ok(rows.map(Into::into))
}

/// Replace the attribute values of a product.
///
/// Requires `catalog.products.write` covering the product. The product's readiness is computed
/// again.
#[utoipa::path(
    put,
    path = "/products/{id}/attributes",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "put_catalog_product_attributes",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product.")),
    request_body = ProductAttributesRequest,
    responses(
        (status = 200, description = "The values.", body = Data<Vec<StoredValueResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`, or it does not cover this product; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No such product in your scope (`catalog.product_not_found`) or attribute (`catalog.attribute_not_found`).", body = Problem),
        (status = 422, description = "A value does not fit its attribute; see `errors`.", body = Problem),
    )
)]
async fn put_product_attributes(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ProductAttributesRequest>,
) -> Result<Data<Vec<StoredValueResponse>>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let values = input
        .attributes
        .into_iter()
        .map(|item| AttributeValueInput { attribute_id: item.attribute_id, value: item.value })
        .collect();
    let rows = service.put_product_attributes(&tenant, &principal, id, values).await?;
    Ok(Data { data: rows.into_iter().map(Into::into).collect() })
}

/// A licence or certificate of a conditional product.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogProductDocument)]
pub(crate) struct DocumentResponse {
    pub id: Uuid,
    #[schema(example = "alcohol_retail_licence")]
    pub document_type: String,
    pub number: String,
    pub issued_by: String,
    pub issued_on: Date,
    /// The document is valid until the end of this day; `null`: no end.
    pub expires_on: Option<Date>,
    /// Key of the scanned file in storage.
    pub file_key: Option<String>,
    pub created_at: Timestamp,
}

impl From<product_document::Model> for DocumentResponse {
    fn from(row: product_document::Model) -> Self {
        DocumentResponse {
            id: row.id,
            document_type: row.document_type,
            number: row.number,
            issued_by: row.issued_by,
            issued_on: row.issued_on,
            expires_on: row.expires_on,
            file_key: row.file_key,
            created_at: row.created_at,
        }
    }
}

/// List the documents of a product.
///
/// Requires `catalog.documents.manage` covering the product. Newest first unless `order=asc`;
/// paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/products/{id}/documents",
    extensions(("x-permissions" = json!(["catalog.documents.manage"]))),
    operation_id = "list_catalog_product_documents",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product."), PageQuery),
    responses(
        (status = 200, description = "One page of the documents.", body = Page<DocumentResponse>),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.documents.manage`, or it does not cover the product; codes `permission_denied`, `org_out_of_scope`.", body = Problem),
        (status = 404, description = "No product with this id in your scope; code `catalog.product_not_found`.", body = Problem),
    )
)]
async fn list_product_documents(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<DocumentResponse>, Error> {
    principal.require(&DOCUMENTS_MANAGE)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.list_documents(&tenant, &principal, id, &page).await?;
    Ok(rows.map(Into::into))
}

/// A new document.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCreateDocument)]
pub(crate) struct CreateDocumentRequest {
    /// A document type a rule requires, e.g. `alcohol_retail_licence`.
    pub document_type: String,
    pub number: String,
    pub issued_by: String,
    pub issued_on: Date,
    pub expires_on: Option<Date>,
    /// Key of a scanned file already stored.
    pub file_key: Option<String>,
}

/// Add a document to a product.
///
/// Requires `catalog.documents.manage` covering the product. The product's readiness is computed
/// again.
#[utoipa::path(
    post,
    path = "/products/{id}/documents",
    extensions(("x-permissions" = json!(["catalog.documents.manage"]))),
    operation_id = "add_catalog_product_document",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product.")),
    request_body = CreateDocumentRequest,
    responses(
        (status = 201, description = "Created.", body = Data<DocumentResponse>, headers(("Location" = String, description = "Path of the document."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.documents.manage`, or it does not cover the product; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No product with this id in your scope; code `catalog.product_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn add_product_document(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CreateDocumentRequest>,
) -> Result<Created<DocumentResponse>, Error> {
    principal.require(&DOCUMENTS_MANAGE)?;
    let input = NewDocument {
        document_type: input.document_type,
        number: input.number,
        issued_by: input.issued_by,
        issued_on: input.issued_on,
        expires_on: input.expires_on,
        file_key: input.file_key,
    };
    let row = DocumentResponse::from(service.add_document(&tenant, &principal, id, input).await?);
    Ok(Created::new(format!("/catalog/products/{id}/documents/{}", row.id), row))
}

/// Remove a document from a product.
///
/// Requires `catalog.documents.manage` covering the product.
#[utoipa::path(
    delete,
    path = "/products/{id}/documents/{document_id}",
    extensions(("x-permissions" = json!(["catalog.documents.manage"]))),
    operation_id = "delete_catalog_product_document",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product."), ("document_id" = Uuid, Path, description = "Id of the document.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.documents.manage`, or it does not cover the product; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No such product (`catalog.product_not_found`) or document (`catalog.document_not_found`).", body = Problem),
    )
)]
async fn delete_product_document(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, document_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, Error> {
    principal.require(&DOCUMENTS_MANAGE)?;
    service.delete_document(&tenant, &principal, id, document_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// What to count by.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct CountsQuery {
    /// `status` (default) or `readiness`.
    pub group_by: Option<String>,
}

/// A count of products.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogProductCount)]
pub(crate) struct CountResponse {
    /// A status (`draft`, `active`, `inactive`, `archived`) or a readiness (`ready`,
    /// `missing_attributes`, `missing_documents`, `restricted`).
    pub key: String,
    pub count: u64,
}

/// Count the products in your scope by status or readiness.
///
/// Requires `catalog.products.read`. Every key is listed, with 0 when there is no product, in a
/// fixed order (`draft`, `active`, `inactive`, `archived`; or `ready`, `missing_attributes`,
/// `missing_documents`, `restricted`; `order=desc` reverses it); paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/products/counts",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "count_catalog_products",
    security(("bearer" = [])),
    params(CountsQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the counts, one per key.", body = Page<CountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "`group_by` is not `status` or `readiness`, or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn count_products(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<CountsQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<CountResponse>, Error> {
    principal.require(&PRODUCTS_READ)?;
    let page = PageRequest::try_from(page)?;
    let group_by = query.group_by.unwrap_or_else(|| "status".to_owned());
    let counts = service.count_products(&tenant, &principal, &group_by, &page).await?;
    Ok(counts.map(|(key, count)| CountResponse { key, count }))
}
