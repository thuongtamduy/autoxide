//! Thuộc tính theo biến thể (website bán hàng): `GET` / `PUT /variants/{id}/attributes`.

use autoxide_kernel::time::Date;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::entity::variant_attribute;
use crate::permissions::{PRODUCTS_READ, PRODUCTS_WRITE};
use crate::service::CatalogService;
use crate::variant_attributes::VariantAttributeInput;

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(get_variant_attributes, put_variant_attributes))
        .with_state(service)
}

/// The value of one attribute on a variant.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogVariantAttributeValue)]
pub(crate) struct VariantAttributeDto {
    pub attribute_id: Uuid,
    /// By the attribute's type, as for a product: text (in the product's default language), a
    /// whole number, a decimal written as text, `true` / `false`, a date, an option code or a list
    /// of option codes.
    pub value: Value,
    /// For a text attribute: the text in other languages, e.g. `{"en": "Machine washable"}`.
    #[serde(default)]
    pub texts: BTreeMap<String, String>,
}

/// The whole set of attribute values of a variant.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogVariantAttributes)]
pub(crate) struct VariantAttributesRequest {
    pub attributes: Vec<VariantAttributeDto>,
}

/// One stored value of an attribute on a variant (an `options` attribute has one row per option).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogVariantAttributeStored)]
pub(crate) struct VariantAttributeResponse {
    pub attribute_id: Uuid,
    pub value_text: Option<String>,
    pub value_integer: Option<i64>,
    pub value_boolean: Option<bool>,
    pub value_date: Option<Date>,
    pub option_id: Option<Uuid>,
    /// Translations of `value_text`.
    pub texts: BTreeMap<String, String>,
}

impl From<variant_attribute::Model> for VariantAttributeResponse {
    fn from(row: variant_attribute::Model) -> Self {
        VariantAttributeResponse {
            attribute_id: row.attribute_id,
            value_text: row.value_text,
            value_integer: row.value_integer,
            value_boolean: row.value_boolean,
            value_date: row.value_date,
            option_id: row.option_id,
            texts: row
                .texts
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or_default(),
        }
    }
}

/// List the attribute values of a variant.
///
/// Requires `catalog.products.read` covering the product. In the order of the last replace
/// (`PUT`) unless `order=desc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/variants/{id}/attributes",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "get_catalog_variant_attributes",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the variant."), NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the values.", body = Page<VariantAttributeResponse>),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No variant with this id in your scope; code `catalog.variant_not_found`.", body = Problem),
    )
)]
async fn get_variant_attributes(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<VariantAttributeResponse>, Error> {
    principal.require(&PRODUCTS_READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.get_variant_attributes(&tenant, &principal, id, &page).await?;
    Ok(rows.map(Into::into))
}

/// Replace the attribute values of a variant.
///
/// Requires `catalog.products.write` covering the product. For what differs between variants and
/// should show on the product page with the variant chosen (how many people it serves, what is in
/// the box, care instructions, dishwasher safe…): use the tenant's attributes (`POST
/// /attributes`), the same as for products. Text values may carry translations in `texts`. The
/// public product page (`GET /public/products/by-slug/{slug}`) lists them per variant in the
/// language asked.
#[utoipa::path(
    put,
    path = "/variants/{id}/attributes",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "put_catalog_variant_attributes",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the variant.")),
    request_body = VariantAttributesRequest,
    responses(
        (status = 200, description = "The values.", body = Data<Vec<VariantAttributeResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`, or it does not cover this product; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No such variant in your scope (`catalog.variant_not_found`) or attribute (`catalog.attribute_not_found`).", body = Problem),
        (status = 422, description = "A value does not fit its attribute, or a translation is for a language not configured; see `errors`.", body = Problem),
    )
)]
async fn put_variant_attributes(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<VariantAttributesRequest>,
) -> Result<Data<Vec<VariantAttributeResponse>>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let values = input
        .attributes
        .into_iter()
        .map(|item| VariantAttributeInput {
            attribute_id: item.attribute_id,
            value: item.value,
            texts: item.texts,
        })
        .collect();
    let rows = service.put_variant_attributes(&tenant, &principal, id, values).await?;
    Ok(Data { data: rows.into_iter().map(Into::into).collect() })
}
