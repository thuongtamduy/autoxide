//! Endpoint quản trị danh mục, bảng giá, dòng giá và sàn giá, dưới `/api/v1/catalog`.

use autoxide_kernel::time::Timestamp;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest};
use autoxide_kernel::reference::Reference;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::{PriceQuery, PriceSnapshot};
use crate::categories::NewCategory;
use crate::floors::NewGuard;
use crate::http::dto::{
    CategoryResponse, CategoryTextDto, FloorActionDto, PriceGuardResponse, PriceListResponse,
    PriceResponse, present,
};
use crate::permissions::{CATEGORIES_MANAGE, PRICES_READ, PRICES_WRITE, PRODUCTS_READ};
use crate::pricing::{NewPriceList, PriceInput, PriceListChanges};
use crate::service::CatalogService;

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_categories, create_category))
        .routes(routes!(update_category, delete_category))
        .routes(routes!(move_category))
        .routes(routes!(list_price_lists, create_price_list))
        .routes(routes!(update_price_list))
        .routes(routes!(price_history, add_prices))
        .routes(routes!(resolve_price))
        .routes(routes!(list_price_guards, create_price_guard))
        .routes(routes!(delete_price_guard))
        .with_state(service)
}

// ---- Danh mục ------------------------------------------------------------------------------

/// List every category of the tenant.
///
/// Requires `catalog.products.read`. Ordered by depth, then display order, then id: parents before
/// children (`order=desc` reverses it); paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/categories",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "list_catalog_categories",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the categories.", body = Page<CategoryResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_categories(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<CategoryResponse>, Error> {
    principal.require(&PRODUCTS_READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.list_categories_page(&tenant, &page).await?;
    Ok(rows.map(CategoryResponse::from))
}

/// A new category.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCreateCategory)]
pub(crate) struct CreateCategoryRequest {
    /// Omitted: a top category. Categories nest at most 7 levels deep.
    pub parent_id: Option<Uuid>,
    /// Display order among siblings; omitted: last.
    pub position: Option<i32>,
    /// Shown on the storefront; default `true`.
    #[serde(default = "yes")]
    pub visible: bool,
    /// Name and slug per language, e.g. `{"en": {"name": "Shirts"}}`.
    pub texts: BTreeMap<String, CategoryTextDto>,
}

fn yes() -> bool {
    true
}

/// Create a category.
///
/// Requires `catalog.categories.manage` for the whole tenant.
#[utoipa::path(
    post,
    path = "/categories",
    extensions(("x-permissions" = json!(["catalog.categories.manage"]))),
    operation_id = "create_catalog_category",
    security(("bearer" = [])),
    request_body = CreateCategoryRequest,
    responses(
        (status = 201, description = "Created.", body = Data<CategoryResponse>, headers(("Location" = String, description = "Path of the category."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.categories.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "The parent does not exist; code `catalog.category_not_found`.", body = Problem),
        (status = 409, description = "A slug is used already in that language; code `catalog.slug_taken`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`) or a language is not configured (`catalog.unsupported_locale`).", body = Problem),
    )
)]
async fn create_category(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateCategoryRequest>,
) -> Result<Created<CategoryResponse>, Error> {
    principal.require(&CATEGORIES_MANAGE)?;
    let input = NewCategory {
        parent_id: input.parent_id,
        position: input.position,
        visible: input.visible,
        texts: input.texts.into_iter().map(|(locale, text)| (locale, text.into())).collect(),
    };
    let category =
        CategoryResponse::from(service.create_category(&tenant, &principal, input).await?);
    Ok(Created::new(format!("/catalog/categories/{}", category.id), category))
}

/// Changes to a category. Omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogUpdateCategory)]
pub(crate) struct UpdateCategoryRequest {
    pub visible: Option<bool>,
    pub position: Option<i32>,
    /// Names and slugs to set, per language; languages left out stay as they are.
    #[serde(default)]
    pub texts: BTreeMap<String, CategoryTextDto>,
}

/// Change a category.
///
/// Requires `catalog.categories.manage` for the whole tenant.
#[utoipa::path(
    patch,
    path = "/categories/{id}",
    extensions(("x-permissions" = json!(["catalog.categories.manage"]))),
    operation_id = "update_catalog_category",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the category.")),
    request_body = UpdateCategoryRequest,
    responses(
        (status = 200, description = "The category after the change.", body = Data<CategoryResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.categories.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such category; code `catalog.category_not_found`.", body = Problem),
        (status = 409, description = "A slug is used already in that language; code `catalog.slug_taken`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`) or a language is not configured (`catalog.unsupported_locale`).", body = Problem),
    )
)]
async fn update_category(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<UpdateCategoryRequest>,
) -> Result<Data<CategoryResponse>, Error> {
    principal.require(&CATEGORIES_MANAGE)?;
    let texts = input.texts.into_iter().map(|(locale, text)| (locale, text.into())).collect();
    let category = service
        .update_category(&tenant, &principal, id, input.visible, input.position, texts)
        .await?;
    Ok(Data { data: category.into() })
}

