//! Endpoint quy tắc làm tròn giá đẹp và đổi giá hàng loạt (D-222), dưới `/api/v1/catalog`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::bulk::{ChangeFilter, NewChange};
use crate::derived::NewRoundingRule;
use crate::entity::{price_change, rounding_rule};
use crate::permissions::{PRICES_APPROVE, PRICES_READ, PRICES_WRITE};
use crate::service::CatalogService;

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_rounding_rules, create_rounding_rule))
        .routes(routes!(create_price_change))
        .routes(routes!(get_price_change))
        .routes(routes!(preview_price_change))
        .routes(routes!(approve_price_change))
        .with_state(service)
}

/// A rule that rounds prices to nice values: multiples of `step_minor`, or `k × step_minor +
/// ending_minor` (step 10000 and ending 9000 give 129000, 199000…).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogRoundingRule)]
pub(crate) struct RoundingRuleResponse {
    pub id: Uuid,
    pub name: String,
    pub step_minor: i64,
    /// `up`, `down` or `nearest`.
    pub mode: String,
    pub ending_minor: Option<i64>,
    pub created_at: DateTime<Utc>,
}

impl From<rounding_rule::Model> for RoundingRuleResponse {
    fn from(row: rounding_rule::Model) -> Self {
        RoundingRuleResponse {
            id: row.id,
            name: row.name,
            step_minor: row.step_minor,
            mode: row.mode,
            ending_minor: row.ending_minor,
            created_at: row.created_at,
        }
    }
}

/// List the rounding rules of the tenant.
///
/// Requires `catalog.prices.read`.
#[utoipa::path(
    get,
    path = "/rounding-rules",
    extensions(("x-permissions" = json!(["catalog.prices.read"]))),
    operation_id = "list_catalog_rounding_rules",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every rounding rule.", body = Data<Vec<RoundingRuleResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_rounding_rules(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<RoundingRuleResponse>>, Error> {
    principal.require(&PRICES_READ)?;
    let rows = service.list_rounding_rules(&tenant).await?;
    Ok(Data { data: rows.into_iter().map(Into::into).collect() })
}

/// A new rounding rule.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCreateRoundingRule)]
pub(crate) struct CreateRoundingRuleRequest {
    #[schema(example = "Ends in 9,000")]
    pub name: String,
    /// Step in minor units, above 0, e.g. `10000`.
    pub step_minor: i64,
    /// `up`, `down` or `nearest`.
    pub mode: String,
    /// Ending in minor units, 0 or more and below `step_minor`, e.g. `9000`.
    pub ending_minor: Option<i64>,
}

