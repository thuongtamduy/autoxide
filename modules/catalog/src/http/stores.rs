//! Tuỳ biến theo cửa hàng: cửa hàng ẩn (hay hiện lại) sản phẩm của chuỗi trên website của mình.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::permissions::PRODUCTS_WRITE;
use crate::service::CatalogService;

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(put_store_override)).with_state(service)
}

/// Whether a store shows a product.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogStoreOverrideInput)]
pub(crate) struct StoreOverrideRequest {
    /// `true`: the store's website (and the websites of units below it) does not show the
    /// product; `false`: shows it again.
    pub hidden: bool,
}

/// A store's choice for a product.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogStoreOverride)]
pub(crate) struct StoreOverrideResponse {
    pub org_id: Uuid,
    pub product_id: Uuid,
    pub hidden: bool,
    pub updated_at: DateTime<Utc>,
}

/// Hide or show a product on a store's website.
///
/// Requires `catalog.products.write` at the store's unit. The public lists of the store (and of
/// the units below it) follow within moments: they are rebuilt by a background job.
#[utoipa::path(
    put,
    path = "/stores/{org_id}/overrides/{product_id}",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "put_catalog_store_override",
    security(("bearer" = [])),
    params(
        ("org_id" = Uuid, Path, description = "Organization unit of the store."),
        ("product_id" = Uuid, Path, description = "Id of the product."),
    ),
    request_body = StoreOverrideRequest,
    responses(
        (status = 200, description = "The choice is saved.", body = Data<StoreOverrideResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write` at the store; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such store in your scope (`site_not_found`) or no such product (`catalog.product_not_found`).", body = Problem),
        (status = 422, description = "The body is invalid; see `errors`.", body = Problem),
    )
)]
async fn put_store_override(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((org_id, product_id)): Path<(Uuid, Uuid)>,
    ValidJson(body): ValidJson<StoreOverrideRequest>,
) -> Result<Data<StoreOverrideResponse>, Error> {
    principal.require(&PRODUCTS_WRITE)?;
    let row =
        service.set_store_override(&tenant, &principal, org_id, product_id, body.hidden).await?;
    Ok(Data {
        data: StoreOverrideResponse {
            org_id: row.org_id,
            product_id: row.product_id,
            hidden: row.hidden,
            updated_at: row.updated_at,
        },
    })
}
