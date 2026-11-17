//! Route REST của ký hiệu chứng từ và uỷ nhiệm lập hoá đơn (dưới `/api/v1/invoices`).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};
use axum::extract::{Path, Query, State};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::entity::{mandate, series};
use crate::service::{InvoicesService, MandateInput, SeriesInput};

pub(crate) fn routes(service: Arc<InvoicesService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_series, create_series))
        .routes(routes!(update_series))
        .routes(routes!(list_mandates, create_mandate))
        .routes(routes!(revoke_mandate))
        .with_state(service)
}

/// A document series (symbol) of a unit.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = InvoicesSeries)]
pub(crate) struct SeriesResponse {
    id: Uuid,
    org_id: Uuid,
    kind: String,
    #[schema(example = "1C26TAA")]
    symbol: String,
    year: i32,
    /// E-invoice provider; `null`: internal documents only.
    einvoice_provider: Option<String>,
    /// `active` or `inactive`.
    status: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<series::Model> for SeriesResponse {
    fn from(row: series::Model) -> Self {
        SeriesResponse {
            id: row.id,
            org_id: row.org_id,
            kind: row.kind,
            symbol: row.symbol,
            year: row.year,
            einvoice_provider: row.einvoice_provider,
            status: row.status,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A series to create.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InvoicesSeriesRequest)]
pub(crate) struct SeriesRequest {
    /// The unit; default the tenant's root.
    org_id: Option<Uuid>,
    /// Document kind, e.g. `vat_invoice`.
    kind: String,
    /// Symbol registered with the tax authority, 1 to 16 letters and digits.
    #[schema(example = "1C26TAA")]
    symbol: String,
    /// Year the numbers run in.
    year: i32,
    /// E-invoice provider set up on this server; omit for internal documents.
    einvoice_provider: Option<String>,
}

/// A change of series status.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InvoicesSeriesChange)]
pub(crate) struct SeriesChange {
    /// `active` or `inactive`.
    status: String,
}

/// List the series in your scope.
///
/// Requires `invoices.read`.
#[utoipa::path(
    get,
    path = "/series",
    extensions(("x-permissions" = json!(["invoices.read"]))),
    operation_id = "list_invoice_series",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of series.", body = Page<SeriesResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_series(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<SeriesResponse>, Error> {
    let page = service.list_series(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(SeriesResponse::from))
}

/// Create a series.
///
/// Requires `invoices.series_manage` on the unit. Numbers of a series run 1, 2, 3… with no gap.
#[utoipa::path(
    post,
    path = "/series",
    extensions(("x-permissions" = json!(["invoices.series_manage"]))),
    operation_id = "create_invoice_series",
    security(("bearer" = [])),
    request_body = SeriesRequest,
    responses(
        (status = 201, description = "The series.", body = Data<SeriesResponse>, headers(("Location" = String, description = "Path of the series list."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.series_manage` (`permission_denied`) or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 409, description = "The unit has this series already; code `invoices.series_exists`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn create_series(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<SeriesRequest>,
) -> Result<Created<SeriesResponse>, Error> {
    let org = input.org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
    let row = service
        .create_series(
            &tenant,
            &principal,
            SeriesInput {
                org,
                kind: input.kind,
                symbol: input.symbol,
                year: input.year,
                einvoice_provider: input.einvoice_provider,
            },
        )
        .await?;
    Ok(Created::new("/invoices/series".to_owned(), row.into()))
}

/// Retire or reuse a series.
///
/// Requires `invoices.series_manage` on the unit. An inactive series issues no new number.
#[utoipa::path(
    patch,
    path = "/series/{id}",
    extensions(("x-permissions" = json!(["invoices.series_manage"]))),
    operation_id = "update_invoice_series",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the series.")),
    request_body = SeriesChange,
    responses(
        (status = 200, description = "The series after the change.", body = Data<SeriesResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.series_manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such series in your scope; code `invoices.series_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn update_series(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<SeriesChange>,
) -> Result<Data<SeriesResponse>, Error> {
    let row = service.set_series_status(&tenant, &principal, id, input.status).await?;
    Ok(Data { data: row.into() })
}

/// A mandate to issue invoices on behalf of a seller.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = InvoicesMandate)]
pub(crate) struct MandateResponse {
    id: Uuid,
    /// The seller who gives the mandate.
    principal_org_id: Uuid,
    /// The unit that issues for the seller (e.g. the marketplace).
    agent_org_id: Uuid,
    kinds: Vec<String>,
    valid_from: NaiveDate,
    valid_to: Option<NaiveDate>,
    /// The signed mandate (file in the `files` module).
    document_file_id: Option<Uuid>,
    /// `active` or `revoked`.
    status: String,
    created_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

impl From<mandate::Model> for MandateResponse {
    fn from(row: mandate::Model) -> Self {
        MandateResponse {
            id: row.id,
            principal_org_id: row.principal_org_id,
            agent_org_id: row.agent_org_id,
            kinds: serde_json::from_str(&row.kinds).unwrap_or_default(),
            valid_from: row.valid_from,
            valid_to: row.valid_to,
            document_file_id: row.document_file_id,
            status: row.status,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
        }
    }
}

/// A mandate to record.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InvoicesMandateRequest)]
pub(crate) struct MandateRequest {
    /// The seller giving the mandate.
    principal_org_id: Uuid,
    /// The unit receiving it.
    agent_org_id: Uuid,
    /// Kinds of invoice, e.g. `["vat_invoice"]`.
    kinds: Vec<String>,
    valid_from: NaiveDate,
    valid_to: Option<NaiveDate>,
    document_file_id: Option<Uuid>,
}

/// List the mandates of sellers in your scope.
///
/// Requires `invoices.read`.
#[utoipa::path(
    get,
    path = "/mandates",
    extensions(("x-permissions" = json!(["invoices.read"]))),
    operation_id = "list_invoice_mandates",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of mandates.", body = Page<MandateResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_mandates(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<MandateResponse>, Error> {
    let page = service.list_mandates(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(MandateResponse::from))
}

/// Record a mandate to issue invoices on behalf of a seller.
///
/// Requires `invoices.mandates_manage` on the seller's unit. Keep the signed mandate in `files` and
/// link it (`document_file_id`). Check the rules on mandated invoicing against the original texts.
#[utoipa::path(
    post,
    path = "/mandates",
    extensions(("x-permissions" = json!(["invoices.mandates_manage"]))),
    operation_id = "create_invoice_mandate",
    security(("bearer" = [])),
    request_body = MandateRequest,
    responses(
        (status = 201, description = "The mandate.", body = Data<MandateResponse>, headers(("Location" = String, description = "Path of the mandate list."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.mandates_manage` (`permission_denied`) or the seller is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn create_mandate(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<MandateRequest>,
) -> Result<Created<MandateResponse>, Error> {
    let row = service
        .create_mandate(
            &tenant,
            &principal,
            MandateInput {
                principal: OrgId::from_uuid(input.principal_org_id),
                agent: OrgId::from_uuid(input.agent_org_id),
                kinds: input.kinds,
                valid_from: input.valid_from,
                valid_to: input.valid_to,
                document_file_id: input.document_file_id,
            },
        )
        .await?;
    Ok(Created::new("/invoices/mandates".to_owned(), row.into()))
}

/// Revoke a mandate.
///
/// Requires `invoices.mandates_manage` on the seller's unit. Revoking a revoked mandate returns it.
#[utoipa::path(
    post,
    path = "/mandates/{id}/revoke",
    extensions(("x-permissions" = json!(["invoices.mandates_manage"]))),
    operation_id = "revoke_invoice_mandate",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the mandate.")),
    responses(
        (status = 200, description = "The revoked mandate.", body = Data<MandateResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.mandates_manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such mandate in your scope; code `invoices.mandate_not_found`.", body = Problem),
    )
)]
async fn revoke_mandate(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<MandateResponse>, Error> {
    Ok(Data { data: service.revoke_mandate(&tenant, &principal, id).await?.into() })
}
