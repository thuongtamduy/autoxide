//! Tool MCP của module `catalog`. Mỗi tool: kiểu đầu vào kiểm như thân JSON của REST,
//! permission như endpoint REST tương ứng, thân là lời gọi service — không có logic riêng (luật
//! kiến trúc 8).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageRequest};
use autoxide_kernel::reference::Reference;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::api::PriceQuery;
use crate::http::dto::{
    CategoryResponse, CreateProductRequest, KindDto, PriceResponse, ProductListItem,
    ProductResponse, StatusDto, VariantRequest, VariantResponse,
};
use crate::http::pricing::ResolvedPriceDto;
use crate::http::products::{ProductsQuery, UpdateVariantRequest};
use crate::permissions::{
    PRICES_READ, PRICES_WRITE, PRODUCTS_PUBLISH, PRODUCTS_READ, PRODUCTS_WRITE,
};
use crate::pricing::PriceInput;
use crate::products::{NewProduct, ProductChanges};
use crate::service::CatalogService;

/// Products to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogListProductsInput)]
pub(crate) struct ListProductsInput {
    #[schema(inline)]
    pub status: Option<StatusDto>,
    #[schema(inline)]
    pub kind: Option<KindDto>,
    pub org_id: Option<Uuid>,
    pub category_id: Option<Uuid>,
    pub readiness: Option<String>,
    /// Words in the name, SKU or short description.
    pub q: Option<String>,
    /// Maximum number of products, 1 to 200. Defaults to 50.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    pub count: Option<bool>,
}

/// A product to act on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogProductId)]
pub(crate) struct ProductId {
    /// Id of the product.
    pub id: Uuid,
}

/// Changes to a product.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogUpdateProductInput)]
pub(crate) struct UpdateProductInput {
    pub id: Uuid,
    pub tax_category: Option<String>,
    /// `null` removes the warranty.
    #[serde(default, deserialize_with = "crate::http::dto::present")]
    #[schema(value_type = Option<i32>, nullable)]
    pub warranty_months: Option<Option<i32>>,
    pub lot_tracked: Option<bool>,
    pub serial_tracked: Option<bool>,
    pub default_locale: Option<String>,
}

/// The new sale status of a product.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogSetStatusInput)]
pub(crate) struct SetStatusInput {
    pub id: Uuid,
    #[schema(inline)]
    pub status: StatusDto,
}

/// A variant to create (give `product_id`) or change (give `id`).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogUpsertVariantInput)]
pub(crate) struct UpsertVariantInput {
    /// Id of the variant to change.
    pub id: Option<Uuid>,
    /// Product of a new variant.
    pub product_id: Option<Uuid>,
    /// A new variant (with `product_id`).
    #[schema(inline)]
    pub create: Option<VariantRequest>,
    /// Changes to the variant `id`.
    #[schema(inline)]
    pub update: Option<UpdateVariantRequest>,
}

/// Prices to add to a price list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogAddPricesInput)]
pub(crate) struct AddPricesInput {
    pub price_list_id: Uuid,
    /// Your code for this change; the same reference and content again adds nothing.
    pub reference: String,
    #[schema(inline)]
    pub prices: Vec<PriceItemInput>,
}

// Trường phẳng (client MCP không đọc `$ref`): tiền là số nguyên đơn vị nhỏ nhất dạng chuỗi, theo
// tiền tệ của bảng giá.
/// A price to add.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogPriceItemInput)]
pub(crate) struct PriceItemInput {
    pub variant_id: Uuid,
    /// Integer amount in minor units of the list's currency, e.g. `"150000"` for 150,000 VND.
    pub amount: String,
    /// Struck-through price in minor units, higher than `amount`.
    pub compare_at: Option<String>,
    /// When the price starts; omitted: now.
    pub valid_from: Option<Timestamp>,
    /// End of a temporary price.
    pub valid_to: Option<Timestamp>,
}

fn minor(raw: &str, field: &str) -> Result<i64, Error> {
    raw.trim().parse().map_err(|_| {
        Error::validation(vec![FieldError::new(
            field,
            "format",
            "must be an integer amount in minor units, like `150000`",
        )])
    })
}

/// The price question.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogResolvePriceInput)]
pub(crate) struct ResolvePriceInput {
    pub variant_id: Uuid,
    /// The store selling.
    pub org_id: Uuid,
    pub currency: String,
    pub customer_group: Option<String>,
    /// Default: now.
    pub at: Option<Timestamp>,
}

