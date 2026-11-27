//! Route REST của dòng tiền (dưới `/api/v1/ledger`): cài đặt dòng tiền, khoản tạm giữ, chi trả, tài
//! khoản ngân hàng, thanh toán VietQR, webhook biến động số dư, trạng thái niêm phong của sổ.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
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

use crate::api::{FlowSettings, Hold, NewPayout, NewTransferPayment, Payment, Payout};
use crate::bank::{BankAccountChange, NewBankAccount};
use crate::entity::bank_account;
use crate::flows::{FlowSettingsInput, HoldFilter, describe_hold};
use crate::http::present;
use crate::payouts::{PayoutFilter, describe_payout};
use crate::service::LedgerService;

pub(crate) fn routes(service: Arc<LedgerService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(get_flow_settings, put_flow_settings))
        .routes(routes!(list_holds))
        .routes(routes!(release_hold))
        .routes(routes!(list_payouts, create_payout))
        .routes(routes!(get_payout))
        .routes(routes!(list_bank_accounts, create_bank_account))
        .routes(routes!(update_bank_account))
        .routes(routes!(preview_vietqr))
        .routes(routes!(create_vietqr_payment))
        .routes(routes!(receive_bank_feed))
        .routes(routes!(receive_tenant_bank_feed))
        .routes(routes!(seal_status))
        .routes(routes!(verify_book))
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

// ---- Cài đặt dòng tiền ------------------------------------------------------------------------

/// Money-flow settings to set on a unit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerFlowSettingsRequest)]
pub(crate) struct FlowSettingsRequest {
    /// `self_collect`, `partner_collect` or `split_at_gateway`.
    #[schema(example = "partner_collect")]
    mode: String,
    /// Payment partner; required in the collect and split modes, and must support them.
    payment_provider: Option<String>,
    /// Payout partner; required in the collect mode.
    payout_provider: Option<String>,
    /// Days the seller's share is held after the money comes in, 0 to 365. Defaults to 7.
    hold_days: Option<i32>,
    /// `partner_collection` (default) or `seller_direct`: where cash on delivery goes.
    cod_remittance: Option<String>,
}

/// Get the money-flow settings in force for a unit.
///
/// Requires `ledger.read` on the unit. Settings are inherited from the nearest unit above that has
/// them; with none set, the unit collects its own money (`self_collect`).
#[utoipa::path(
    get,
    path = "/flow-settings/{org_id}",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "get_ledger_flow_settings",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "Id of the unit.")),
    responses(
        (status = 200, description = "The settings in force.", body = Data<FlowSettings>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read` (`permission_denied`) or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
    )
)]
async fn get_flow_settings(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
) -> Result<Data<FlowSettings>, Error> {
    let found = service.flow_settings_for(&tenant, &principal, OrgId::from_uuid(org_id)).await?;
    Ok(Data { data: found })
}

/// Set the money-flow mode of a unit.
///
/// Requires `ledger.settings_manage` on the unit. Units below inherit it. In `partner_collect`, a
/// licensed partner collects for the marketplace (this unit's book) and pays sellers out after the
/// hold; in `split_at_gateway` the partner splits each payment. The platform itself never holds the
/// money.
#[utoipa::path(
    put,
    path = "/flow-settings/{org_id}",
    extensions(("x-permissions" = json!(["ledger.settings_manage"]))),
    operation_id = "set_ledger_flow_settings",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "Id of the unit.")),
    request_body = FlowSettingsRequest,
    responses(
        (status = 200, description = "The settings now in force.", body = Data<FlowSettings>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.settings_manage` (`permission_denied`) or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 422, description = "A value is invalid (every problem in `errors`) or the partner cannot work in this mode (`ledger.flow_mode_unsupported`).", body = Problem),
    )
)]
async fn put_flow_settings(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
    ValidJson(input): ValidJson<FlowSettingsRequest>,
) -> Result<Data<FlowSettings>, Error> {
    let input = FlowSettingsInput {
        mode: input.mode,
        payment_provider: input.payment_provider,
        payout_provider: input.payout_provider,
        hold_days: input.hold_days,
        cod_remittance: input.cod_remittance,
    };
    let found =
        service.set_flow_settings(&tenant, &principal, OrgId::from_uuid(org_id), input).await?;
    Ok(Data { data: found })
}

