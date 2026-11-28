//! Tool MCP của dòng tiền: cùng service, cùng permission với REST (luật 8). Đầu vào phẳng (client
//! MCP không đọc `$ref`): số tiền là chữ thập phân theo đơn vị chính kèm `currency`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::decimal::Rounding;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::api::{FlowSettings, Hold, NewPayout, NewTransferPayment, Payment, Payout};
use crate::flows::{HoldFilter, describe_hold};
use crate::payouts::{PayoutFilter, describe_payout};
use crate::permissions::{HOLDS_RELEASE, PAYMENTS_CREATE, PAYOUTS_CREATE, READ};
use crate::service::LedgerService;

fn amount_of(amount: &str, currency: &str) -> Result<Money, Error> {
    let currency = Currency::parse(currency)?;
    Money::from_decimal(autoxide_kernel::decimal::parse(amount)?, currency, Rounding::HalfUp)
}

/// Which unit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FlowSettingsInput {
    /// Id of the unit; default the tenant's root.
    org_id: Option<Uuid>,
}

/// Which held amounts.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct HoldsInput {
    /// `held` or `released`.
    status: Option<String>,
    /// Only for this seller (unit id).
    seller_org_id: Option<Uuid>,
    /// Only for documents of this module, e.g. `orders`.
    source_module: Option<String>,
    /// Only for this document.
    source_id: Option<String>,
    /// Maximum number of items, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// Which hold.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReleaseInput {
    /// Id of the hold.
    hold_id: Uuid,
}

/// Which payouts.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PayoutsInput {
    /// `pending`, `sent`, `succeeded` or `failed`.
    status: Option<String>,
    /// Only payouts to this seller.
    payee_org_id: Option<Uuid>,
    /// Maximum number of items, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// A payout to make.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PayoutInput {
    /// The seller (unit) to pay.
    payee_org_id: Uuid,
    /// Amount as a decimal string, e.g. `"935000"`; at most the available balance.
    amount: String,
    /// Currency code, e.g. `VND`.
    currency: String,
    /// Short text, 1 to 140 characters, no personal data.
    description: String,
    /// Your id of this payout: sending it again returns the same payout.
    reference: String,
}

/// A VietQR payment to create.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct VietqrInput {
    /// The unit that receives the money; default the tenant's root.
    org_id: Option<Uuid>,
    /// Receiving bank account; may be left out when the unit has only one.
    bank_account_id: Option<Uuid>,
    /// Amount in VND as a decimal string, e.g. `"150000"`.
    amount: String,
    /// Short text, 1 to 140 characters, no personal data.
    description: String,
    /// Id of your document.
    source_id: Option<String>,
    /// Your id of this payment: sending it again returns the same payment.
    reference: String,
}

pub(crate) fn all(service: &Arc<LedgerService>) -> Vec<Tool> {
    let (settings, holds, release) = (service.clone(), service.clone(), service.clone());
    let (payouts, payout, vietqr) = (service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "flow_settings",
            "The money-flow mode in force for a unit (self collect, partner collect, split at the \
             gateway), its partners, hold days and where cash on delivery goes.",
            move |tenant: TenantContext, principal: Principal, input: FlowSettingsInput| {
                let service = settings.clone();
                async move {
                    let org = input.org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
                    let found: FlowSettings = service.flow_settings_for(&tenant, &principal, org).await?;
                    Ok::<_, Error>(found)
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "holds",
            "Seller money held until the return window ends, with what is still held; follow \
             `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: HoldsInput| {
                let service = holds.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).with_count(input.count.unwrap_or(false)).page()?;
                    let filter = HoldFilter {
                        status: input.status,
                        seller_org_id: input.seller_org_id,
                        source_module: input.source_module,
                        source_id: input.source_id,
                    };
                    let found = service.list_holds(&tenant, &principal, filter, &page).await?;
                    let Page { data, pagination } = found;
                    let data = data.iter().map(describe_hold).collect::<Result<Vec<Hold>, _>>()?;
                    Ok::<_, Error>(Page { data, pagination })
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "release_hold",
            "Release held seller money now: what is still held moves to the seller's available \
             balance. Releasing again changes nothing.",
            move |tenant: TenantContext, principal: Principal, input: ReleaseInput| {
                let service = release.clone();
                async move {
                    let found: Hold =
                        service.release_hold_manual(&tenant, &principal, input.hold_id).await?;
                    Ok::<_, Error>(found)
                }
            },
        )
        .requires(HOLDS_RELEASE),
        Tool::new(
            "payouts",
            "Payouts of seller balances through the payout partner, with their status; follow \
             `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: PayoutsInput| {
                let service = payouts.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).with_count(input.count.unwrap_or(false)).page()?;
                    let filter =
                        PayoutFilter { status: input.status, payee_org_id: input.payee_org_id };
                    let found = service.list_payouts(&tenant, &principal, filter, &page).await?;
                    let Page { data, pagination } = found;
                    let data =
                        data.iter().map(describe_payout).collect::<Result<Vec<Payout>, _>>()?;
                    Ok::<_, Error>(Page { data, pagination })
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "create_payout",
            "Pay a seller (in the partner collect mode) from its available balance through the \
             payout partner. Never more than the balance; a refused payout puts the money back. The \
             same reference and content again returns the payout already made.",
            move |tenant: TenantContext, principal: Principal, input: PayoutInput| {
                let service = payout.clone();
                async move {
                    let amount = amount_of(&input.amount, &input.currency)?;
                    let request = NewPayout::new(
                        OrgId::from_uuid(input.payee_org_id),
                        amount,
                        input.description,
                        input.reference,
                    );
                    let found: Payout = service
                        .create_payout_manual(&tenant, &principal, request)
                        .await?
                        .into_inner();
                    Ok::<_, Error>(found)
                }
            },
        )
        .requires(PAYOUTS_CREATE)
        .requires_approval_for_agent(),
        Tool::new(
            "create_vietqr_payment",
            "Create a VietQR bank transfer payment into the unit's own bank account: returns the QR \
             payload and the transfer code the buyer keeps in the description. Matched \
             automatically when the bank reports the transfer.",
            move |tenant: TenantContext, principal: Principal, input: VietqrInput| {
                let service = vietqr.clone();
                async move {
                    let amount = amount_of(&input.amount, "VND")?;
                    let org = input.org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
                    let mut request = NewTransferPayment::new(
                        org,
                        amount,
                        input.description,
                        "ledger",
                        input.reference,
                    );
                    request.bank_account = input.bank_account_id;
                    request.source_id = input.source_id;
                    let found: Payment = service
                        .create_transfer_payment_manual(&tenant, &principal, request)
                        .await?
                        .into_inner();
                    Ok::<_, Error>(found)
                }
            },
        )
        .requires(PAYMENTS_CREATE),
    ]
}
