//! Tạo sản phẩm nháp từ ảnh bằng AI (D-229): `/products/from-images`.

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

use crate::entity::image_draft;
use crate::from_images::NewImageDraft;
use crate::service::CatalogService;

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(start_image_draft))
        .routes(routes!(get_image_draft))
        .with_state(service)
}

/// Draft a product from photos.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = CatalogImageDraftRequest)]
pub(crate) struct ImageDraftRequest {
    /// 1 to 8 images of the media library (JPEG, PNG, GIF or WebP). Public ones become the
    /// product's images.
    pub media_ids: Vec<Uuid>,
    /// Language of the texts, one of `AUTOXIDE_CATALOG_LOCALES`.
    pub locale: String,
    /// Category to use; omitted: the AI picks one of your category tree (or none).
    pub category_id: Option<Uuid>,
    /// Tax category of the new product, e.g. `vat.standard` (the AI never chooses it).
    pub tax_category: String,
    /// Unit owning the new product; leave out when you work in one unit only.
    pub org_id: Option<Uuid>,
}

/// A request to draft a product from photos.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogImageDraft)]
pub(crate) struct ImageDraftResponse {
    pub id: Uuid,
    /// `queued`, `running`, `succeeded` (see `product_id`) or `failed` (see `error_code`).
    pub status: String,
    pub org_id: Uuid,
    pub media_ids: Vec<Uuid>,
    pub locale: String,
    pub category_id: Option<Uuid>,
    /// The new **draft** product (marked `ai_assisted`), once `succeeded`. Check it, set prices,
    /// SKU and stock, then put it on sale yourself.
    pub product_id: Option<Uuid>,
    /// Why it failed, e.g. `catalog.ai_images_unreadable`, `ai.disabled`.
    pub error_code: Option<String>,
    pub created_at: Timestamp,
    pub finished_at: Option<Timestamp>,
}

impl From<image_draft::Model> for ImageDraftResponse {
    fn from(row: image_draft::Model) -> Self {
        ImageDraftResponse {
            id: row.id,
            status: row.status,
            org_id: row.org_id,
            media_ids: serde_json::from_str(&row.media_ids).unwrap_or_default(),
            locale: row.locale,
            category_id: row.category_id,
            product_id: row.product_id,
            error_code: row.error_code,
            created_at: row.created_at,
            finished_at: row.finished_at,
        }
    }
}

/// Draft a product from photos with AI.
///
/// Requires `catalog.products.write` and plan feature `catalog.ai`. Runs in the background: an AI
/// model looks at the photos and writes the name, descriptions, SEO texts, a category of your tree
/// and the attribute values it is sure of. The result is always a **draft** product marked
/// `ai_assisted`, never put on sale; prices, stock and barcode are left for you, and the SKU is a
/// temporary `AI-…` code to replace. Follow `Location` until `status` is `succeeded`.
#[utoipa::path(
    post,
    path = "/products/from-images",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "draft_catalog_product_from_images",
    security(("bearer" = [])),
    request_body = ImageDraftRequest,
    responses(
        (status = 201, description = "Queued.", body = Data<ImageDraftResponse>, headers(("Location" = String, description = "Path of the request."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write` (code `permission_denied`), or the plan lacks `catalog.ai` (code `feature_not_enabled`).", body = Problem),
        (status = 422, description = "A value is invalid (an image not in the media library, too many images, unknown language or category, bad tax category); see `errors`.", body = Problem),
        (status = 503, description = "AI is off (code `ai.disabled`), or the model cannot read images (code `catalog.ai_images_unavailable`).", body = Problem),
    )
)]
async fn start_image_draft(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ImageDraftRequest>,
) -> Result<Created<ImageDraftResponse>, Error> {
    let input = NewImageDraft {
        media_ids: input.media_ids,
        locale: input.locale,
        category_id: input.category_id,
        org_id: input.org_id,
        tax_category: input.tax_category,
    };
    let row = service.start_image_draft(&tenant, &principal, input).await?;
    let response = ImageDraftResponse::from(row);
    Ok(Created::new(format!("/catalog/products/from-images/{}", response.id), response))
}

/// Get a request to draft a product from photos.
///
/// Requires `catalog.products.write`. Only requests you made are visible.
#[utoipa::path(
    get,
    path = "/products/from-images/{id}",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "get_catalog_image_draft",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the request.")),
    responses(
        (status = 200, description = "The request.", body = Data<ImageDraftResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No request of yours has this id; code `catalog.image_draft_not_found`.", body = Problem),
    )
)]
async fn get_image_draft(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ImageDraftResponse>, Error> {
    let row = service.get_image_draft(&tenant, &principal, id).await?;
    Ok(Data { data: ImageDraftResponse::from(row) })
}
