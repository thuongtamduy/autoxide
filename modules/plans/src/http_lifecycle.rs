//! Route REST của vòng đời thuê bao (bước 35 phần E / F): khách đổi, huỷ, tiếp tục; nhà vận hành
//! huỷ ngay, khoá hẳn khách, thu tay kỳ thanh toán.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::entity::billing_period;
use crate::http::{AddonItem, SubscriptionResponse, operator};
use crate::lifecycle::ChangeInput;
use crate::permissions::{BILLING_MANAGE, BILLING_READ, SUBSCRIPTIONS_MANAGE, SUBSCRIPTIONS_READ};
use crate::service::{PlansService, subscription_not_found};

pub(crate) fn routes() -> OpenApiRouter<Arc<PlansService>> {
    OpenApiRouter::new()
        .routes(routes!(change_subscription))
        .routes(routes!(cancel_subscription))
        .routes(routes!(resume_subscription))
        .routes(routes!(cancel_tenant_subscription))
        .routes(routes!(suspend_tenant))
        .routes(routes!(list_billing_periods))
        .routes(routes!(get_billing_period))
        .routes(routes!(tenant_billing_periods))
        .routes(routes!(mark_paid))
        .routes(routes!(void_period))
        .routes(routes!(pay_period))
}

/// A billing period of a subscription: what is charged for which time, and whether it is paid.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct BillingPeriodResponse {
    id: Uuid,
    tenant_id: Uuid,
    /// `initial` (first period), `renewal`, `proration` (part of a period after an upgrade) or
    /// `metered`.
    kind: String,
    plan_id: Uuid,
    price_id: Option<Uuid>,
    period_start: Timestamp,
    period_end: Timestamp,
    amount: Money,
    /// Unique per tenant: `plans:<subscription_id>:<period start>:<kind>`.
    reference: String,
    /// `open`, `paid` or `void`.
    status: String,
    due_at: Timestamp,
    paid_at: Option<Timestamp>,
    /// The payment provider's reference (or the bank transfer's), when paid.
    provider_reference: Option<String>,
    /// The operator's invoice for the period (`GET /api/v1/invoices/documents/{id}` for the
    /// operator), and its series and number.
    invoice_id: Option<Uuid>,
    invoice_number: Option<String>,
    created_at: Timestamp,
}

impl From<billing_period::Model> for BillingPeriodResponse {
    fn from(row: billing_period::Model) -> Self {
        let amount = Money::from_columns(row.amount_minor, &row.amount_currency)
            .unwrap_or(Money::zero(autoxide_kernel::money::Currency::VND));
        BillingPeriodResponse {
            id: row.id,
            tenant_id: row.tenant_id,
            kind: row.kind,
            plan_id: row.plan_id,
            price_id: row.price_id,
            period_start: row.period_start,
            period_end: row.period_end,
            amount,
            reference: row.reference,
            status: row.status,
            due_at: row.due_at,
            paid_at: row.paid_at,
            provider_reference: row.provider_reference,
            invoice_id: row.invoice_id,
            invoice_number: row.invoice_number,
            created_at: row.created_at,
        }
    }
}

/// Move your tenant to another plan.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChangeRequest {
    /// An active base plan.
    plan_id: Uuid,
    /// An active recurring price of that plan.
    price_id: Option<Uuid>,
    /// Replace the add-ons with these; leave out to keep them.
    addons: Option<Vec<AddonItem>>,
    /// Where billing reminders go.
    #[validate(email)]
    billing_email: Option<String>,
    /// Your reference for this change (1 to 200 characters), kept in the audit trail; sending the
    /// same change again changes nothing more.
    reference: String,
}

const fn at_end() -> bool {
    true
}

/// Cancel your tenant's subscription.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CancelRequest {
    /// `true` (default): cancel when the current period ends, and keep using it until then;
    /// `false`: cancel now (data becomes read-only and is kept for the plan's retention days).
    #[serde(default = "at_end")]
    at_period_end: bool,
}

/// Suspend a tenant completely.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SuspendRequest {
    /// Why, 1 to 500 characters (kept in the audit trail).
    #[validate(length(min = 1, max = 500))]
    reason: String,
}

