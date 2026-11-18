//! Route REST của chứng từ (dưới `/api/v1/invoices`).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::einvoice::EInvoiceFormat;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Currency;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::NaiveDate;
use serde::{Deserialize, Deserializer};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::{Buyer, Document, NewDocument, NewDocumentLine};
use crate::documents::{DocumentFilter, DraftChange};
use crate::issue::Correction;
use crate::service::InvoicesService;

pub(crate) fn routes(service: Arc<InvoicesService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_documents, create_document))
        .routes(routes!(get_document, update_document, delete_document))
        .routes(routes!(issue_document))
        .routes(routes!(refresh_document))
        .routes(routes!(cancel_document))
        .routes(routes!(adjust_document))
        .routes(routes!(replace_document))
        .routes(routes!(download_einvoice))
        .with_state(service)
}

// Trường JSON có ba trạng thái: không gửi (`None`), `null` (`Some(None)`), có giá trị.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

fn submitted(outcome: Submitted<Document>) -> Response {
    match outcome {
        Submitted::Created(document) => {
            Created::new(format!("/invoices/documents/{}", document.id), document).into_response()
        }
        Submitted::Replayed(document) => Data { data: document }.into_response(),
    }
}

/// A draft document to write.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InvoicesDocumentRequest)]
pub(crate) struct DocumentRequest {
    /// The unit that writes it; default the tenant's root.
    org_id: Option<Uuid>,
    /// The seller it is written for, under a mandate (`POST /mandates`).
    on_behalf_of_org_id: Option<Uuid>,
    /// `vat_invoice`, `sales_invoice`, `cash_register_invoice`, `retail_receipt`, `delivery_note`,
    /// `goods_receipt`, `cash_receipt`, `cash_payment` or `withholding_certificate`.
    #[schema(example = "vat_invoice")]
    kind: String,
    /// Series; default the only active series of the unit for this kind and year.
    series_id: Option<Uuid>,
    /// Planned issue day; default the day it is issued.
    issue_date: Option<NaiveDate>,
    /// Line prices include taxes; default `false`.
    #[serde(default)]
    prices_include_tax: bool,
    #[schema(example = "VND")]
    currency: String,
    #[serde(default)]
    buyer: Buyer,
    lines: Vec<NewDocumentLine>,
    /// Id of your source document (order…).
    source_id: Option<String>,
    /// Your id of this document: sending it again returns the same draft.
    reference: String,
}

/// Changes to a draft. Omitted fields stay; a rejected e-invoice may only change `buyer`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InvoicesDocumentChange)]
pub(crate) struct DocumentChange {
    buyer: Option<Buyer>,
    /// Replaces every line.
    lines: Option<Vec<NewDocumentLine>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    series_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<NaiveDate>, nullable)]
    issue_date: Option<Option<NaiveDate>>,
    prices_include_tax: Option<bool>,
}

/// Which documents.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DocumentsQuery {
    /// Only documents written by this unit.
    org_id: Option<Uuid>,
    kind: Option<String>,
    status: Option<String>,
    /// Only documents created by this module (`invoices` for documents made through this API).
    source_module: Option<String>,
    source_id: Option<String>,
}

/// List documents in your scope.
///
/// Requires `invoices.read`.
#[utoipa::path(
    get,
    path = "/documents",
    extensions(("x-permissions" = json!(["invoices.read"]))),
    operation_id = "list_invoice_documents",
    security(("bearer" = [])),
    params(DocumentsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of documents with their lines.", body = Page<Document>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_documents(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<DocumentsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<Document>, Error> {
    let filter = DocumentFilter {
        org: query.org_id.map(OrgId::from_uuid),
        kind: query.kind,
        status: query.status,
        source_module: query.source_module,
        source_id: query.source_id,
    };
    service.list_documents(&tenant, &principal, filter, &PageRequest::try_from(page)?).await
}