/// Create a rounding rule.
///
/// Requires `catalog.prices.write`. Rounding is done on whole minor units, never with floating
/// point.
#[utoipa::path(
    post,
    path = "/rounding-rules",
    extensions(("x-permissions" = json!(["catalog.prices.write"]))),
    operation_id = "create_catalog_rounding_rule",
    security(("bearer" = [])),
    request_body = CreateRoundingRuleRequest,
    responses(
        (status = 201, description = "Created.", body = Data<RoundingRuleResponse>, headers(("Location" = String, description = "Path of the rule."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.write`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn create_rounding_rule(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateRoundingRuleRequest>,
) -> Result<Created<RoundingRuleResponse>, Error> {
    principal.require(&PRICES_WRITE)?;
    let input = NewRoundingRule {
        name: input.name,
        step_minor: input.step_minor,
        mode: input.mode,
        ending_minor: input.ending_minor,
    };
    let rule =
        RoundingRuleResponse::from(service.create_rounding_rule(&tenant, &principal, input).await?);
    Ok(Created::new(format!("/catalog/rounding-rules/{}", rule.id), rule))
}

/// Which variants a bulk change covers: the union of the categories (with their subcategories),
/// products and variants listed. All empty: every variant with a price in the list.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogPriceChangeFilter)]
pub(crate) struct FilterDto {
    #[serde(default)]
    pub category_ids: Vec<Uuid>,
    #[serde(default)]
    pub product_ids: Vec<Uuid>,
    #[serde(default)]
    pub variant_ids: Vec<Uuid>,
}

/// A bulk price change.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogPriceChange)]
pub(crate) struct ChangeResponse {
    pub id: Uuid,
    pub price_list_id: Uuid,
    pub filter: FilterDto,
    /// `set`, `percent` or `amount`.
    pub operation: String,
    /// New price (`set`), basis points (`percent`, signed) or minor units added (`amount`, signed).
    pub value: i64,
    pub rounding_rule_id: Option<Uuid>,
    pub valid_from: DateTime<Utc>,
    /// End of a temporary change; the earlier prices apply again after it.
    pub valid_to: Option<DateTime<Utc>>,
    /// `draft`, `previewed`, `approved`, `applied` or `cancelled`.
    pub status: String,
    /// Variants changed (known after the preview).
    pub affected: i64,
    /// The preview: `samples` (up to 20 rows of `variant_id`, `before`, `after`), `floor_violations`,
    /// `blocked` and `warned` counts. Prices below a `block` floor are not written.
    #[schema(value_type = Option<Object>)]
    pub preview: Option<Value>,
    pub created_by: Option<Uuid>,
    pub approved_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<price_change::Model> for ChangeResponse {
    fn from(row: price_change::Model) -> Self {
        let filter: ChangeFilter = serde_json::from_str(&row.filter).unwrap_or_default();
        ChangeResponse {
            id: row.id,
            price_list_id: row.price_list_id,
            filter: FilterDto {
                category_ids: filter.category_ids,
                product_ids: filter.product_ids,
                variant_ids: filter.variant_ids,
            },
            operation: row.operation,
            value: row.value,
            rounding_rule_id: row.rounding_rule_id,
            valid_from: row.valid_from,
            valid_to: row.valid_to,
            status: row.status,
            affected: row.affected,
            preview: row.preview.and_then(|raw| serde_json::from_str(&raw).ok()),
            created_by: row.created_by,
            approved_by: row.approved_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A new bulk price change.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCreatePriceChange)]
pub(crate) struct CreateChangeRequest {
    #[serde(default)]
    pub filter: FilterDto,
    /// `set` (the new price is `value`), `percent` (`value` in basis points, signed: `-1000` =
    /// 10% off) or `amount` (`value` minor units added, signed).
    pub operation: String,
    pub value: i64,
    pub rounding_rule_id: Option<Uuid>,
    /// When the new prices start; omitted: now. A future time schedules them.
    pub valid_from: Option<DateTime<Utc>>,
    /// End of a temporary change.
    pub valid_to: Option<DateTime<Utc>>,
}

/// Create a bulk price change for a price list, as a draft.
///
/// Requires `catalog.prices.write` covering the list. Nothing changes until it is previewed and
/// approved.
#[utoipa::path(
    post,
    path = "/price-lists/{id}/changes",
    extensions(("x-permissions" = json!(["catalog.prices.write"]))),
    operation_id = "create_catalog_price_change",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the price list.")),
    request_body = CreateChangeRequest,
    responses(
        (status = 201, description = "Created as a draft.", body = Data<ChangeResponse>, headers(("Location" = String, description = "Path of the change."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.write`, or it does not cover the list; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No such price list (`catalog.price_list_not_found`) or rounding rule (`catalog.rounding_rule_not_found`).", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn create_price_change(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CreateChangeRequest>,
) -> Result<Created<ChangeResponse>, Error> {
    principal.require(&PRICES_WRITE)?;
    let input = NewChange {
        filter: ChangeFilter {
            category_ids: input.filter.category_ids,
            product_ids: input.filter.product_ids,
            variant_ids: input.filter.variant_ids,
        },
        operation: input.operation,
        value: input.value,
        rounding_rule_id: input.rounding_rule_id,
        valid_from: input.valid_from,
        valid_to: input.valid_to,
    };
    let change = ChangeResponse::from(service.create_change(&tenant, &principal, id, input).await?);
    Ok(Created::new(format!("/catalog/price-lists/{id}/changes/{}", change.id), change))
}

/// Get a bulk price change.
///
/// Requires `catalog.prices.read` covering the list.
#[utoipa::path(
    get,
    path = "/price-lists/{id}/changes/{change_id}",
    extensions(("x-permissions" = json!(["catalog.prices.read"]))),
    operation_id = "get_catalog_price_change",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the price list."), ("change_id" = Uuid, Path, description = "Id of the change.")),
    responses(
        (status = 200, description = "The change.", body = Data<ChangeResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such price list (`catalog.price_list_not_found`) or change (`catalog.price_change_not_found`).", body = Problem),
    )
)]
async fn get_price_change(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, change_id)): Path<(Uuid, Uuid)>,
) -> Result<Data<ChangeResponse>, Error> {
    principal.require(&PRICES_READ)?;
    Ok(Data { data: service.get_change(&tenant, &principal, id, change_id).await?.into() })
}

