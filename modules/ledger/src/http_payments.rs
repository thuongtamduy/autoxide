//! Route REST của thanh toán, webhook đối tác, đối soát và hàng chờ (dưới `/api/v1/ledger`).

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::{NewPayment, NewRefund, Payment, Refund};
use crate::entity::{exception, reconciliation};
use crate::payments::{PaymentFilter, describe_payment, describe_refund};
use crate::permissions::READ;
use crate::reconcile::Resolution;
use crate::service::LedgerService;

pub(crate) fn routes(service: Arc<LedgerService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_payments, create_payment))
        .routes(routes!(get_payment))
        .routes(routes!(create_refund))
        .routes(routes!(receive_webhook))
        .routes(routes!(receive_tenant_webhook))
        .routes(routes!(list_exceptions))
        .routes(routes!(resolve_exception))
        .routes(routes!(list_reconciliations, run_reconciliation))
        .with_state(service)
}

fn submitted<T: Serialize + ToSchema>(
    outcome: Submitted<T>,
    path: impl FnOnce(&T) -> String,
) -> Response {
    match outcome {
        Submitted::Created(value) => Created::new(path(&value), value).into_response(),
        Submitted::Replayed(value) => Data { data: value }.into_response(),
    }
}

// ---- Thanh toán -----------------------------------------------------------------------------------

/// A payment to create.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerPaymentRequest)]
pub(crate) struct PaymentRequest {
    /// The unit that receives the money; default the tenant's root.
    org_id: Option<Uuid>,
    /// Payment partner; may be left out when the server has only one.
    provider: Option<String>,
    /// `card`, `wallet`, `vietqr`… — one the partner supports.
    #[schema(example = "card")]
    method: String,
    /// In the currency of the unit's book.
    amount: Money,
    /// Short text shown to the buyer, 1 to 140 characters, no personal data.
    description: String,
    /// Page the buyer returns to.
    return_url: Option<String>,
    /// Id of your document (order, invoice…).
    source_id: Option<String>,
    /// Your id of this payment: sending it again returns the same payment (and asks the partner
    /// again if it did not answer the first time).
    reference: String,
}

/// A payment with its refunds.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerPaymentDetail)]
pub(crate) struct PaymentDetail {
    #[serde(flatten)]
    payment: Payment,
    refunds: Vec<Refund>,
}

/// Which payments.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PaymentsQuery {
    /// Only payments with this status.
    status: Option<String>,
    /// Only payments created by this module (`ledger` for payments created through this API).
    source_module: Option<String>,
    /// Only payments for this document.
    source_id: Option<String>,
    /// Only payments of this amount, in the smallest unit of the currency (e.g. `150000` for
    /// 150,000 VND) — to find the payment a bank transfer was meant for.
    #[param(value_type = Option<String>, example = "150000")]
    amount: Option<String>,
    /// Only payments in this currency (ISO 4217, e.g. `VND`).
    #[param(value_type = Option<String>, example = "VND")]
    currency: Option<String>,
    /// Only payments received by this unit (not the units below it).
    org_id: Option<Uuid>,
}