/// Write a draft document.
///
/// Requires `invoices.write` on the unit. Taxes are computed now (`GET /api/v1/tax/rates`); the
/// draft has no number until it is issued. The same `reference` and content again returns the same
/// draft (`200`).
#[utoipa::path(
    post,
    path = "/documents",
    extensions(("x-permissions" = json!(["invoices.write"]))),
    operation_id = "create_invoice_document",
    security(("bearer" = [])),
    request_body = DocumentRequest,
    responses(
        (status = 200, description = "The draft was written before with this `reference` and content.", body = Data<Document>),
        (status = 201, description = "The draft.", body = Data<Document>, headers(("Location" = String, description = "Path of the document."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.write` (`permission_denied`) or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 422, description = "A value is invalid (e.g. buyer tax code, a tax category, a missing rate `tax.rate_missing`), or the reference was used for other content (`reference_reused`).", body = Problem),
    )
)]
async fn create_document(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<DocumentRequest>,
) -> Result<Response, Error> {
    let org = input.org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
    let currency = Currency::parse(&input.currency)?;
    let mut draft = NewDocument::new(
        org,
        input.kind,
        currency,
        input.buyer,
        input.lines,
        "invoices",
        input.reference,
    );
    draft.on_behalf_of = input.on_behalf_of_org_id.map(OrgId::from_uuid);
    draft.series = input.series_id;
    draft.issue_date = input.issue_date;
    draft.prices_include_tax = input.prices_include_tax;
    draft.source_id = input.source_id;
    Ok(submitted(service.create_manual(&tenant, &principal, draft).await?))
}

/// Get a document with its lines.
///
/// Requires `invoices.read` on the unit that wrote it.
#[utoipa::path(
    get,
    path = "/documents/{id}",
    extensions(("x-permissions" = json!(["invoices.read"]))),
    operation_id = "get_invoice_document",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the document.")),
    responses(
        (status = 200, description = "The document.", body = Data<Document>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such document in your scope; code `invoices.document_not_found`.", body = Problem),
    )
)]
async fn get_document(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Document>, Error> {
    Ok(Data { data: service.document_detail(&tenant, &principal, id).await? })
}

/// Change a draft.
///
/// Requires `invoices.write` on the unit. Taxes are computed again. A rejected e-invoice may only
/// change its buyer, then be issued again with the same number.
#[utoipa::path(
    patch,
    path = "/documents/{id}",
    extensions(("x-permissions" = json!(["invoices.write"]))),
    operation_id = "update_invoice_document",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the document.")),
    request_body = DocumentChange,
    responses(
        (status = 200, description = "The document after the change.", body = Data<Document>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such document in your scope; code `invoices.document_not_found`.", body = Problem),
        (status = 409, description = "Not a draft; code `invoices.not_draft`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn update_document(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<DocumentChange>,
) -> Result<Data<Document>, Error> {
    let change = DraftChange {
        buyer: input.buyer,
        lines: input.lines,
        series: input.series_id,
        issue_date: input.issue_date,
        prices_include_tax: input.prices_include_tax,
    };
    Ok(Data { data: service.update_draft(&tenant, &principal, id, change).await? })
}

/// Delete a draft.
///
/// Requires `invoices.write` on the unit. Issued documents are accounting records and are kept.
#[utoipa::path(
    delete,
    path = "/documents/{id}",
    extensions(("x-permissions" = json!(["invoices.write"]))),
    operation_id = "delete_invoice_document",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the document.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such document in your scope; code `invoices.document_not_found`.", body = Problem),
        (status = 409, description = "Issued documents must be kept; code `invoices.retention_hold`.", body = Problem),
    )
)]
async fn delete_document(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_draft(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Issue a document.
///
/// Requires `invoices.issue` on the unit. Gives the next number of the series (no gaps), books the
/// sale, and sends the e-invoice when the series has a provider. Issuing again returns the same
/// document and number; a rejected e-invoice is sent again with the same number.
#[utoipa::path(
    post,
    path = "/documents/{id}/issue",
    extensions(("x-permissions" = json!(["invoices.issue"]))),
    operation_id = "issue_invoice_document",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the document.")),
    responses(
        (status = 200, description = "The issued document.", body = Data<Document>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.issue` (`permission_denied`), the unit is frozen (`org_frozen`), or no mandate for the seller (`invoices.mandate_missing`).", body = Problem),
        (status = 404, description = "No such document (`invoices.document_not_found`) or no series for it (`invoices.series_not_found`).", body = Problem),
        (status = 409, description = "The series is retired (`invoices.series_inactive`) or the booking day is in a closed period (`ledger.period_closed`).", body = Problem),
        (status = 422, description = "The series does not fit the document; every problem in `errors`.", body = Problem),
    )
)]
async fn issue_document(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Document>, Error> {
    Ok(Data { data: service.issue_manual(&tenant, &principal, id).await? })
}

/// Ask the e-invoice provider about a document now.
///
/// Requires `invoices.issue` on the unit. A document sent to the tax authority (`submitting`) is
/// checked with the provider at once instead of waiting for the next periodic check, and becomes
/// `accepted` (with the authority code) or `rejected` (with the reason) when the provider has an
/// answer; still waiting leaves it `submitting`. A document in any other status is returned as it
/// is. Safe to call again.
#[utoipa::path(
    post,
    path = "/documents/{id}/refresh",
    extensions(("x-permissions" = json!(["invoices.issue"]))),
    operation_id = "refresh_invoice_document",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the document.")),
    responses(
        (status = 200, description = "The document after asking.", body = Data<Document>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.issue`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such document; code `invoices.document_not_found`.", body = Problem),
        (status = 503, description = "The provider did not answer; try again later (`einvoice.provider_unavailable`).", body = Problem),
    )
)]
async fn refresh_document(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Document>, Error> {
    Ok(Data { data: service.refresh_manual(&tenant, &principal, id).await? })
}

/// Why the document is cancelled.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InvoicesCancelRequest)]
pub(crate) struct CancelRequest {
    /// 1 to 500 characters.
    reason: String,
}

/// Cancel an issued document.
///
/// Requires `invoices.correct` on the unit. Tells the e-invoice provider, and reverses the booking.
/// Cancelling a cancelled document returns it.
#[utoipa::path(
    post,
    path = "/documents/{id}/cancel",
    extensions(("x-permissions" = json!(["invoices.correct"]))),
    operation_id = "cancel_invoice_document",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the document.")),
    request_body = CancelRequest,
    responses(
        (status = 200, description = "The cancelled document.", body = Data<Document>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.correct`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such document in your scope; code `invoices.document_not_found`.", body = Problem),
        (status = 409, description = "A draft or a corrected document cannot be cancelled; code `invoices.not_cancellable`.", body = Problem),
        (status = 503, description = "The e-invoice provider did not answer (`einvoice.provider_unavailable`).", body = Problem),
    )
)]
async fn cancel_document(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CancelRequest>,
) -> Result<Data<Document>, Error> {
    Ok(Data { data: service.cancel_manual(&tenant, &principal, id, input.reason).await? })
}

/// An adjustment to write.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InvoicesAdjustRequest)]
pub(crate) struct AdjustRequest {
    /// `increase` or `decrease` of the amounts.
    direction: String,
    /// Lines of the difference.
    lines: Vec<NewDocumentLine>,
    /// Corrected buyer, when it changes.
    buyer: Option<Buyer>,
    reference: String,
}

/// A replacement to write.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InvoicesReplaceRequest)]
pub(crate) struct ReplaceRequest {
    /// Every line of the new document.
    lines: Vec<NewDocumentLine>,
    buyer: Option<Buyer>,
    reference: String,
}

/// Write an adjustment of an issued document.
///
/// Requires `invoices.correct` on the unit. Writes a draft linked to the document; issuing the draft
/// marks the document `adjusted` and books the difference.
#[utoipa::path(
    post,
    path = "/documents/{id}/adjust",
    extensions(("x-permissions" = json!(["invoices.correct"]))),
    operation_id = "adjust_invoice_document",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the issued document.")),
    request_body = AdjustRequest,
    responses(
        (status = 200, description = "The draft was written before with this `reference`.", body = Data<Document>),
        (status = 201, description = "The adjustment draft.", body = Data<Document>, headers(("Location" = String, description = "Path of the draft."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.correct`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such document in your scope; code `invoices.document_not_found`.", body = Problem),
        (status = 409, description = "The document is not issued; code `invoices.not_correctable`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn adjust_document(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AdjustRequest>,
) -> Result<Response, Error> {
    let correction = Correction {
        relation: "adjustment",
        direction: Some(input.direction),
        buyer: input.buyer,
        lines: input.lines,
        reference: input.reference,
    };
    Ok(submitted(service.correct(&tenant, &principal, id, correction).await?))
}

/// Write a replacement of an issued document.
///
/// Requires `invoices.correct` on the unit. Writes a draft linked to the document; issuing the draft
/// marks the document `replaced` and reverses its booking.
#[utoipa::path(
    post,
    path = "/documents/{id}/replace",
    extensions(("x-permissions" = json!(["invoices.correct"]))),
    operation_id = "replace_invoice_document",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the issued document.")),
    request_body = ReplaceRequest,
    responses(
        (status = 200, description = "The draft was written before with this `reference`.", body = Data<Document>),
        (status = 201, description = "The replacement draft.", body = Data<Document>, headers(("Location" = String, description = "Path of the draft."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.correct`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such document in your scope; code `invoices.document_not_found`.", body = Problem),
        (status = 409, description = "The document is not issued; code `invoices.not_correctable`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn replace_document(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ReplaceRequest>,
) -> Result<Response, Error> {
    let correction = Correction {
        relation: "replacement",
        direction: None,
        buyer: input.buyer,
        lines: input.lines,
        reference: input.reference,
    };
    Ok(submitted(service.correct(&tenant, &principal, id, correction).await?))
}

/// Which file.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct EInvoiceQuery {
    /// `xml` (default) or `pdf`.
    format: Option<String>,
}

/// Download the e-invoice file from the provider.
///
/// Requires `invoices.read` on the unit.
#[utoipa::path(
    get,
    path = "/documents/{id}/einvoice",
    extensions(("x-permissions" = json!(["invoices.read"]))),
    operation_id = "download_invoice_einvoice",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the document."), EInvoiceQuery),
    responses(
        (status = 200, description = "The file (`application/xml` or `application/pdf`).", content_type = "application/octet-stream", body = Vec<u8>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such document in your scope; code `invoices.document_not_found`.", body = Problem),
        (status = 409, description = "The e-invoice is not sent yet; code `invoices.einvoice_not_ready`.", body = Problem),
        (status = 422, description = "`format` is not `xml` or `pdf`; every problem in `errors`.", body = Problem),
        (status = 503, description = "The provider did not answer (`einvoice.provider_unavailable`).", body = Problem),
    )
)]
async fn download_einvoice(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<EInvoiceQuery>,
) -> Result<Response, Error> {
    let (format, kind) = match query.format.as_deref().unwrap_or("xml") {
        "xml" => (EInvoiceFormat::Xml, "application/xml"),
        "pdf" => (EInvoiceFormat::Pdf, "application/pdf"),
        _ => {
            return Err(Error::validation(vec![FieldError::new(
                "format",
                "invalid",
                "must be `xml` or `pdf`",
            )]));
        }
    };
    let bytes = service.download_einvoice(&tenant, &principal, id, format).await?;
    Ok(([(header::CONTENT_TYPE, kind)], bytes).into_response())
}