/// Delete an empty category.
///
/// Requires `catalog.categories.manage`. The category must have no subcategories or products.
#[utoipa::path(
    delete,
    path = "/categories/{id}",
    extensions(("x-permissions" = json!(["catalog.categories.manage"]))),
    operation_id = "delete_catalog_category",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the category.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.categories.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such category; code `catalog.category_not_found`.", body = Problem),
        (status = 409, description = "It still has subcategories or products; code `catalog.category_not_empty`.", body = Problem),
    )
)]
async fn delete_category(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&CATEGORIES_MANAGE)?;
    service.delete_category(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Where to move a category.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogMoveCategory)]
pub(crate) struct MoveCategoryRequest {
    /// New parent; `null` makes it a top category.
    pub parent_id: Option<Uuid>,
}

/// Move a category with everything below it.
///
/// Requires `catalog.categories.manage`. A category cannot move under itself or its own
/// subcategories.
#[utoipa::path(
    post,
    path = "/categories/{id}/move",
    extensions(("x-permissions" = json!(["catalog.categories.manage"]))),
    operation_id = "move_catalog_category",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the category.")),
    request_body = MoveCategoryRequest,
    responses(
        (status = 200, description = "The category after the move.", body = Data<CategoryResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.categories.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such category or parent; code `catalog.category_not_found`.", body = Problem),
        (status = 409, description = "The new parent is inside the category; code `catalog.category_cycle`.", body = Problem),
        (status = 422, description = "The tree would be too deep; see `errors`.", body = Problem),
    )
)]
async fn move_category(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<MoveCategoryRequest>,
) -> Result<Data<CategoryResponse>, Error> {
    principal.require(&CATEGORIES_MANAGE)?;
    Ok(Data { data: service.move_category(&tenant, &principal, id, input.parent_id).await?.into() })
}

// ---- Bảng giá ------------------------------------------------------------------------------

/// List the price lists in your scope.
///
/// Requires `catalog.prices.read`.
#[utoipa::path(
    get,
    path = "/price-lists",
    extensions(("x-permissions" = json!(["catalog.prices.read"]))),
    operation_id = "list_catalog_price_lists",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of price lists.", body = Page<PriceListResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_price_lists(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<PriceListResponse>, Error> {
    principal.require(&PRICES_READ)?;
    let page = service.list_price_lists(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(PriceListResponse::from))
}

/// A new price list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCreatePriceList)]
pub(crate) struct CreatePriceListRequest {
    /// The store or chain unit; prices apply there and below. Omit when you work in one unit.
    pub org_id: Option<Uuid>,
    #[schema(example = "Retail")]
    pub name: String,
    /// ISO 4217 code. Consumer prices in Viet Nam must be `VND`.
    #[schema(example = "VND")]
    pub currency: String,
    /// Prices include tax; consumer prices in Viet Nam must.
    pub tax_inclusive: bool,
    /// `consumer` (default) or `business`.
    #[serde(default = "consumer")]
    pub audience: String,
    /// Only for this customer group (a free code until customer groups exist, step 47).
    pub customer_group: Option<String>,
    /// Higher wins among lists that tie on unit and filters. Default 0.
    #[serde(default)]
    pub priority: i32,
    /// Make a derived list: its prices are written from this source list (which cannot itself be
    /// derived; `catalog.derived_chain`), adjusted and rounded. Same currency as the source.
    pub derived_from: Option<Uuid>,
    /// Adjustment of a derived list in basis points, signed (`-1500` = 15% off), -10000 to 100000.
    #[serde(default)]
    pub adjust_bp: i64,
    /// Amount added after the percentage, in minor units, signed.
    pub adjust_minor: Option<i64>,
    /// Rounding rule of a derived list (`GET /rounding-rules`).
    pub rounding_rule_id: Option<Uuid>,
}

fn consumer() -> String {
    "consumer".to_owned()
}