fn neither_create_nor_update() -> Error {
    Error::validation(vec![FieldError::new(
        "id",
        "invalid",
        "give either `id` with `update`, or `product_id` with `create`",
    )])
}

pub(crate) fn all(service: &Arc<CatalogService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let s = || service.clone();
    let (list, get, create, update, status, delete) = (s(), s(), s(), s(), s(), s());
    let (variant, categories, prices, resolve) = (s(), s(), s(), s());
    let mut tools = vec![
        Tool::new(
            "list_products",
            "List the products in your scope, newest first; filter by status, kind, unit, \
             category, readiness or words; follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: ListProductsInput| {
                let service = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None)
                        .with_count(input.count.unwrap_or(false))
                        .page()?;
                    let filter = ProductsQuery {
                        status: input.status,
                        kind: input.kind,
                        org_id: input.org_id,
                        category_id: input.category_id,
                        readiness: input.readiness,
                        q: input.q,
                    }
                    .filter()?;
                    let found = service.list_products(&tenant, &principal, &filter, &page).await?;
                    Ok::<Page<ProductListItem>, Error>(found.map(ProductListItem::from))
                }
            },
        )
        .requires(PRODUCTS_READ)
        .read_only(),
        Tool::new(
            "get_product",
            "Get a product with its texts, options, variants, categories and images.",
            move |tenant: TenantContext, principal: Principal, input: ProductId| {
                let service = get.clone();
                async move {
                    let product = service.get_product(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(ProductResponse::from(product))
                }
            },
        )
        .requires(PRODUCTS_READ)
        .read_only(),
        Tool::new(
            "create_product",
            "Create a product as a draft with its first variant (SKU unique in the tenant).",
            move |tenant: TenantContext, principal: Principal, input: CreateProductRequest| {
                let service = create.clone();
                async move {
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
                    Ok::<_, Error>(ProductResponse::from(product))
                }
            },
        )
        .requires(PRODUCTS_WRITE),
        Tool::new(
            "update_product",
            "Change the tax category, warranty, tracking or default language of a product.",
            move |tenant: TenantContext, principal: Principal, input: UpdateProductInput| {
                let service = update.clone();
                async move {
                    let changes = ProductChanges {
                        tax_category: input.tax_category,
                        warranty_months: input.warranty_months,
                        lot_tracked: input.lot_tracked,
                        serial_tracked: input.serial_tracked,
                        default_locale: input.default_locale,
                    };
                    let product =
                        service.update_product(&tenant, &principal, input.id, changes, None).await?;
                    Ok::<_, Error>(ProductResponse::from(product))
                }
            },
        )
        .requires(PRODUCTS_WRITE),
        Tool::new(
            "set_product_status",
            "Put a product on sale (`active`), stop it (`inactive`), archive it, or reopen an \
             archived product as a `draft`. On a marketplace, goods of a seller go on sale only once \
             the seller has verified its identity (`compliance.kyc_required`, \
             `catalog.seller_verification_unavailable`).",
            move |tenant: TenantContext, principal: Principal, input: SetStatusInput| {
                let service = status.clone();
                async move {
                    let product = service
                        .set_status(&tenant, &principal, input.id, input.status.into(), None)
                        .await?;
                    Ok::<_, Error>(ProductResponse::from(product))
                }
            },
        )
        .requires(PRODUCTS_PUBLISH),
        Tool::new(
            "delete_product",
            "Delete a product that was never on sale, with all its variants. It cannot be \
             restored; archive products that were on sale instead.",
            move |tenant: TenantContext, principal: Principal, input: ProductId| {
                let service = delete.clone();
                async move {
                    service.delete_product(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(serde_json::json!({ "deleted": input.id }))
                }
            },
        )
        .requires(PRODUCTS_WRITE)
        .destructive(),
        Tool::new(
            "upsert_variant",
            "Create a variant (give `product_id` and `create`) or change one (give `id` and \
             `update`).",
            move |tenant: TenantContext, principal: Principal, input: UpsertVariantInput| {
                let service = variant.clone();
                async move {
                    let row = match (input.id, input.product_id, input.create, input.update) {
                        (Some(id), None, None, Some(update)) => {
                            service.update_variant(&tenant, &principal, id, update.into(), None).await?
                        }
                        (None, Some(product), Some(create), None) => {
                            service.create_variant(&tenant, &principal, product, create.into()).await?
                        }
                        _ => {
                            return Err(neither_create_nor_update());
                        }
                    };
                    Ok::<_, Error>(VariantResponse::from(&row))
                }
            },
        )
        .requires(PRODUCTS_WRITE),
        // mcp-principal: danh mục chung của khách, không gắn nút tổ chức; REST `GET /categories` cũng trả cả khách.
        Tool::new(
            "list_categories",
            "List the categories of the tenant, by depth then display order (parents before \
             children); follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, _principal: Principal, input: CategoriesInput| {
                let service = categories.clone();
                async move {
                    // Cùng kiểu tham số trang với REST (`NaturalOrderPageQuery`, mặc định `asc`).
                    let page = PageRequest::try_from(
                        NaturalOrderPageQuery::new(
                            input.limit.map(|limit| limit.to_string()),
                            input.cursor,
                            input.order,
                        )
                        .with_count(input.count.unwrap_or(false)),
                    )?;
                    let rows = service.list_categories_page(&tenant, &page).await?;
                    Ok::<Page<CategoryResponse>, Error>(rows.map(CategoryResponse::from))
                }
            },
        )
        .requires(PRODUCTS_READ)
        .read_only(),
        Tool::new(
            "add_prices",
            "Add up to 500 prices to a price list. Rows are only added, never changed; a future \
             `valid_from` schedules a price and `valid_to` makes it temporary.",
            move |tenant: TenantContext, principal: Principal, input: AddPricesInput| {
                let service = prices.clone();
                async move {
                    let mut problems = Vec::new();
                    let Some(reference) =
                        Reference::checked(&input.reference, "reference", &mut problems)
                    else {
                        return Err(Error::validation(problems));
                    };
                    let list =
                        service.get_price_list(&tenant, &principal, input.price_list_id).await?;
                    let currency = Currency::parse(&list.currency)?;
                    let mut items = Vec::new();
                    for item in input.prices {
                        let compare_at = match &item.compare_at {
                            Some(raw) => Some(Money::new(minor(raw, "compare_at")?, currency)),
                            None => None,
                        };
                        items.push(PriceInput {
                            variant_id: item.variant_id,
                            amount: Money::new(minor(&item.amount, "amount")?, currency),
                            compare_at,
                            valid_from: item.valid_from,
                            valid_to: item.valid_to,
                        });
                    }
                    let added = service
                        .add_prices(&tenant, &principal, input.price_list_id, reference, items)
                        .await?;
                    Ok::<_, Error>(serde_json::json!({
                        "prices": added.prices.into_iter().map(PriceResponse::from).collect::<Vec<_>>(),
                        "warnings": added.warnings.iter().map(|warning| serde_json::json!({
                            "variant_id": warning.variant_id,
                            "code": "catalog.price_below_floor",
                            "floor": warning.floor,
                        })).collect::<Vec<_>>(),
                    }))
                }
            },
        )
        .requires(PRICES_WRITE),
        Tool::new(
            "resolve_price",
            "Resolve the price of a variant at a store (optionally for a customer group, at a \
             time) and show which price lists applied, in selection order.",
            move |tenant: TenantContext, principal: Principal, input: ResolvePriceInput| {
                let service = resolve.clone();
                async move {
                    let currency = Currency::parse(&input.currency)?;
                    let at = input.at.unwrap_or_else(Timestamp::now);
                    let mut query = PriceQuery::new(
                        OrgId::from_uuid(input.org_id),
                        currency,
                        at,
                        vec![input.variant_id],
                    );
                    if let Some(group) = input.customer_group {
                        query = query.for_customer_group(group);
                    }
                    let resolved = service.explain_price(&tenant, &principal, &query).await?;
                    Ok::<_, Error>(serde_json::json!({
                        "variant_id": input.variant_id,
                        "price": resolved.price.map(ResolvedPriceDto::from),
                        "candidates": resolved
                            .candidates
                            .into_iter()
                            .map(crate::http::dto::PriceListResponse::from)
                            .collect::<Vec<_>>(),
                    }))
                }
            },
        )
        .requires(PRICES_READ)
        .read_only(),
    ];
    tools.extend(crate::collections::tools::all(service));
    tools.extend(crate::tools_assist::all(service));
    tools
}

/// One page of the categories: the same as the query parameters of `GET /api/v1/catalog/categories`.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogListCategoriesInput)]
pub(crate) struct CategoriesInput {
    /// Maximum number of categories, 1 to 200. Defaults to 50.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub cursor: Option<String>,
    /// `asc` (the default: parents first) or `desc`.
    pub order: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    pub count: Option<bool>,
}
