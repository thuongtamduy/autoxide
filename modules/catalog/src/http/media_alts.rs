//! `alt` của ảnh sản phẩm theo ngôn ngữ (D-202).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::Problem;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, State};
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::http::dto::ProductResponse;
use crate::permissions::PRODUCTS_WRITE;
use crate::service::CatalogService;

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(put_media_alt, delete_media_alt)).with_state(service)
}

/// The alternative text of an image in one language.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = CatalogMediaAltRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct MediaAltRequest {
    /// What the image shows, for people who cannot see it; 1 to 500 characters.
    pub alt: String,
}

fn tagged(response: ProductResponse) -> Tagged<ProductResponse> {
    Tagged { version: response.updated_at, data: response }
}

/// Set the alternative text of a product image in one language.
///
/// Requires `catalog.products.write` covering the product. The image must be one of the
/// product's images. Public pages in that language show this text; without it they fall back to
/// the product's own `alt` (in the default language), then to the media library's text.
#[utoipa::path(
    put,
    path = "/products/{id}/media/{media_id}/alts/{locale}",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "put_catalog_product_media_alt",
    security(("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Id of the product."),
        ("media_id" = Uuid, Path, description = "Id of the image in the media library."),
        ("locale" = String, Path, description = "Language, e.g. `en`."),
        ("If-Match" = Option<String>, Header, description = "The `ETag` of the product you read; otherwise `412` when it changed since."),
    ),
    request_body = MediaAltRequest,
    responses(
        (status = 200, description = "The product after the change.", body = autoxide_kernel::response::Data<ProductResponse>, headers(("ETag" = String, description = "Version of the record."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`, or it does not cover this product; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No product with this id in your scope (`catalog.product_not_found`), or the image is not one of its images (`catalog.product_media_not_found`).", body = Problem),
        (status = 412, description = "`If-Match` does not match; code `precondition_failed`.", body = Problem),
        (status = 422, description = "The text is empty or too long (see `errors`), or the language is not configured (`catalog.unsupported_locale`).", body = Problem),
    )
)]
async fn put_media_alt(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, media_id, locale)): Path<(Uuid, Uuid, String)>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<MediaAltRequest>,
) -> Result<Tagged<ProductResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let expected = if_match.expected()?;
    let detail = service
        .put_media_alt(&tenant, &principal, id, media_id, &locale, &input.alt, expected)
        .await?;
    Ok(tagged(detail.into()))
}

/// Remove the alternative text of a product image in one language.
///
/// Requires `catalog.products.write` covering the product. Public pages fall back as described
/// for `PUT`.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    delete,
    path = "/products/{id}/media/{media_id}/alts/{locale}",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "delete_catalog_product_media_alt",
    security(("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Id of the product."),
        ("media_id" = Uuid, Path, description = "Id of the image in the media library."),
        ("locale" = String, Path, description = "Language, e.g. `en`."),
        ("If-Match" = Option<String>, Header, description = "The `ETag` of the product you read; otherwise `412` when it changed since."),
    ),
    responses(
        (status = 200, description = "The product after the change.", body = autoxide_kernel::response::Data<ProductResponse>, headers(("ETag" = String, description = "Version of the record."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`, or it does not cover this product; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No product with this id in your scope (`catalog.product_not_found`), or the image is not one of its images (`catalog.product_media_not_found`).", body = Problem),
        (status = 412, description = "`If-Match` does not match; code `precondition_failed`.", body = Problem),
        (status = 422, description = "The language is not configured; code `catalog.unsupported_locale`.", body = Problem),
    )
)]
async fn delete_media_alt(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, media_id, locale)): Path<(Uuid, Uuid, String)>,
    if_match: IfMatch,
) -> Result<Tagged<ProductResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let expected = if_match.expected()?;
    let detail =
        service.delete_media_alt(&tenant, &principal, id, media_id, &locale, expected).await?;
    Ok(tagged(detail.into()))
}
