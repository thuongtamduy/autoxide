//! Bản nháp AI cho mô tả và SEO của sản phẩm (D-109): tạo, áp, bỏ.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::copywriting::draft_fields;
use crate::entity::suggestion;
use crate::permissions::PRODUCTS_WRITE;
use crate::service::CatalogService;

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(create_suggestion))
        .routes(routes!(apply_suggestion))
        .routes(routes!(discard_suggestion))
        .with_state(service)
}

/// Which language to draft.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = CatalogSuggestionRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct SuggestionRequest {
    /// Language of the draft (one of the configured product languages).
    #[schema(example = "vi")]
    pub locale: String,
}

/// An AI draft of a product's description and SEO texts. It changes nothing until applied.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogSuggestion)]
pub(crate) struct SuggestionResponse {
    pub id: Uuid,
    pub product_id: Uuid,
    pub locale: String,
    /// `pending`, `applied` or `discarded`.
    pub status: String,
    pub short_description: String,
    pub description: String,
    pub meta_title: String,
    pub meta_description: String,
    /// Model that wrote the draft.
    pub model: String,
    pub created_at: Timestamp,
    pub decided_at: Option<Timestamp>,
}

impl From<suggestion::Model> for SuggestionResponse {
    fn from(row: suggestion::Model) -> Self {
        let fields = draft_fields(&row);
        let field = |name: &str| fields.get(name).cloned().unwrap_or_default();
        SuggestionResponse {
            short_description: field("short_description"),
            description: field("description"),
            meta_title: field("meta_title"),
            meta_description: field("meta_description"),
            id: row.id,
            product_id: row.product_id,
            locale: row.locale,
            status: row.status,
            model: row.model,
            created_at: row.created_at,
            decided_at: row.decided_at,
        }
    }
}

/// Ask AI for a draft description and SEO texts of a product.
///
/// Requires `catalog.products.write` covering the product and the plan feature `catalog.ai`. The
/// draft only uses facts already in the product's texts and is saved as `pending`: nothing on the
/// product changes until someone applies it (`POST /suggestions/{id}/apply`).
#[utoipa::path(
    post,
    path = "/products/{id}/suggestions",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "create_catalog_suggestion",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the product.")),
    request_body = SuggestionRequest,
    responses(
        (status = 201, description = "The draft.", body = Data<SuggestionResponse>, headers(("Location" = String, description = "Path of the draft."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write` (`permission_denied`), or the plan has no `catalog.ai` (`feature_not_available`).", body = Problem),
        (status = 404, description = "No product in your scope has this id; code `catalog.product_not_found`.", body = Problem),
        (status = 422, description = "The language is not configured; code `catalog.unsupported_locale`.", body = Problem),
        (status = 503, description = "The `ai` module is not installed (`catalog.ai_unavailable`), AI is off (`ai.disabled`) or the provider failed (`ai.provider_unavailable`); try again later.", body = Problem),
    )
)]
async fn create_suggestion(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<SuggestionRequest>,
) -> Result<Created<SuggestionResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let row = service.suggest_copy(&tenant, &principal, id, &input.locale).await?;
    let response = SuggestionResponse::from(row);
    Ok(Created::new(format!("/catalog/suggestions/{}", response.id), response))
}

/// Apply an AI draft to the product.
///
/// Requires `catalog.products.write` covering the product. Writes the non-empty draft fields into
/// the product's text in the draft's language (name and address unchanged) and marks the product
/// `ai_assisted`. Change anything you want first with `PUT /products/{id}/translations/{locale}`.
#[utoipa::path(
    post,
    path = "/suggestions/{id}/apply",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "apply_catalog_suggestion",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the draft.")),
    responses(
        (status = 200, description = "The draft, now `applied`.", body = Data<SuggestionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No draft in your scope has this id; code `catalog.suggestion_not_found`.", body = Problem),
        (status = 409, description = "The draft was applied or discarded already; code `catalog.suggestion_decided`.", body = Problem),
        (status = 422, description = "The product has no name in the draft's language yet; see `errors`.", body = Problem),
    )
)]
async fn apply_suggestion(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<SuggestionResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let row = service.apply_suggestion(&tenant, &principal, id).await?;
    Ok(Data { data: SuggestionResponse::from(row) })
}

/// Discard an AI draft.
///
/// Requires `catalog.products.write` covering the product.
#[utoipa::path(
    post,
    path = "/suggestions/{id}/discard",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "discard_catalog_suggestion",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the draft.")),
    responses(
        (status = 200, description = "The draft, now `discarded`.", body = Data<SuggestionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No draft in your scope has this id; code `catalog.suggestion_not_found`.", body = Problem),
        (status = 409, description = "The draft was applied or discarded already; code `catalog.suggestion_decided`.", body = Problem),
    )
)]
async fn discard_suggestion(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<SuggestionResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let row = service.discard_suggestion(&tenant, &principal, id).await?;
    Ok(Data { data: SuggestionResponse::from(row) })
}