/// Record a payment made outside the system (for example a bank transfer checked by hand).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MarkPaidRequest {
    /// The bank's or provider's reference of the payment, 1 to 200 characters.
    #[validate(length(min = 1, max = 200))]
    provider_reference: String,
    /// When it was paid; default now.
    paid_at: Option<Timestamp>,
}

async fn own_subscription(
    service: &PlansService,
    tenant: TenantId,
) -> Result<Data<SubscriptionResponse>, Error> {
    let row = service.subscription(tenant).await?.ok_or_else(subscription_not_found)?;
    Ok(Data { data: row.into() })
}

/// Move your tenant to another plan.
///
/// Requires `plans.billing.manage`. A plan that costs at least as much per month applies at once
/// (a `proration` billing period charges the rest of the current period); a cheaper one applies
/// when the current period ends (`scheduled_plan_id`). Usage above the new limits removes nothing,
/// it only stops new use.
#[utoipa::path(
    post,
    path = "/subscription/change",
    extensions(("x-permissions" = json!(["plans.billing.manage"]))),
    operation_id = "change_own_subscription",
    security(("bearer" = [])),
    request_body = ChangeRequest,
    responses(
        (status = 200, description = "The subscription after the change.", body = Data<SubscriptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `plans.billing.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No subscription (`plans.subscription_not_found`), plan (`plans.plan_not_found`) or price (`plans.price_not_found`).", body = Problem),
        (status = 409, description = "The plan is not sold (`plans.plan_not_active`), or the subscription is not trialing or active (`plans.invalid_transition`), or it is paid through an app store (`plans.store_billed`): change it in the store.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or an add-on is of the wrong kind (`plans.addon_not_allowed`).", body = Problem),
    )
)]
async fn change_subscription(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ChangeRequest>,
) -> Result<Data<SubscriptionResponse>, Error> {
    principal.require(&BILLING_MANAGE)?;
    service.ensure_not_store_billed(tenant.id()).await?;
    let reference = input.reference.trim();
    if reference.is_empty() || reference.len() > 200 {
        return Err(Error::validation(vec![FieldError::new(
            "reference",
            "length",
            "must be 1 to 200 characters long",
        )]));
    }
    let change = ChangeInput {
        plan_id: input.plan_id,
        price_id: input.price_id,
        addons: input
            .addons
            .map(|items| items.into_iter().map(|item| (item.plan_id, item.quantity)).collect()),
        billing_email: input.billing_email,
    };
    service.change(Actor::from(&principal), tenant.id(), change).await?;
    own_subscription(&service, tenant.id()).await
}

/// Cancel your tenant's subscription.
///
/// Requires `plans.billing.manage`. By default at the end of the current period (undo with
/// `POST /subscription/resume`).
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    post,
    path = "/subscription/cancel",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["plans.billing.manage"]))),
    operation_id = "cancel_own_subscription",
    security(("bearer" = [])),
    request_body = CancelRequest,
    responses(
        (status = 200, description = "The subscription after the change.", body = Data<SubscriptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `plans.billing.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No subscription; code `plans.subscription_not_found`.", body = Problem),
        (status = 409, description = "Already canceled (`plans.invalid_transition`), or paid through an app store (`plans.store_billed`): cancel it in the store.", body = Problem),
    )
)]
async fn cancel_subscription(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CancelRequest>,
) -> Result<Data<SubscriptionResponse>, Error> {
    principal.require(&BILLING_MANAGE)?;
    service.ensure_not_store_billed(tenant.id()).await?;
    service.cancel(Actor::from(&principal), tenant.id(), input.at_period_end).await?;
    own_subscription(&service, tenant.id()).await
}