/// Preview a bulk price change.
///
/// Requires `catalog.prices.write` covering the list. Counts the variants, shows up to 20 rows
/// before / after and the price-floor violations; no price is written.
#[utoipa::path(
    post,
    path = "/price-lists/{id}/changes/{change_id}/preview",
    extensions(("x-permissions" = json!(["catalog.prices.write"]))),
    operation_id = "preview_catalog_price_change",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the price list."), ("change_id" = Uuid, Path, description = "Id of the change.")),
    responses(
        (status = 200, description = "The change with its preview.", body = Data<ChangeResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.write`, or it does not cover the list; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No such price list or change; codes `catalog.price_list_not_found`, `catalog.price_change_not_found`.", body = Problem),
        (status = 409, description = "The change was approved already; code `catalog.price_change_state`.", body = Problem),
        (status = 422, description = "The filter matches too many variants; see `errors`.", body = Problem),
    )
)]
async fn preview_price_change(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, change_id)): Path<(Uuid, Uuid)>,
) -> Result<Data<ChangeResponse>, Error> {
    principal.require(&PRICES_WRITE)?;
    Ok(Data { data: service.preview_change(&tenant, &principal, id, change_id).await?.into() })
}

/// Approve a previewed bulk price change; a job writes the prices.
///
/// Requires `catalog.prices.approve` covering the list. When the unit setting
/// `catalog.price_change_requires_second_person` is on, the approver must not be the person who
/// made the change.
#[utoipa::path(
    post,
    path = "/price-lists/{id}/changes/{change_id}/approve",
    extensions(("x-permissions" = json!(["catalog.prices.approve"]))),
    operation_id = "approve_catalog_price_change",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the price list."), ("change_id" = Uuid, Path, description = "Id of the change.")),
    responses(
        (status = 200, description = "Approved; the prices are written by a job (status becomes `applied`).", body = Data<ChangeResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.prices.approve` (`permission_denied`, `org_out_of_scope`), or you made the change and a second person must approve it (`catalog.self_approval`).", body = Problem),
        (status = 404, description = "No such price list or change; codes `catalog.price_list_not_found`, `catalog.price_change_not_found`.", body = Problem),
        (status = 409, description = "Not previewed yet (`catalog.price_change_not_previewed`) or already approved (`catalog.price_change_state`).", body = Problem),
    )
)]
async fn approve_price_change(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, change_id)): Path<(Uuid, Uuid)>,
) -> Result<Data<ChangeResponse>, Error> {
    principal.require(&PRICES_APPROVE)?;
    Ok(Data { data: service.approve_change(&tenant, &principal, id, change_id).await?.into() })
}
