//! Nhập / xuất sản phẩm bằng CSV (chạy bằng job): `/imports`, `/exports`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::entity::transfer;
use crate::service::CatalogService;
use crate::transfers::RowError;

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(start_import))
        .routes(routes!(get_import))
        .routes(routes!(start_export))
        .routes(routes!(get_export))
        .routes(routes!(export_content))
        .with_state(service)
}

/// One problem of one row of an imported file.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogTransferError)]
pub(crate) struct TransferErrorResponse {
    /// Line of the file (the header is line 1); 0 for a problem with the whole file.
    pub row: usize,
    /// Column (or field) at fault; empty when the problem is not about one column.
    pub field: String,
    /// Stable code of the problem (`required`, `format`, `catalog.sku_taken`…).
    pub code: String,
    /// What to fix, in English.
    pub message: String,
}

/// An import or export of products as CSV.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogTransfer)]
pub(crate) struct TransferResponse {
    pub id: Uuid,
    /// `import` or `export`.
    pub kind: String,
    /// `queued`, `running`, `succeeded`, `succeeded_with_errors` (some rows were not written; see
    /// `errors`), `rejected` (the file is not valid; nothing was written) or `failed` (server
    /// problem; start a new one).
    pub status: String,
    /// The unit new products go to (import) or the branch exported; `null`: by your rights.
    pub org_id: Option<Uuid>,
    /// Data rows read (import) or written (export).
    pub total_rows: i64,
    /// Rows written without error.
    pub ok_rows: i64,
    /// Problems by row, at most 1 000.
    pub errors: Vec<TransferErrorResponse>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// For a finished export: where to download the file (`GET`, same sign-in).
    pub content_path: Option<String>,
}

impl From<transfer::Model> for TransferResponse {
    fn from(row: transfer::Model) -> Self {
        let errors: Vec<RowError> = serde_json::from_str(&row.errors).unwrap_or_default();
        let content_path = (row.kind == "export"
            && row.status == crate::transfers::status::SUCCEEDED
            && row.file_key.is_some())
        .then(|| format!("/api/v1/catalog/exports/{}/content", row.id));
        TransferResponse {
            id: row.id,
            kind: row.kind,
            status: row.status,
            org_id: row.org_id,
            total_rows: row.total_rows,
            ok_rows: row.ok_rows,
            errors: errors
                .into_iter()
                .map(|error| TransferErrorResponse {
                    row: error.row,
                    field: error.field,
                    code: error.code,
                    message: error.message,
                })
                .collect(),
            created_at: row.created_at,
            finished_at: row.finished_at,
            content_path,
        }
    }
}

/// Where imported products go.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ImportQuery {
    /// Unit that owns the new products; leave out when you work in one unit only.
    pub org_id: Option<Uuid>,
}

