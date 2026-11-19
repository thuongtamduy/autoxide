//! Endpoint công khai (không cần đăng nhập; gửi token thì token phải đúng), dưới
//! `/api/v1/inventory/public`. Tình trạng hàng có `ETag` (dấu vân tay của câu trả lời → 304) và cache
//! ngắn: mức tồn đổi theo từng đơn nên không cache lâu.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::reference::fingerprint;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::public::{MAX_PUBLIC_VARIANTS, PublicAvailability};
use crate::service::InventoryService;

pub(crate) fn routes(service: Arc<InventoryService>) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(public_availability)).with_state(service)
}

/// Query of the public availability.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct AvailabilityQuery {
    /// Variant ids, comma separated (1 to 100).
    pub variant_ids: String,
    /// The store (organization unit); omit it for every store offering pickup.
    pub store_id: Option<Uuid>,
}

/// Public availability of variants at stores.
///
/// No sign-in needed. For each variant and store: `in_stock`, `low` with the `quantity` left (only
/// below a small threshold), `out` or `preorder`. Large stock numbers are never shown. Unknown or
/// hidden variants are left out. Answers carry an `ETag` and may be cached for a few seconds.
#[utoipa::path(
    get,
    path = "/public/availability",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_inventory_availability",
    security((), ("bearer" = [])),
    params(AvailabilityQuery),
    responses(
        (status = 200, description = "One row per variant and store.", body = Data<Vec<PublicAvailability>>, headers(("ETag" = String, description = "Fingerprint of the answer; send it in `If-None-Match` to get 304."))),
        (status = 304, description = "Unchanged since the `ETag` sent in `If-None-Match`."),
        (status = 422, description = "`variant_ids` is empty, has more than 100 ids or an id that is not a UUID; see `errors`.", body = Problem),
    )
)]
async fn public_availability(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    _principal: Option<Principal>,
    Query(query): Query<AvailabilityQuery>,
) -> Result<Response, Error> {
    let mut ids = Vec::new();
    for raw in query.variant_ids.split(',').map(str::trim).filter(|raw| !raw.is_empty()) {
        let id = raw.parse::<Uuid>().map_err(|_| {
            Error::validation(vec![FieldError::new(
                "variant_ids",
                "uuid",
                "every id must be a UUID",
            )])
        })?;
        ids.push(id);
        if ids.len() > MAX_PUBLIC_VARIANTS {
            break;
        }
    }
    let rows = service.public_availability(&tenant, &ids, query.store_id).await?;
    // Có ID khách trong dấu vân tay: hai khách cùng câu trả lời vẫn khác `ETag`.
    let etag = fingerprint(&(tenant.id().as_uuid(), &rows))?;
    let mut response = Data { data: rows }.into_response();
    if let Ok(value) = HeaderValue::from_str(&format!("\"{}\"", &etag[..etag.len().min(32)])) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=5"));
    Ok(response)
}