// ---- Tạm giữ ------------------------------------------------------------------------------------

/// Which held amounts.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct HoldsQuery {
    /// `held` or `released`.
    status: Option<String>,
    /// Only for this seller (unit id).
    seller_org_id: Option<Uuid>,
    /// Only for documents of this module (e.g. `orders`).
    source_module: Option<String>,
    /// Only for this document.
    source_id: Option<String>,
}

/// List seller money held until the return window ends.
///
/// Requires `ledger.read`; shows the holds in the books of your scope.
#[utoipa::path(
    get,
    path = "/holds",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "list_ledger_holds",
    security(("bearer" = [])),
    params(HoldsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of held amounts.", body = Page<Hold>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_holds(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<HoldsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<Hold>, Error> {
    // Quyền trước tham số trang: thiếu quyền là 403, không phải 422 (luật `permission-first` của xtask).
    principal.require(&crate::permissions::READ)?;
    let filter = HoldFilter {
        status: query.status,
        seller_org_id: query.seller_org_id,
        source_module: query.source_module,
        source_id: query.source_id,
    };
    let page =
        service.list_holds(&tenant, &principal, filter, &PageRequest::try_from(page)?).await?;
    let Page { data, pagination } = page;
    let data = data.iter().map(describe_hold).collect::<Result<Vec<_>, _>>()?;
    Ok(Page { data, pagination })
}

/// Release held seller money now.
///
/// Requires `ledger.holds_release` on the book's unit. Moves what is still held to the seller's
/// available balance (the job does this when the return window ends). Releasing a released hold
/// returns it unchanged.
#[utoipa::path(
    post,
    path = "/holds/{id}/release",
    extensions(("x-permissions" = json!(["ledger.holds_release"]))),
    operation_id = "release_ledger_hold",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the hold.")),
    responses(
        (status = 200, description = "The hold after the release.", body = Data<Hold>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.holds_release`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such hold in your scope; code `ledger.hold_not_found`.", body = Problem),
        (status = 409, description = "The accounting period is closed (`ledger.period_closed`).", body = Problem),
    )
)]
async fn release_hold(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Hold>, Error> {
    let found = service.release_hold_manual(&tenant, &principal, id).await?;
    Ok(Data { data: found })
}

// ---- Chi trả ------------------------------------------------------------------------------------

/// A payout to make.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerPayoutRequest)]
pub(crate) struct PayoutRequest {
    /// The seller (unit) to pay.
    payee_org_id: Uuid,
    /// At most the seller's available balance.
    amount: Money,
    /// Short text, 1 to 140 characters, no personal data.
    description: String,
    /// Your id of this payout: sending it again returns the same payout (and asks the partner again
    /// if it did not answer the first time).
    reference: String,
}

/// Which payouts.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PayoutsQuery {
    /// `pending`, `sent`, `succeeded` or `failed`.
    status: Option<String>,
    /// Only payouts to this seller.
    payee_org_id: Option<Uuid>,
}

/// Pay a seller its available balance.
///
/// Requires `ledger.payouts_create` on the seller's unit, and the seller must be in the
/// `partner_collect` mode. The balance is taken first, then the payout partner pays from its
/// collection account; if it refuses, the amount goes back to the balance. Payouts never exceed
/// the available balance, even when sent at the same time.
#[utoipa::path(
    post,
    path = "/payouts",
    extensions(("x-permissions" = json!(["ledger.payouts_create"]))),
    operation_id = "create_ledger_payout",
    security(("bearer" = [])),
    request_body = PayoutRequest,
    responses(
        (status = 200, description = "The payout was made before with this `reference` and content.", body = Data<Payout>),
        (status = 201, description = "The payout.", body = Data<Payout>, headers(("Location" = String, description = "Path of the payout."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.payouts_create` (`permission_denied`) or the seller is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 409, description = "The available balance is not enough (`ledger.insufficient_balance`).", body = Problem),
        (status = 422, description = "A value is invalid, the seller is not in the collect mode (`ledger.flow_mode_unsupported`), the payout partner is not set up (`ledger.payout_not_configured`) or the reference was used for other content (`reference_reused`).", body = Problem),
        (status = 503, description = "The partner did not answer; send the same request again (`payment.provider_unavailable`).", body = Problem),
    )
)]
async fn create_payout(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<PayoutRequest>,
) -> Result<Response, Error> {
    let payout = NewPayout::new(
        OrgId::from_uuid(input.payee_org_id),
        input.amount,
        input.description,
        input.reference,
    );
    let outcome = service.create_payout_manual(&tenant, &principal, payout).await?;
    Ok(submitted(outcome, |payout| format!("/ledger/payouts/{}", payout.id)))
}