/// Import products from a CSV file.
///
/// Requires `catalog.import`, and the rights of a manual change for every row
/// (`catalog.products.write` on the unit). The body is the file itself (`Content-Type: text/csv`,
/// UTF-8, at most 20 MiB and 50 000 rows). Columns (first line, any order): `sku` (required),
/// `name`, `locale`, `short_description`, `description`, `kind`, `tax_category`, `barcode`,
/// `weight_grams`. A known SKU updates that variant and the product's text in `locale`; a new SKU
/// creates a **draft** product with one variant (needs `name` and `tax_category`). Nothing is put on
/// sale by an import, so importing the same file again changes nothing. The import runs in the
/// background: follow `Location` (`GET /imports/{id}`) for the status and the errors by row.
#[utoipa::path(
    post,
    path = "/imports",
    extensions(("x-permissions" = json!(["catalog.import", "catalog.products.write"]))),
    operation_id = "start_catalog_import",
    security(("bearer" = [])),
    params(ImportQuery),
    request_body(content = String, content_type = "text/csv", description = "The CSV file, UTF-8."),
    responses(
        (status = 201, description = "Queued.", body = Data<TransferResponse>, headers(("Location" = String, description = "Path of the import."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.import`; code `permission_denied`.", body = Problem),
        (status = 413, description = "The file is larger than 20 MiB; code `payload_too_large`.", body = Problem),
        (status = 422, description = "The file is too large; see `errors`.", body = Problem),
        (status = 503, description = "No file storage is installed; code `catalog.storage_unavailable`.", body = Problem),
    )
)]
async fn start_import(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ImportQuery>,
    body: Bytes,
) -> Result<Created<TransferResponse>, Error> {
    let row = service.start_import(&tenant, &principal, query.org_id, body).await?;
    let response = TransferResponse::from(row);
    Ok(Created::new(format!("/catalog/imports/{}", response.id), response))
}

/// Get an import you started: status, counts and errors by row.
///
/// Requires `catalog.import`. Only imports you started are visible.
#[utoipa::path(
    get,
    path = "/imports/{id}",
    extensions(("x-permissions" = json!(["catalog.import"]))),
    operation_id = "get_catalog_import",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the import.")),
    responses(
        (status = 200, description = "The import.", body = Data<TransferResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.import`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No import of yours has this id; code `catalog.transfer_not_found`.", body = Problem),
    )
)]
async fn get_import(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<TransferResponse>, Error> {
    let row = service.get_transfer(&tenant, &principal, "import", id).await?;
    Ok(Data { data: TransferResponse::from(row) })
}

/// What to export.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[schema(as = CatalogExportRequest)]
pub(crate) struct ExportRequest {
    /// Only products of this unit and the units below it; leave out for everything you can read.
    pub org_id: Option<Uuid>,
}

/// Export products to a CSV file.
///
/// Requires `catalog.export` and `catalog.products.read`. Every variant of the products you can
/// read, one row each, with the columns of an import (so the file can be edited and imported
/// back). Cells that a spreadsheet would run as a formula start with `'`. Runs in the background:
/// follow `Location` until `status` is `succeeded`, then download `content_path`.
#[utoipa::path(
    post,
    path = "/exports",
    extensions(("x-permissions" = json!(["catalog.export", "catalog.products.read"]))),
    operation_id = "start_catalog_export",
    security(("bearer" = [])),
    request_body = ExportRequest,
    responses(
        (status = 201, description = "Queued.", body = Data<TransferResponse>, headers(("Location" = String, description = "Path of the export."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.export`; code `permission_denied`.", body = Problem),
        (status = 422, description = "The body is not valid; see `errors`.", body = Problem),
        (status = 503, description = "No file storage is installed; code `catalog.storage_unavailable`.", body = Problem),
    )
)]
async fn start_export(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ExportRequest>,
) -> Result<Created<TransferResponse>, Error> {
    let row = service.start_export(&tenant, &principal, input.org_id).await?;
    let response = TransferResponse::from(row);
    Ok(Created::new(format!("/catalog/exports/{}", response.id), response))
}

/// Get an export you started.
///
/// Requires `catalog.export`. Only exports you started are visible; `content_path` is set once
/// the file is ready (it is kept 7 days).
#[utoipa::path(
    get,
    path = "/exports/{id}",
    extensions(("x-permissions" = json!(["catalog.export"]))),
    operation_id = "get_catalog_export",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the export.")),
    responses(
        (status = 200, description = "The export.", body = Data<TransferResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.export`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No export of yours has this id; code `catalog.transfer_not_found`.", body = Problem),
    )
)]
async fn get_export(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<TransferResponse>, Error> {
    let row = service.get_transfer(&tenant, &principal, "export", id).await?;
    Ok(Data { data: TransferResponse::from(row) })
}

/// Download the CSV file of a finished export.
///
/// Requires `catalog.export`; only exports you started.
#[utoipa::path(
    get,
    path = "/exports/{id}/content",
    extensions(("x-permissions" = json!(["catalog.export"]))),
    operation_id = "download_catalog_export",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the export.")),
    responses(
        (status = 200, description = "The CSV file (UTF-8 with BOM).", body = String, content_type = "text/csv", headers(("Content-Disposition" = String, description = "`attachment` with the file name `products-<id>.csv`."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.export`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No finished export of yours has this id, or its file was cleaned up after 7 days; code `catalog.transfer_not_found`.", body = Problem),
    )
)]
async fn export_content(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Response, Error> {
    let file = service.export_content(&tenant, &principal, id).await?;
    let mut response = Body::from_stream(file.content).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(file.size));
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"products-{id}.csv\""))
    {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}