/// Create a price list.
///
/// Requires `catalog.prices.write` on the unit; a second list in the tenant needs the plan
/// feature `catalog.price_lists`. Consumer lists of a unit with a Vietnamese tax profile must be
/// in VND and include tax (Price Law 16/2023/QH15).
#[utoipa::path(
    post,
    path = "/price-lists",
    extensions(("x-permissions" = json!(["catalog.prices.write"]))),
    operation_id = "create_catalog_price_list",
    security(("bearer" = [])),
    request_body = CreatePriceListRequest,
    responses(
        (status = 201, description = "Created.", body = Data<PriceListResponse>, headers(("Location" = String, description = "Path of the price list."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.write` or the plan feature (`feature_not_enabled`), or the unit is outside your scope (`org_out_of_scope`, `org_frozen`).", body = Problem),
        (status = 404, description = "The source list (`catalog.price_list_not_found`) or rounding rule (`catalog.rounding_rule_not_found`) does not exist.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), the currency is unknown (`money.currency_unknown`) or differs from the source list (`catalog.currency_mismatch`), the source list is derived (`catalog.derived_chain`), or the consumer rules are broken (`catalog.consumer_price_rules`).", body = Problem),
    )
)]
async fn create_price_list(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreatePriceListRequest>,
) -> Result<Created<PriceListResponse>, Error> {
    principal.require(&PRICES_WRITE)?;
    let input = NewPriceList {
        org_id: input.org_id,
        name: input.name,
        currency: Currency::parse(&input.currency)?,
        tax_inclusive: input.tax_inclusive,
        audience: input.audience,
        customer_group: input.customer_group,
        priority: input.priority,
        derived_from: input.derived_from,
        adjust_bp: input.adjust_bp,
        adjust_minor: input.adjust_minor,
        rounding_rule_id: input.rounding_rule_id,
    };
    let list =
        PriceListResponse::from(service.create_price_list(&tenant, &principal, input).await?);
    Ok(Created::new(format!("/catalog/price-lists/{}", list.id), list))
}

/// Changes to a price list. Omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogUpdatePriceList)]
pub(crate) struct UpdatePriceListRequest {
    pub name: Option<String>,
    pub active: Option<bool>,
    pub priority: Option<i32>,
    /// `null` makes the list apply to every customer.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub customer_group: Option<Option<String>>,
}

/// Change a price list.
///
/// Requires `catalog.prices.write` covering the list.
#[utoipa::path(
    patch,
    path = "/price-lists/{id}",
    extensions(("x-permissions" = json!(["catalog.prices.write"]))),
    operation_id = "update_catalog_price_list",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the price list."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read; otherwise `412` when it changed since.")),
    request_body = UpdatePriceListRequest,
    responses(
        (status = 200, description = "The list after the change.", body = Data<PriceListResponse>, headers(("ETag" = String, description = "Version of the record."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.write`, or it does not cover the list; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No price list with this id in your scope; code `catalog.price_list_not_found`.", body = Problem),
        (status = 412, description = "`If-Match` does not match; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_price_list(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdatePriceListRequest>,
) -> Result<Tagged<PriceListResponse>, Error> {
    principal.require(&PRICES_WRITE)?;
    let changes = PriceListChanges {
        name: input.name,
        active: input.active,
        priority: input.priority,
        customer_group: input.customer_group,
    };
    let expected = if_match.expected()?;
    let list = service.update_price_list(&tenant, &principal, id, changes, expected).await?;
    Ok(Tagged { version: list.updated_at, data: list.into() })
}

/// Filter of the price history.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct HistoryQuery {
    /// Only rows of this variant.
    pub variant_id: Option<Uuid>,
}

/// List the price rows of a price list, newest first.
///
/// Requires `catalog.prices.read` covering the list. Rows are never changed, so this is the full
/// price history.
#[utoipa::path(
    get,
    path = "/price-lists/{id}/prices",
    extensions(("x-permissions" = json!(["catalog.prices.read"]))),
    operation_id = "list_catalog_prices",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the price list."), HistoryQuery, PageQuery),
    responses(
        (status = 200, description = "One page of price rows.", body = Page<PriceResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No price list with this id in your scope; code `catalog.price_list_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn price_history(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<HistoryQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<PriceResponse>, Error> {
    principal.require(&PRICES_READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.price_history(&tenant, &principal, id, query.variant_id, &page).await?;
    Ok(rows.map(PriceResponse::from))
}

/// A price to add.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogPriceInput)]
pub(crate) struct PriceRequest {
    pub variant_id: Uuid,
    /// In the currency of the list, not negative.
    #[schema(inline)]
    pub amount: Money,
    /// Struck-through price, higher than `amount`; display only.
    #[schema(inline)]
    pub compare_at: Option<Money>,
    /// When the price starts; omitted: now. A future time schedules the price.
    pub valid_from: Option<Timestamp>,
    /// End of a temporary price; the earlier price applies again after it.
    pub valid_to: Option<Timestamp>,
}

/// Prices to add to a price list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogAddPrices)]
pub(crate) struct AddPricesRequest {
    /// Your code for this change (1 to 100 visible ASCII characters): the same reference and
    /// content again returns the rows written the first time instead of adding them twice.
    #[schema(example = "price-update-2026-11-18")]
    pub reference: String,
    /// 1 to 500 prices.
    pub prices: Vec<PriceRequest>,
}

/// Prices added, with warnings.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogAddedPrices)]
pub(crate) struct AddedPricesResponse {
    pub prices: Vec<PriceResponse>,
    /// Prices below a `warn` price floor; they were saved.
    pub warnings: Vec<PriceWarningDto>,
}

/// A price below a `warn` price floor.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogPriceWarning)]
pub(crate) struct PriceWarningDto {
    pub variant_id: Uuid,
    /// Always `catalog.price_below_floor`.
    pub code: String,
    pub floor: Money,
}

/// Add prices to a price list.
///
/// Requires `catalog.prices.write` covering the list. Rows are only ever added: a new price is a
/// new row with its own `valid_from`. A price below a `block` floor is refused; below a `warn`
/// floor it is saved with a warning.
#[utoipa::path(
    post,
    path = "/price-lists/{id}/prices",
    extensions(("x-permissions" = json!(["catalog.prices.write"]))),
    operation_id = "add_catalog_prices",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the price list.")),
    request_body = AddPricesRequest,
    responses(
        (status = 200, description = "The rows (also when `reference` was sent before with the same content), with warnings.", body = Data<AddedPricesResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.write`, or it does not cover the list; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No such price list (`catalog.price_list_not_found`) or variant (`catalog.variant_not_found`).", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), a currency differs from the list (`catalog.currency_mismatch`), a price is below a `block` floor (`catalog.price_below_floor`), or the reference was used for other content (`reference_reused`).", body = Problem),
    )
)]
async fn add_prices(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AddPricesRequest>,
) -> Result<Data<AddedPricesResponse>, Error> {
    principal.require(&PRICES_WRITE)?;
    let mut problems = Vec::new();
    let reference = Reference::checked(&input.reference, "reference", &mut problems);
    let Some(reference) = reference else {
        return Err(Error::validation(problems));
    };
    let items = input
        .prices
        .into_iter()
        .map(|item| PriceInput {
            variant_id: item.variant_id,
            amount: item.amount,
            compare_at: item.compare_at,
            valid_from: item.valid_from,
            valid_to: item.valid_to,
        })
        .collect();
    let added = service.add_prices(&tenant, &principal, id, reference, items).await?;
    Ok(Data {
        data: AddedPricesResponse {
            prices: added.prices.into_iter().map(PriceResponse::from).collect(),
            warnings: added
                .warnings
                .into_iter()
                .map(|warning| PriceWarningDto {
                    variant_id: warning.variant_id,
                    code: "catalog.price_below_floor".to_owned(),
                    floor: warning.floor,
                })
                .collect(),
        },
    })
}

/// The price question.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct ResolveQuery {
    /// The store selling.
    pub org_id: Uuid,
    /// ISO 4217 code.
    pub currency: String,
    /// The buyer's customer group, for prices set for a group; omit for the price everyone gets.
    pub customer_group: Option<String>,
    /// Default: now.
    pub at: Option<Timestamp>,
}