/// List payouts in your scope.
///
/// Requires `ledger.read`.
#[utoipa::path(
    get,
    path = "/payouts",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "list_ledger_payouts",
    security(("bearer" = [])),
    params(PayoutsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of payouts.", body = Page<Payout>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_payouts(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PayoutsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<Payout>, Error> {
    // Quyền trước tham số trang: thiếu quyền là 403, không phải 422 (luật `permission-first` của xtask).
    principal.require(&crate::permissions::READ)?;
    let filter = PayoutFilter { status: query.status, payee_org_id: query.payee_org_id };
    let page =
        service.list_payouts(&tenant, &principal, filter, &PageRequest::try_from(page)?).await?;
    let Page { data, pagination } = page;
    let data = data.iter().map(describe_payout).collect::<Result<Vec<_>, _>>()?;
    Ok(Page { data, pagination })
}

/// Get a payout.
///
/// Requires `ledger.read` on the book's unit.
#[utoipa::path(
    get,
    path = "/payouts/{id}",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "get_ledger_payout",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the payout.")),
    responses(
        (status = 200, description = "The payout.", body = Data<Payout>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such payout in your scope; code `ledger.payout_not_found`.", body = Problem),
    )
)]
async fn get_payout(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Payout>, Error> {
    let row = service.payout_for(&tenant, &principal, id).await?;
    Ok(Data { data: describe_payout(&row)? })
}

// ---- Tài khoản ngân hàng, VietQR -----------------------------------------------------------------

/// A bank account money is transferred into (the unit's own account, never the platform's).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerBankAccount)]
pub(crate) struct BankAccountResponse {
    id: Uuid,
    org_id: Uuid,
    /// NAPAS bank identification number, e.g. `970436`.
    bank_bin: String,
    account_number: String,
    account_name: String,
    /// Bank feed that reports the transfers, if any.
    feed_provider: Option<String>,
    /// `active` or `disabled`.
    status: String,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<bank_account::Model> for BankAccountResponse {
    fn from(row: bank_account::Model) -> Self {
        BankAccountResponse {
            id: row.id,
            org_id: row.org_id,
            bank_bin: row.bank_bin,
            account_number: row.account_number,
            account_name: row.account_name,
            feed_provider: row.feed_provider,
            status: row.status,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A bank account to register.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerBankAccountRequest)]
pub(crate) struct BankAccountRequest {
    /// The unit that owns the account; default the tenant's root.
    org_id: Option<Uuid>,
    /// NAPAS bank identification number (6 digits), e.g. `970436`.
    #[schema(example = "970436")]
    bank_bin: String,
    /// 1 to 19 letters or digits.
    account_number: String,
    /// Name on the account, 1 to 50 characters.
    account_name: String,
    /// Bank feed (statement reader) that reports transfers into this account.
    feed_provider: Option<String>,
}

/// List the bank accounts in your scope.
///
/// Requires `ledger.read`.
#[utoipa::path(
    get,
    path = "/bank-accounts",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "list_ledger_bank_accounts",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of bank accounts.", body = Page<BankAccountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_bank_accounts(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<BankAccountResponse>, Error> {
    // Quyền trước tham số trang: thiếu quyền là 403, không phải 422 (luật `permission-first` của xtask).
    principal.require(&crate::permissions::READ)?;
    let page =
        service.list_bank_accounts(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(BankAccountResponse::from))
}

/// Register a bank account for VietQR transfers.
///
/// Requires `ledger.settings_manage` on the unit. Buyers transfer straight into this account; the
/// bank feed reports the transfers so payments are matched automatically.
#[utoipa::path(
    post,
    path = "/bank-accounts",
    extensions(("x-permissions" = json!(["ledger.settings_manage"]))),
    operation_id = "create_ledger_bank_account",
    security(("bearer" = [])),
    request_body = BankAccountRequest,
    responses(
        (status = 201, description = "The bank account.", body = Data<BankAccountResponse>, headers(("Location" = String, description = "Path of the bank accounts."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.settings_manage` (`permission_denied`) or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 409, description = "The account is already registered (`ledger.bank_account_exists`).", body = Problem),
        (status = 422, description = "A value is invalid (unknown bank BIN, bad account number…); every problem in `errors`.", body = Problem),
    )
)]
async fn create_bank_account(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<BankAccountRequest>,
) -> Result<Created<BankAccountResponse>, Error> {
    let org = input.org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
    let account = NewBankAccount {
        org,
        bank_bin: input.bank_bin,
        account_number: input.account_number,
        account_name: input.account_name,
        feed_provider: input.feed_provider,
    };
    let row = service.create_bank_account(&tenant, &principal, account).await?;
    Ok(Created::new("/ledger/bank-accounts".to_owned(), row.into()))
}

/// Changes to a bank account. Fields left out stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerBankAccountChangeRequest)]
pub(crate) struct BankAccountChangeRequest {
    /// Name on the account, 1 to 50 characters.
    account_name: Option<String>,
    /// Bank feed that reports transfers into this account; `null` stops reading a feed. Changing
    /// it reads the new feed from its start.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    feed_provider: Option<Option<String>>,
    /// `active`, or `disabled`: new VietQR payments no longer go to it and its feed is not read;
    /// payments already created are still matched.
    status: Option<String>,
}

/// Change a bank account: name, bank feed, status.
///
/// Requires `ledger.settings_manage` on the account's unit. The bank and the account number never
/// change: register the other account instead, then disable this one.
#[utoipa::path(
    patch,
    path = "/bank-accounts/{id}",
    extensions(("x-permissions" = json!(["ledger.settings_manage"]))),
    operation_id = "update_ledger_bank_account",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the bank account.")),
    request_body = BankAccountChangeRequest,
    responses(
        (status = 200, description = "The bank account after the change.", body = Data<BankAccountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.settings_manage` (`permission_denied`) or not on the account's unit (`org_out_of_scope`).", body = Problem),
        (status = 404, description = "No such bank account in your scope; code `ledger.bank_account_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid (unknown bank feed, unknown status…); every problem in `errors`.", body = Problem),
    )
)]
async fn update_bank_account(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<BankAccountChangeRequest>,
) -> Result<Data<BankAccountResponse>, Error> {
    let change = BankAccountChange {
        account_name: input.account_name,
        feed_provider: input.feed_provider,
        status: input.status,
    };
    let row = service.update_bank_account(&tenant, &principal, id, change).await?;
    Ok(Data { data: row.into() })
}

/// What to put in a sample VietQR code.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerVietqrPreviewRequest)]
pub(crate) struct VietqrPreviewRequest {
    /// Amount in VND, e.g. `{"amount": "10000", "currency": "VND"}`.
    amount: Money,
    /// Transfer note, 1 to 25 letters, digits or spaces without accents; default a sample transfer
    /// code like the ones real payments get.
    memo: Option<String>,
}

/// A sample VietQR code for a bank account.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerVietqrPreview)]
pub(crate) struct VietqrPreviewResponse {
    /// The VietQR (EMVCo) text to draw as a QR code.
    qr_payload: String,
    /// The transfer note inside the code.
    memo: String,
    amount: Money,
    /// The account the code pays into.
    bank_account: BankAccountResponse,
}

/// Make a sample VietQR code for a bank account, without creating a payment.
///
/// Requires `ledger.read` on the account's unit. Nothing is stored: scan the code with a banking app
/// to check that it shows the right bank, account number and name before taking real payments —
/// do not complete the transfer, it would not be matched to any payment.
#[utoipa::path(
    post,
    path = "/bank-accounts/{id}/vietqr-preview",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "preview_ledger_vietqr",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the bank account.")),
    request_body = VietqrPreviewRequest,
    responses(
        (status = 200, description = "The sample code.", body = Data<VietqrPreviewResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such bank account in your scope; code `ledger.bank_account_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid (amount not positive VND, note too long…); every problem in `errors`.", body = Problem),
    )
)]
async fn preview_vietqr(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<VietqrPreviewRequest>,
) -> Result<Data<VietqrPreviewResponse>, Error> {
    let preview = service.preview_vietqr(&tenant, &principal, id, input.amount, input.memo).await?;
    Ok(Data {
        data: VietqrPreviewResponse {
            qr_payload: preview.qr_payload,
            memo: preview.memo,
            amount: preview.amount,
            bank_account: preview.account.into(),
        },
    })
}

/// A VietQR payment to create.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerVietqrPaymentRequest)]
pub(crate) struct VietqrPaymentRequest {
    /// The unit that receives the money; default the tenant's root.
    org_id: Option<Uuid>,
    /// Receiving bank account; may be left out when the unit has only one active account.
    bank_account_id: Option<Uuid>,
    /// In VND.
    amount: Money,
    /// Short text, 1 to 140 characters, no personal data.
    description: String,
    /// Id of your document (order, invoice…).
    source_id: Option<String>,
    /// Your id of this payment: sending it again returns the same payment.
    reference: String,
}

/// Create a VietQR bank transfer payment.
///
/// Requires `ledger.payments_create` on the unit. The answer has `qr_payload` (the VietQR code to
/// show) and `transfer_code`, which the transfer description must keep. When the bank feed reports
/// a transfer with the code and the exact amount, the payment succeeds; another amount, an unknown
/// code or a second transfer goes to `GET /exceptions`. The money goes straight to the unit's bank
/// account. The payment expires like every payment (`expires_at`).
#[utoipa::path(
    post,
    path = "/payments/vietqr",
    extensions(("x-permissions" = json!(["ledger.payments_create"]))),
    operation_id = "create_ledger_vietqr_payment",
    security(("bearer" = [])),
    request_body = VietqrPaymentRequest,
    responses(
        (status = 200, description = "The payment was created before with this `reference` and content.", body = Data<Payment>),
        (status = 201, description = "The payment.", body = Data<Payment>, headers(("Location" = String, description = "Path of the payment."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.payments_create` (`permission_denied`) or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 404, description = "The unit has no such active bank account (`ledger.bank_account_not_found`).", body = Problem),
        (status = 422, description = "A value is invalid (amount not in VND…) or the reference was used for other content (`reference_reused`).", body = Problem),
    )
)]
async fn create_vietqr_payment(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<VietqrPaymentRequest>,
) -> Result<Response, Error> {
    let org = input.org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
    let mut payment =
        NewTransferPayment::new(org, input.amount, input.description, "ledger", input.reference);
    payment.bank_account = input.bank_account_id;
    payment.source_id = input.source_id;
    let outcome = service.create_transfer_payment_manual(&tenant, &principal, payment).await?;
    Ok(submitted(outcome, |payment| format!("/ledger/payments/{}", payment.id)))
}

/// Receipt of a bank feed notification.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerBankFeedReceipt)]
pub(crate) struct BankFeedReceipt {
    /// Transactions stored now (ones sent before are not counted).
    stored: u64,
}

/// Receive bank account transactions from a bank feed.
///
/// No sign-in: the bank feed signs every notification and the signature is checked on the raw body
/// before anything is stored. A transaction sent again is stored once; matching runs in the
/// background. Point the bank feed's webhook setting at this URL.
#[utoipa::path(
    post,
    path = "/bank-feeds/{provider}",
    extensions(("x-public" = json!(true))),
    operation_id = "receive_ledger_bank_feed",
    params(("provider" = String, Path, description = "Bank feed name.")),
    request_body(content = String, description = "The bank feed's notification, as it sends it.", content_type = "application/json"),
    responses(
        (status = 200, description = "Stored (or stored before).", body = Data<BankFeedReceipt>),
        (status = 401, description = "The signature is not valid; code `payment.signature_invalid`.", body = Problem),
        (status = 413, description = "The body is larger than `AUTOXIDE_LEDGER_WEBHOOK_MAX_BYTES`.", body = Problem),
        (status = 422, description = "The bank feed is not set up on this server; code `ledger.bank_feed_not_configured`.", body = Problem),
    )
)]
async fn receive_bank_feed(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    Path(provider): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Data<BankFeedReceipt>, Error> {
    let stored = service.receive_bank_feed(&tenant, &provider, &headers, &body).await?;
    Ok(Data { data: BankFeedReceipt { stored } })
}

/// Receive bank account transactions from a bank feed at a tenant's own address.
///
/// No sign-in. The last part of the address identifies the tenant (copy the address from
/// `GET /api/v1/kernel/connections/webhooks`); the notification is then checked with that tenant's
/// secret at the bank feed before anything is stored. A transaction sent again is stored once;
/// matching runs in the background. The answer is the one the bank feed expects (its documented
/// acknowledgement), or the standard envelope when it expects nothing special.
#[utoipa::path(
    post,
    path = "/bank-feeds/{provider}/{hook}",
    extensions(("x-public" = json!(true))),
    operation_id = "receive_ledger_tenant_bank_feed",
    params(
        ("provider" = String, Path, description = "Bank feed name, e.g. `sepay`."),
        ("hook" = String, Path, description = "The tenant's webhook identifier."),
    ),
    request_body(content = String, description = "The bank feed's notification, as it sends it.", content_type = "application/json"),
    responses(
        (status = 200, description = "Stored (or stored before). The body is the bank feed's expected acknowledgement when it has one.", body = Data<BankFeedReceipt>),
        (status = 204, description = "Stored (or stored before), for bank feeds that expect an empty answer."),
        (status = 401, description = "The signature is not valid; code `payment.signature_invalid`.", body = Problem),
        (status = 404, description = "The address is not known; code `provider.webhook_unknown`.", body = Problem),
        (status = 413, description = "The body is larger than `AUTOXIDE_LEDGER_WEBHOOK_MAX_BYTES`.", body = Problem),
        (status = 422, description = "The bank feed is not installed on this server (`ledger.bank_feed_not_configured`) or the tenant has not set its account (`provider.not_configured`).", body = Problem),
    )
)]
async fn receive_tenant_bank_feed(
    State(service): State<Arc<LedgerService>>,
    Path((provider, hook)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Error> {
    let tenant = service.webhook_tenant("bank-feeds", &provider, &hook).await?;
    let stored = service.receive_bank_feed(&tenant, &provider, &headers, &body).await?;
    Ok(match service.bank_feed(&provider)?.webhook_ack() {
        Some(ack) => ack.into_response(),
        None => Data { data: BankFeedReceipt { stored } }.into_response(),
    })
}

// ---- Niêm phong ----------------------------------------------------------------------------------

/// How many entries of a book are in its hash chain.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerSealStatus)]
pub(crate) struct SealStatusResponse {
    book_id: Uuid,
    /// The period counted, or `null` for the whole book.
    period_id: Option<Uuid>,
    total: u64,
    sealed: u64,
    /// When the oldest entry not yet sealed was posted; entries are sealed within seconds.
    oldest_unsealed_at: Option<Timestamp>,
}

/// The result of checking a book's hash chain.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerChainCheck)]
pub(crate) struct ChainCheckResponse {
    book_id: Uuid,
    /// `true` when every sealed entry and seal is as it was sealed.
    intact: bool,
    /// Number of sealed entries checked (when intact).
    checked: u64,
    /// Where the chain first breaks, when it is not intact.
    broken: Option<ChainBreak>,
    checked_at: Timestamp,
}

/// The first place a hash chain breaks.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = LedgerChainBreak)]
pub(crate) struct ChainBreak {
    /// Sequence number in the chain where the check failed.
    sequence: i64,
    /// The entry there, when it still exists.
    entry_id: Option<Uuid>,
    /// What was found, in English, e.g. `the entry or its lines were changed`.
    reason: String,
}

/// Check a book's hash chain now.
///
/// Requires `ledger.read` on the book's unit. Reads every sealed entry of the book again and
/// compares it with its seal, the way the daily check does (`autoxide doctor`): `intact: false`
/// means someone changed or deleted entries outside the API — keep the database as it is and
/// investigate. Only reads; takes longer the more entries the book has. Entries not yet sealed are
/// not checked (see `seal-status`).
#[utoipa::path(
    get,
    path = "/books/{id}/integrity",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "check_ledger_integrity",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the book.")),
    responses(
        (status = 200, description = "The result of the check.", body = Data<ChainCheckResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such book in your scope; code `ledger.book_not_found`.", body = Problem),
    )
)]
async fn verify_book(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ChainCheckResponse>, Error> {
    let verdict = service.verify_book(&tenant, &principal, id).await?;
    let (intact, checked, broken) = match verdict {
        crate::chain::Verdict::Intact(checked) => (true, checked, None),
        crate::chain::Verdict::Broken { sequence, entry, reason } => {
            (false, 0, Some(ChainBreak { sequence, entry_id: entry, reason: reason.to_owned() }))
        }
    };
    Ok(Data {
        data: ChainCheckResponse {
            book_id: id,
            intact,
            checked,
            broken,
            checked_at: Timestamp::now(),
        },
    })
}

/// Which period.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SealQuery {
    /// Count only the entries of this period.
    period_id: Option<Uuid>,
}

/// Get the seal status of a book.
///
/// Requires `ledger.read` on the book's unit. Shows how many entries are sealed into the book's hash
/// chain (tamper evidence) and the oldest one still waiting.
#[utoipa::path(
    get,
    path = "/books/{id}/seal-status",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "get_ledger_seal_status",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the book."), SealQuery),
    responses(
        (status = 200, description = "The counts.", body = Data<SealStatusResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such book (`ledger.book_not_found`) or period (`ledger.period_not_found`) in your scope.", body = Problem),
    )
)]
async fn seal_status(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<SealQuery>,
) -> Result<Data<SealStatusResponse>, Error> {
    let found = service.seal_status(&tenant, &principal, id, query.period_id).await?;
    Ok(Data {
        data: SealStatusResponse {
            book_id: id,
            period_id: query.period_id,
            total: found.total,
            sealed: found.sealed,
            oldest_unsealed_at: found.oldest_unsealed_at,
        },
    })
}