/// Resume your tenant's subscription.
///
/// Requires `plans.billing.manage`. Undoes a cancellation at the end of the period; a canceled
/// subscription whose data is still kept starts again (its first period must be paid).
#[utoipa::path(
    post,
    path = "/subscription/resume",
    extensions(("x-permissions" = json!(["plans.billing.manage"]))),
    operation_id = "resume_own_subscription",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The subscription after the change.", body = Data<SubscriptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `plans.billing.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No subscription; code `plans.subscription_not_found`.", body = Problem),
        (status = 409, description = "Nothing to resume (`plans.invalid_transition`), or paid through an app store (`plans.store_billed`).", body = Problem),
    )
)]
async fn resume_subscription(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<SubscriptionResponse>, Error> {
    principal.require(&BILLING_MANAGE)?;
    service.ensure_not_store_billed(tenant.id()).await?;
    service.resume(tenant.id()).await?;
    own_subscription(&service, tenant.id()).await
}

/// Cancel a tenant's subscription now.
///
/// Platform staff with `plans.subscriptions.manage`. The tenant becomes read-only; its data is kept
/// for the plan's retention days.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    post,
    path = "/tenants/{tenant_id}/subscription/cancel",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["plans.subscriptions.manage"]))),
    operation_id = "cancel_tenant_subscription",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant.")),
    responses(
        (status = 200, description = "The subscription, now `canceled`.", body = Data<SubscriptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No subscription; code `plans.subscription_not_found`.", body = Problem),
        (status = 409, description = "Already canceled; code `plans.invalid_transition`.", body = Problem),
    )
)]
async fn cancel_tenant_subscription(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
) -> Result<Data<SubscriptionResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_MANAGE)?;
    let tenant = TenantId::from_uuid(tenant_id);
    service.cancel(Actor::from(&principal), tenant, false).await?;
    own_subscription(&service, tenant).await
}