/// Create a payment through a payment partner.
///
/// Requires `ledger.payments_create` on the unit. The money goes to the partner, never to this
/// server; the answer has the partner's `checkout_url` (or `qr_payload`) for the buyer. When the
/// partner reports the payment, the book gets the entry and the event `ledger.payment_succeeded`
/// is sent. If the partner does not answer (`503`), send the same request again.
#[utoipa::path(
    post,
    path = "/payments",
    extensions(("x-permissions" = json!(["ledger.payments_create"]))),
    operation_id = "create_ledger_payment",
    security(("bearer" = [])),
    request_body = PaymentRequest,
    responses(
        (status = 200, description = "The payment was created before with this `reference` and content.", body = Data<Payment>),
        (status = 201, description = "The payment.", body = Data<Payment>, headers(("Location" = String, description = "Path of the payment."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.payments_create` (`permission_denied`) or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 422, description = "A value is invalid, the partner is not set up (`ledger.gateway_not_configured`), refused (`payment.provider_rejected`), or the reference was used for other content (`reference_reused`).", body = Problem),
        (status = 503, description = "The partner did not answer; send the same request again (`payment.provider_unavailable`).", body = Problem),
    )
)]
async fn create_payment(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<PaymentRequest>,
) -> Result<Response, Error> {
    let org = input.org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
    let mut payment = NewPayment::new(
        org,
        input.method,
        input.amount,
        input.description,
        "ledger",
        input.reference,
    );
    payment.provider = input.provider;
    payment.return_url = input.return_url;
    payment.source_id = input.source_id;
    let outcome = service.create_payment_manual(&tenant, &principal, payment).await?;
    Ok(submitted(outcome, |payment| format!("/ledger/payments/{}", payment.id)))
}

/// List payments in your scope.
///
/// Requires `ledger.read`.
#[utoipa::path(
    get,
    path = "/payments",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "list_ledger_payments",
    security(("bearer" = [])),
    params(PaymentsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of payments.", body = Page<Payment>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_payments(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PaymentsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<Payment>, Error> {
    let mut problems = Vec::new();
    let amount_minor = query.amount.as_deref().map(str::trim).and_then(|raw| {
        raw.parse::<i64>().ok().or_else(|| {
            problems.push(FieldError::new(
                "amount",
                "invalid",
                "must be a whole number in the smallest unit of the currency, e.g. `150000`",
            ));
            None
        })
    });
    let currency = query.currency.as_deref().map(str::trim).and_then(|raw| {
        Currency::parse(raw).ok().map(|currency| currency.code().to_owned()).or_else(|| {
            problems.push(FieldError::new(
                "currency",
                "unknown",
                "must be an ISO 4217 currency code, e.g. `VND`",
            ));
            None
        })
    });
    let page = PageRequest::try_from(page);
    if let Err(error) = &page {
        problems.extend(error.fields().iter().cloned());
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let filter = PaymentFilter {
        status: query.status,
        source_module: query.source_module,
        source_id: query.source_id,
        amount_minor,
        currency,
        org_id: query.org_id,
    };
    let page = service.list_payments(&tenant, &principal, filter, &page?).await?;
    let Page { data, pagination } = page;
    let data = data.iter().map(describe_payment).collect::<Result<Vec<_>, _>>()?;
    Ok(Page { data, pagination })
}

/// Get a payment with its refunds.
///
/// Requires `ledger.read` on the unit that receives it.
#[utoipa::path(
    get,
    path = "/payments/{id}",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "get_ledger_payment",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the payment.")),
    responses(
        (status = 200, description = "The payment.", body = Data<PaymentDetail>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such payment in your scope; code `ledger.payment_not_found`.", body = Problem),
    )
)]
async fn get_payment(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PaymentDetail>, Error> {
    let row = service.payment_for(&tenant, &principal, &READ, id).await?;
    let refunds = service.refunds_of(&tenant, row.id).await?;
    Ok(Data {
        data: PaymentDetail {
            payment: describe_payment(&row)?,
            refunds: refunds.iter().map(describe_refund).collect::<Result<Vec<_>, _>>()?,
        },
    })
}

/// A refund to make.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerRefundRequest)]
pub(crate) struct RefundRequest {
    /// At most what is left of the payment.
    amount: Money,
    /// Short reason, 1 to 200 characters, no personal data.
    reason: String,
    /// Your id of this refund: sending it again returns the same refund.
    reference: String,
}

/// Refund (part of) a payment.
///
/// Requires `ledger.refunds_create` on the unit that received it. The partner pays the money back;
/// the book gets the entry when the partner confirms. Refunds never add up to more than the
/// payment, even when sent at the same time.
#[utoipa::path(
    post,
    path = "/payments/{id}/refunds",
    extensions(("x-permissions" = json!(["ledger.refunds_create"]))),
    operation_id = "create_ledger_refund",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the payment.")),
    request_body = RefundRequest,
    responses(
        (status = 200, description = "The refund was made before with this `reference`.", body = Data<Refund>),
        (status = 201, description = "The refund.", body = Data<Refund>, headers(("Location" = String, description = "Path of the payment."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.refunds_create`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such payment in your scope; code `ledger.payment_not_found`.", body = Problem),
        (status = 409, description = "The payment is not collected (`ledger.payment_state`).", body = Problem),
        (status = 422, description = "More than what is left (`ledger.refund_exceeds_payment`), the partner cannot refund (`payment.not_supported`) or a value is invalid.", body = Problem),
        (status = 503, description = "The partner did not answer; send the same request again (`payment.provider_unavailable`).", body = Problem),
    )
)]
async fn create_refund(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<RefundRequest>,
) -> Result<Response, Error> {
    let refund = NewRefund::new(id, input.amount, input.reason, input.reference);
    let outcome = service.refund_manual(&tenant, &principal, refund).await?;
    Ok(submitted(outcome, |refund| format!("/ledger/payments/{}", refund.payment_id)))
}

// ---- Webhook ----------------------------------------------------------------------------------------

/// Receipt of a partner notification.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerWebhookReceipt)]
pub(crate) struct WebhookReceipt {
    received: bool,
}

/// Receive a notification from a payment partner.
///
/// No sign-in: the partner signs every notification and the signature is checked on the raw body
/// before anything is stored. A notification sent again is stored once; it is processed in the
/// background. Point the partner's webhook setting at this URL.
#[utoipa::path(
    post,
    path = "/webhooks/{provider}",
    extensions(("x-public" = json!(true))),
    operation_id = "receive_ledger_webhook",
    params(("provider" = String, Path, description = "Partner name, e.g. `vnpay`.")),
    request_body(content = String, description = "The partner's notification, as the partner sends it.", content_type = "application/json"),
    responses(
        (status = 200, description = "Stored (or stored before).", body = Data<WebhookReceipt>),
        (status = 401, description = "The signature is not valid; code `payment.signature_invalid`.", body = Problem),
        (status = 413, description = "The body is larger than `AUTOXIDE_LEDGER_WEBHOOK_MAX_BYTES`.", body = Problem),
        (status = 422, description = "The partner is not set up on this server; code `ledger.gateway_not_configured`.", body = Problem),
    )
)]
async fn receive_webhook(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    Path(provider): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Data<WebhookReceipt>, Error> {
    service.receive_webhook(&tenant, &provider, &headers, &body).await?;
    Ok(Data { data: WebhookReceipt { received: true } })
}

/// Receive a notification from a payment partner at a tenant's own address.
///
/// No sign-in. The last part of the address identifies the tenant (copy the address from
/// `GET /api/v1/kernel/connections/webhooks`); the notification is then checked with that tenant's
/// secret at the partner before anything is stored. A notification sent again is stored once; it
/// is processed in the background. The answer is the one the partner expects (its documented
/// acknowledgement), or the standard envelope when the partner expects nothing special.
#[utoipa::path(
    post,
    path = "/webhooks/{provider}/{hook}",
    extensions(("x-public" = json!(true))),
    operation_id = "receive_ledger_tenant_webhook",
    params(
        ("provider" = String, Path, description = "Partner name, e.g. `momo`."),
        ("hook" = String, Path, description = "The tenant's webhook identifier."),
    ),
    request_body(content = String, description = "The partner's notification, as the partner sends it.", content_type = "application/json"),
    responses(
        (status = 200, description = "Stored (or stored before). The body is the partner's expected acknowledgement when it has one.", body = Data<WebhookReceipt>),
        (status = 204, description = "Stored (or stored before), for partners that expect an empty answer."),
        (status = 401, description = "The signature is not valid; code `payment.signature_invalid`.", body = Problem),
        (status = 404, description = "The address is not known; code `provider.webhook_unknown`.", body = Problem),
        (status = 413, description = "The body is larger than `AUTOXIDE_LEDGER_WEBHOOK_MAX_BYTES`.", body = Problem),
        (status = 422, description = "The partner is not installed on this server (`ledger.gateway_not_configured`) or the tenant has not set its account (`provider.not_configured`).", body = Problem),
    )
)]
async fn receive_tenant_webhook(
    State(service): State<Arc<LedgerService>>,
    Path((provider, hook)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Error> {
    let tenant = service.webhook_tenant("webhooks", &provider, &hook).await?;
    service.receive_webhook(&tenant, &provider, &headers, &body).await?;
    Ok(match service.gateway(Some(&provider))?.webhook_ack() {
        Some(ack) => ack.into_response(),
        None => Data { data: WebhookReceipt { received: true } }.into_response(),
    })
}

// ---- Hàng chờ, đối soát ------------------------------------------------------------------------------

/// A difference waiting for a person.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerExceptionResponse)]
pub(crate) struct ExceptionResponse {
    id: Uuid,
    /// `amount_mismatch`, `unmatched_credit`, `duplicate_payment`, `missing_in_statement`,
    /// `unknown_in_statement` or `late_payment`.
    kind: String,
    /// The record concerned, e.g. `ledger.payment:<id>`.
    subject: String,
    #[schema(value_type = Object)]
    details: serde_json::Value,
    /// `open`, `resolved` or `ignored`.
    status: String,
    resolution: Option<String>,
    note: Option<String>,
    resolved_by: Option<Uuid>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<exception::Model> for ExceptionResponse {
    fn from(row: exception::Model) -> Self {
        ExceptionResponse {
            id: row.id,
            kind: row.kind,
            subject: row.subject,
            details: serde_json::from_str(&row.details).unwrap_or_default(),
            status: row.status,
            resolution: row.resolution,
            note: row.note,
            resolved_by: row.resolved_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Which items.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ExceptionsQuery {
    /// `open`, `resolved` or `ignored`.
    status: Option<String>,
}

/// List the differences waiting for a person.
///
/// Requires `ledger.read` covering the whole tenant.
#[utoipa::path(
    get,
    path = "/exceptions",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "list_ledger_exceptions",
    security(("bearer" = [])),
    params(ExceptionsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of items.", body = Page<ExceptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read` (`permission_denied`) or not for the whole tenant (`org_out_of_scope`).", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_exceptions(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ExceptionsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<ExceptionResponse>, Error> {
    let page = service
        .list_exceptions(&tenant, &principal, query.status, &PageRequest::try_from(page)?)
        .await?;
    Ok(page.map(ExceptionResponse::from))
}

/// How to resolve an item.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerResolveRequest)]
pub(crate) struct ResolveRequest {
    /// `match` (record the money for `payment_id` when the partner collected exactly the amount
    /// due), `accept_received` (record the amount the partner collected for `payment_id`, even
    /// when it differs from the amount due), `ignore`, or `refunded_outside`.
    action: String,
    /// With `match` and `accept_received`: the payment the item is about (the one it names, if
    /// any), still waiting for its money and in the same currency.
    payment_id: Option<Uuid>,
    /// Up to 500 characters.
    note: Option<String>,
}

/// Resolve a difference.
///
/// Requires `ledger.reconcile`. `match` records the money the partner collected for the payment
/// (same path as a partner notification) and is refused when that amount differs from the amount
/// due; `accept_received` records the amount collected anyway, after a person checked it.
/// Resolving a resolved item returns it unchanged.
#[utoipa::path(
    post,
    path = "/exceptions/{id}/resolve",
    extensions(("x-permissions" = json!(["ledger.reconcile"]))),
    operation_id = "resolve_ledger_exception",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the item.")),
    request_body = ResolveRequest,
    responses(
        (status = 200, description = "The item after the change.", body = Data<ExceptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.reconcile`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such item (`ledger.exception_not_found`) or payment (`ledger.payment_not_found`).", body = Problem),
        (status = 409, description = "The amount collected differs from the amount due and the action is `match` (`ledger.amount_differs`), or the payment is no longer waiting for its money (`ledger.payment_state`).", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn resolve_exception(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ResolveRequest>,
) -> Result<Data<ExceptionResponse>, Error> {
    principal.require(&crate::permissions::RECONCILE)?;
    let resolution = match (input.action.as_str(), input.payment_id) {
        ("match", Some(payment)) => Resolution::Match(payment),
        ("accept_received", Some(payment)) => Resolution::AcceptReceived(payment),
        ("ignore", _) => Resolution::Ignore,
        ("refunded_outside", _) => Resolution::RefundedOutside,
        ("match" | "accept_received", None) => {
            return Err(Error::validation(vec![FieldError::new(
                "payment_id",
                "required",
                "is required with `match` and `accept_received`",
            )]));
        }
        _ => {
            return Err(Error::validation(vec![FieldError::new(
                "action",
                "invalid",
                "must be `match`, `accept_received`, `ignore` or `refunded_outside`",
            )]));
        }
    };
    let row = service.resolve_exception(&tenant, &principal, id, resolution, input.note).await?;
    Ok(Data { data: row.into() })
}

/// A reconciliation run.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerReconciliationResponse)]
pub(crate) struct ReconciliationResponse {
    id: Uuid,
    provider: String,
    /// The day reconciled.
    date: Date,
    /// `completed` or `failed`.
    status: String,
    /// Statement lines read.
    lines: i64,
    /// Lines that matched the book.
    matched: i64,
    /// Differences sent to the exceptions.
    exceptions: i64,
    created_by: Option<Uuid>,
    created_at: Timestamp,
}

impl From<reconciliation::Model> for ReconciliationResponse {
    fn from(row: reconciliation::Model) -> Self {
        ReconciliationResponse {
            id: row.id,
            provider: row.provider,
            date: row.on_date,
            status: row.status,
            lines: row.lines,
            matched: row.matched,
            exceptions: row.exceptions,
            created_by: row.created_by,
            created_at: row.created_at,
        }
    }
}

/// A reconciliation to run.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerReconciliationRequest)]
pub(crate) struct ReconciliationRequest {
    /// Partner; may be left out when the server has only one.
    provider: Option<String>,
    /// Day to reconcile, `YYYY-MM-DD`.
    date: Date,
}

/// List reconciliation runs.
///
/// Requires `ledger.reconcile`.
#[utoipa::path(
    get,
    path = "/reconciliations",
    extensions(("x-permissions" = json!(["ledger.reconcile"]))),
    operation_id = "list_ledger_reconciliations",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of runs.", body = Page<ReconciliationResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.reconcile`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_reconciliations(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<ReconciliationResponse>, Error> {
    let page =
        service.list_reconciliations(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(ReconciliationResponse::from))
}

/// Reconcile a day now.
///
/// Requires `ledger.reconcile`. Reads the partner statement for the day and matches it with the
/// book; differences go to `GET /exceptions`. A day is also reconciled every night. Running a day
/// again adds no duplicate lines or items.
#[utoipa::path(
    post,
    path = "/reconciliations",
    extensions(("x-permissions" = json!(["ledger.reconcile"]))),
    operation_id = "run_ledger_reconciliation",
    security(("bearer" = [])),
    request_body = ReconciliationRequest,
    responses(
        (status = 201, description = "The run.", body = Data<ReconciliationResponse>, headers(("Location" = String, description = "Path of the runs."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.reconcile`; code `permission_denied`.", body = Problem),
        (status = 422, description = "The partner is not set up or has no statements; every problem in `errors`.", body = Problem),
        (status = 503, description = "The partner did not answer (`payment.provider_unavailable`).", body = Problem),
    )
)]
async fn run_reconciliation(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ReconciliationRequest>,
) -> Result<Created<ReconciliationResponse>, Error> {
    let row = service.reconcile_manual(&tenant, &principal, input.provider, input.date).await?;
    Ok(Created::new("/ledger/reconciliations".to_owned(), row.into()))
}