/// The price of a variant and why.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogResolvedPrice)]
pub(crate) struct ResolvedPriceResponse {
    pub variant_id: Uuid,
    /// The winning price; `null` when no candidate list has a price for the variant.
    pub price: Option<ResolvedPriceDto>,
    /// The lists that apply, in selection order; the first with a price wins.
    pub candidates: Vec<PriceListResponse>,
}

/// A resolved price.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogPriceSnapshot)]
pub(crate) struct ResolvedPriceDto {
    pub price_list_id: Uuid,
    pub price_id: Uuid,
    pub amount: Money,
    pub tax_inclusive: bool,
    pub compare_at: Option<Money>,
    pub valid_from: Timestamp,
    pub valid_to: Option<Timestamp>,
}

impl From<PriceSnapshot> for ResolvedPriceDto {
    fn from(price: PriceSnapshot) -> Self {
        ResolvedPriceDto {
            price_list_id: price.price_list_id,
            price_id: price.price_id,
            amount: price.amount,
            tax_inclusive: price.tax_inclusive,
            compare_at: price.compare_at,
            valid_from: price.valid_from,
            valid_to: price.valid_to,
        }
    }
}

/// Resolve the price of a variant at a store, and show which list won.
///
/// Requires `catalog.prices.read` at the store. Order: the unit closest to the store, then a
/// matching company, customer group, channel and market, then higher `priority`, then id.
#[utoipa::path(
    get,
    path = "/variants/{id}/price",
    extensions(("x-permissions" = json!(["catalog.prices.read"]))),
    operation_id = "resolve_catalog_price",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the variant."), ResolveQuery),
    responses(
        (status = 200, description = "The price and the candidate lists.", body = Data<ResolvedPriceResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.read` (`permission_denied`) or the store is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 422, description = "A parameter is invalid (see `errors`) or the currency is unknown (`money.currency_unknown`).", body = Problem),
    )
)]
async fn resolve_price(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<ResolveQuery>,
) -> Result<Data<ResolvedPriceResponse>, Error> {
    principal.require(&PRICES_READ)?;
    let currency = Currency::parse(&query.currency)?;
    let at = query.at.unwrap_or_else(Timestamp::now);
    let mut price_query = PriceQuery::new(OrgId::from_uuid(query.org_id), currency, at, vec![id]);
    if let Some(group) = query.customer_group {
        price_query = price_query.for_customer_group(group);
    }
    let resolved = service.explain_price(&tenant, &principal, &price_query).await?;
    Ok(Data {
        data: ResolvedPriceResponse {
            variant_id: id,
            price: resolved.price.map(Into::into),
            candidates: resolved.candidates.into_iter().map(PriceListResponse::from).collect(),
        },
    })
}

// ---- Sàn giá -------------------------------------------------------------------------------

/// List the price floors in your scope.
///
/// Requires `catalog.prices.read`.
#[utoipa::path(
    get,
    path = "/price-guards",
    extensions(("x-permissions" = json!(["catalog.prices.read"]))),
    operation_id = "list_catalog_price_guards",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of price floors.", body = Page<PriceGuardResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_price_guards(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<PriceGuardResponse>, Error> {
    principal.require(&PRICES_READ)?;
    let page = service.list_guards(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(PriceGuardResponse::from))
}

/// A new price floor.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCreatePriceGuard)]
pub(crate) struct CreateGuardRequest {
    /// The unit it applies to (and below); omit when you work in one unit.
    pub org_id: Option<Uuid>,
    /// Only products in this category or below.
    pub category_id: Option<Uuid>,
    /// ISO 4217 code.
    pub currency: String,
    /// Lowest allowed price.
    pub min_price: Option<Money>,
    /// Lowest margin over unit cost, in basis points (`1500` = 15%), 0 to 100000.
    pub min_margin_bp: Option<i64>,
    pub action: FloorActionDto,
}

/// Create a price floor.
///
/// Requires `catalog.prices.write` on the unit. The rule of the closest unit wins; within a unit,
/// the rule of the closest category wins over a rule without category.
#[utoipa::path(
    post,
    path = "/price-guards",
    extensions(("x-permissions" = json!(["catalog.prices.write"]))),
    operation_id = "create_catalog_price_guard",
    security(("bearer" = [])),
    request_body = CreateGuardRequest,
    responses(
        (status = 201, description = "Created.", body = Data<PriceGuardResponse>, headers(("Location" = String, description = "Path of the rule."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.write`, or the unit is outside your scope; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "The category does not exist; code `catalog.category_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn create_price_guard(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateGuardRequest>,
) -> Result<Created<PriceGuardResponse>, Error> {
    principal.require(&PRICES_WRITE)?;
    let currency = Currency::parse(&input.currency).map_err(|_| {
        Error::validation(vec![FieldError::new("currency", "unknown", "is not an ISO 4217 code")])
    })?;
    let input = NewGuard {
        org_id: input.org_id,
        category_id: input.category_id,
        currency,
        min_price: input.min_price,
        min_margin_bp: input.min_margin_bp,
        action: input.action.into(),
    };
    let guard = PriceGuardResponse::from(service.create_guard(&tenant, &principal, input).await?);
    Ok(Created::new(format!("/catalog/price-guards/{}", guard.id), guard))
}

/// Delete a price floor.
///
/// Requires `catalog.prices.write` covering the rule.
#[utoipa::path(
    delete,
    path = "/price-guards/{id}",
    extensions(("x-permissions" = json!(["catalog.prices.write"]))),
    operation_id = "delete_catalog_price_guard",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the rule.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.write`, or it does not cover the rule; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No such rule in your scope; code `catalog.price_guard_not_found`.", body = Problem),
    )
)]
async fn delete_price_guard(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&PRICES_WRITE)?;
    service.delete_guard(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