/// Suspend a tenant completely.
///
/// Platform staff with `plans.subscriptions.manage`. Unlike a suspended subscription (read-only),
/// every request of the tenant is refused (`tenant_suspended`), reads included. Reactivate with
/// `PATCH /api/v1/kernel/tenants/{id}` (`status: active`).
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    post,
    path = "/tenants/{tenant_id}/subscription/suspend",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["plans.subscriptions.manage"]))),
    operation_id = "suspend_tenant",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant.")),
    request_body = SuspendRequest,
    responses(
        (status = 204, description = "Suspended."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
        (status = 422, description = "`reason` is missing or too long, or the tenant is the platform.", body = Problem),
    )
)]
async fn suspend_tenant(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
    ValidJson(input): ValidJson<SuspendRequest>,
) -> Result<axum::http::StatusCode, Error> {
    operator(&principal, &SUBSCRIPTIONS_MANAGE)?;
    let tenant = TenantId::from_uuid(tenant_id);
    if tenant == autoxide_kernel::tenancy::DEFAULT_TENANT {
        return Err(Error::validation(vec![FieldError::new(
            "tenant_id",
            "not_allowed",
            "the platform tenant cannot be suspended",
        )]));
    }
    tracing::info!(tenant_id = %tenant, reason = %input.reason, "tenant suspended by platform staff");
    service.suspend_tenant(Actor::from(&principal), tenant, &input.reason).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// List your tenant's billing periods.
///
/// Requires `plans.billing.read`. Newest first unless `order=asc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/billing-periods",
    extensions(("x-permissions" = json!(["plans.billing.read"]))),
    operation_id = "list_billing_periods",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of the billing periods, newest first.", body = Page<BillingPeriodResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `plans.billing.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_billing_periods(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<BillingPeriodResponse>, Error> {
    principal.require(&BILLING_READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.billing_periods(tenant.id(), &page).await?;
    Ok(rows.map(BillingPeriodResponse::from))
}

/// List a tenant's billing periods.
///
/// Platform staff with `plans.subscriptions.read`. Newest first unless `order=asc`; paged with
/// `limit` / `cursor`. Record a payment checked by hand with
/// `POST /billing-periods/{id}/mark-paid`.
#[utoipa::path(
    get,
    path = "/tenants/{tenant_id}/billing-periods",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "list_tenant_billing_periods",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant."), PageQuery),
    responses(
        (status = 200, description = "One page of the billing periods, newest first.", body = Page<BillingPeriodResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn tenant_billing_periods(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<BillingPeriodResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.billing_periods(TenantId::from_uuid(tenant_id), &page).await?;
    Ok(rows.map(BillingPeriodResponse::from))
}

/// Get one of your tenant's billing periods.
///
/// Requires `plans.billing.read`.
#[utoipa::path(
    get,
    path = "/billing-periods/{id}",
    extensions(("x-permissions" = json!(["plans.billing.read"]))),
    operation_id = "get_billing_period",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the billing period.")),
    responses(
        (status = 200, description = "The billing period.", body = Data<BillingPeriodResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `plans.billing.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such billing period of your tenant; code `plans.billing_period_not_found`.", body = Problem),
    )
)]
async fn get_billing_period(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BillingPeriodResponse>, Error> {
    principal.require(&BILLING_READ)?;
    Ok(Data { data: service.billing_period(Some(tenant.id()), id).await?.into() })
}

/// Record that a billing period was paid.
///
/// Platform staff with `plans.subscriptions.manage`, for payments checked by hand (no payment
/// provider yet). A past-due or suspended subscription becomes active again.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    post,
    path = "/billing-periods/{id}/mark-paid",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["plans.subscriptions.manage"]))),
    operation_id = "mark_billing_period_paid",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the billing period.")),
    request_body = MarkPaidRequest,
    responses(
        (status = 200, description = "The billing period, now `paid`.", body = Data<BillingPeriodResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such billing period; code `plans.billing_period_not_found`.", body = Problem),
        (status = 409, description = "Already paid or void; code `plans.billing_period_not_payable`.", body = Problem),
    )
)]
async fn mark_paid(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<MarkPaidRequest>,
) -> Result<Data<BillingPeriodResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_MANAGE)?;
    let row = service
        .mark_paid(Actor::from(&principal), id, input.provider_reference, input.paid_at, None)
        .await?;
    Ok(Data { data: row.into() })
}

/// Void an unpaid billing period.
///
/// Platform staff with `plans.subscriptions.manage`. When the period already has an invoice, it is cancelled
/// afterwards by a background job, not within this request: an e-invoice the tax rules no longer let cancel
/// (Vietnam since 2025-06-01) gets a full decrease adjustment instead. Read the invoice later to see the result.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    post,
    path = "/billing-periods/{id}/void",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["plans.subscriptions.manage"]))),
    operation_id = "void_billing_period",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the billing period.")),
    responses(
        (status = 200, description = "The billing period, now `void`; its invoice is cancelled or adjusted asynchronously.", body = Data<BillingPeriodResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such billing period; code `plans.billing_period_not_found`.", body = Problem),
        (status = 409, description = "Already paid or void; code `plans.billing_period_not_payable`.", body = Problem),
    )
)]
async fn void_period(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BillingPeriodResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_MANAGE)?;
    Ok(Data { data: service.void_period(Actor::from(&principal), id).await?.into() })
}

/// How to pay a billing period.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PayRequest {
    /// `card`, `wallet`, `vietqr`… — one the operator's payment partner supports; default `card`.
    method: Option<String>,
    /// Page to come back to after paying.
    return_url: Option<String>,
}

/// Pay a billing period of your tenant online.
///
/// Requires `plans.billing.manage` and a billing profile. Starts a payment with the operator's
/// payment partner and returns its checkout page; the period is marked paid when the partner reports
/// the payment. Calling again returns the payment still waiting, or starts a new one when the last
/// one expired.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    post,
    path = "/billing-periods/{id}/pay",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["plans.billing.manage"]))),
    operation_id = "pay_billing_period",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the billing period.")),
    request_body = PayRequest,
    responses(
        (status = 200, description = "The payment to complete.", body = Data<crate::billing::PayResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `plans.billing.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such billing period of your tenant; code `plans.billing_period_not_found`.", body = Problem),
        (status = 409, description = "Already paid or void (`plans.billing_period_not_payable`), or no billing profile (`plans.billing_profile_required`).", body = Problem),
        (status = 422, description = "The method is not supported (`validation_failed`) or the partner refused (`payment.provider_rejected`).", body = Problem),
        (status = 503, description = "Online payment is not set up (`plans.payments_unavailable`) or the partner did not answer (`payment.provider_unavailable`).", body = Problem),
    )
)]
async fn pay_period(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<PayRequest>,
) -> Result<Data<crate::billing::PayResponse>, Error> {
    Ok(Data { data: service.pay(&tenant, &principal, id, input.method, input.return_url).await? })
}
