//! Endpoint thẻ quà tặng và số dư cửa hàng — TẮT mặc định: mọi endpoint trả 403
//! `customers.stored_value_disabled` khi người vận hành chưa bật hay khách chưa có tính năng. Không có
//! endpoint rút tiền, chuyển số dư hay đổi ra tiền mặt (luật cứng, cần đối chiếu NĐ 52/2024).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::entity::{gift_card, store_credit};
use crate::permissions::STORED_VALUE_MANAGE;
use crate::service::CustomersService;
use crate::stored_value::{NewCard, STORE_CREDIT};

/// A gift card (the code is shown only when it is issued).
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct GiftCard {
    pub id: Uuid,
    /// Last 4 characters of the code.
    pub code_last4: String,
    /// The full code: only in the answer to issuing; never stored in clear.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub initial: Money,
    pub balance: Money,
    /// `active`, `void` or `expired`.
    pub status: String,
    pub expires_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

impl GiftCard {
    fn new(row: gift_card::Model, code: Option<String>) -> Result<Self, Error> {
        Ok(GiftCard {
            initial: Money::from_columns(row.initial_minor, &row.currency)?,
            balance: Money::from_columns(row.balance_minor, &row.currency)?,
            id: row.id,
            code_last4: row.code_last4,
            code,
            status: row.status,
            expires_at: row.expires_at,
            created_at: row.created_at,
        })
    }
}

/// A gift card to issue.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct IssueGiftCardRequest {
    /// Value of the card.
    amount: Money,
    /// Your reference (e.g. the counter receipt); the same reference never issues twice.
    reference: String,
    /// 1 to 120; omitted: no expiry.
    valid_months: Option<i32>,
}

/// A gift card code to look up or attach.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GiftCardCodeRequest {
    /// The 16-character code; spaces and dashes are ignored. Sent in the body, never in the URL.
    code: String,
}

/// Store credit of a customer.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct StoreCredit {
    pub balance: Money,
    pub updated_at: Timestamp,
}

impl StoreCredit {
    fn new(row: store_credit::Model) -> Result<Self, Error> {
        Ok(StoreCredit {
            balance: Money::from_columns(row.balance_minor, &row.currency)?,
            updated_at: row.updated_at,
        })
    }
}

/// A change of store credit by staff (compensation, correction).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdjustStoreCreditRequest {
    /// Positive adds, negative removes (never below zero).
    amount: Money,
    /// Your reference; the same reference never applies twice.
    reference: String,
    /// Why, 1 to 500 characters.
    reason: String,
}

const DISABLED: &str = "Gift cards and store credit are off (`customers.stored_value_disabled`) or you lack the permission (`permission_denied`).";

/// Issue a gift card.
///
/// Requires `customers.stored_value.manage`. Only when gift cards are turned on for the shop. The
/// code is in the answer once; keep it for the buyer.
#[utoipa::path(
    post, path = "/gift-cards", extensions(("x-permissions" = json!(["customers.stored_value.manage"]))),
    operation_id = "issue_gift_card", security(("bearer" = [])), request_body = IssueGiftCardRequest,
    responses(
        (status = 201, description = "Issued; `code` is shown only now.", body = Data<GiftCard>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = DISABLED, body = Problem),
        (status = 422, description = "A value is invalid (see `errors`) or above the largest card value (`customers.face_value_too_high`).", body = Problem),
    )
)]
pub(crate) async fn issue_gift_card(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<IssueGiftCardRequest>,
) -> Result<Created<GiftCard>, Error> {
    principal.require(&STORED_VALUE_MANAGE)?;
    let card = NewCard {
        amount: input.amount,
        reference: input.reference,
        valid_months: input.valid_months,
    };
    let (row, code) = service.issue_gift_card(&tenant, &principal, card).await?;
    Ok(Created::new(format!("/customers/gift-cards/{}", row.id), GiftCard::new(row, code)?))
}

/// Get a gift card.
///
/// Requires `customers.stored_value.manage`.
#[utoipa::path(
    get, path = "/gift-cards/{id}", extensions(("x-permissions" = json!(["customers.stored_value.manage"]))),
    operation_id = "get_gift_card", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the gift card.")),
    responses(
        (status = 200, description = "The gift card (without its code).", body = Data<GiftCard>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = DISABLED, body = Problem),
        (status = 404, description = "No such gift card; code `customers.gift_card_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_gift_card(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<GiftCard>, Error> {
    principal.require(&STORED_VALUE_MANAGE)?;
    Ok(Data { data: GiftCard::new(service.gift_card(&tenant, id).await?, None)? })
}

/// Look up a gift card by its code.
///
/// Requires `customers.stored_value.manage`. Lookups are limited per person and hour against
/// guessing codes.
#[utoipa::path(
    post, path = "/gift-cards/lookup", extensions(("x-permissions" = json!(["customers.stored_value.manage"]))),
    operation_id = "lookup_gift_card", security(("bearer" = [])), request_body = GiftCardCodeRequest,
    responses(
        (status = 200, description = "The gift card (without its code).", body = Data<GiftCard>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = DISABLED, body = Problem),
        (status = 404, description = "Wrong code; code `customers.gift_card_not_found`.", body = Problem),
        (status = 429, description = "Too many lookups; code `customers.lookup_throttled`, see `Retry-After`.", body = Problem),
    )
)]
pub(crate) async fn lookup_gift_card(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<GiftCardCodeRequest>,
) -> Result<Data<GiftCard>, Error> {
    principal.require(&STORED_VALUE_MANAGE)?;
    let row = service.lookup_gift_card(&tenant, principal.user_id(), &input.code).await?;
    Ok(Data { data: GiftCard::new(row, None)? })
}

/// Void a gift card (lost, fraud).
///
/// Requires `customers.stored_value.manage`. The remaining balance is gone.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    post, path = "/gift-cards/{id}/void", extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["customers.stored_value.manage"]))),
    operation_id = "void_gift_card", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the gift card.")),
    responses(
        (status = 200, description = "Voided.", body = Data<GiftCard>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = DISABLED, body = Problem),
        (status = 404, description = "No such gift card; code `customers.gift_card_not_found`.", body = Problem),
        (status = 409, description = "Already voided or expired; code `customers.gift_card_inactive`.", body = Problem),
    )
)]
pub(crate) async fn void_gift_card(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<GiftCard>, Error> {
    principal.require(&STORED_VALUE_MANAGE)?;
    Ok(Data { data: GiftCard::new(service.void_gift_card(&tenant, &principal, id).await?, None)? })
}

/// Add or remove store credit of a customer.
///
/// Requires `customers.stored_value.manage` and `customers.read` covering the customer. Store credit
/// only comes from compensation or refunds; it cannot be bought, withdrawn or transferred.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    post, path = "/{id}/store-credit/adjustments", extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["customers.stored_value.manage"]))),
    operation_id = "adjust_store_credit", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the customer profile.")),
    request_body = AdjustStoreCreditRequest,
    responses(
        (status = 200, description = "Store credit after the change.", body = Data<StoreCredit>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = DISABLED, body = Problem),
        (status = 404, description = "No customer with this id in your scope; code `customers.profile_not_found`.", body = Problem),
        (status = 409, description = "Not enough credit to remove; code `customers.insufficient_balance`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn adjust_store_credit(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AdjustStoreCreditRequest>,
) -> Result<Data<StoreCredit>, Error> {
    principal.require(&STORED_VALUE_MANAGE)?;
    let row = service
        .adjust_store_credit(&tenant, &principal, id, input.amount, &input.reference, &input.reason)
        .await?;
    Ok(Data { data: StoreCredit::new(row)? })
}

/// List your gift cards.
///
/// Any signed-in buyer, when gift cards are on. Newest first unless `order=asc`; paged with
/// `limit` / `cursor`.
#[utoipa::path(
    get, path = "/me/gift-cards", extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_gift_cards", security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of the gift cards attached to your account, newest first.", body = Page<GiftCard>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Gift cards are off; code `customers.stored_value_disabled`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn my_gift_cards(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<GiftCard>, Error> {
    let page = PageRequest::try_from(page)?;
    let rows = service.my_gift_cards(&tenant, principal.user_id(), &page).await?;
    let data =
        rows.data.into_iter().map(|row| GiftCard::new(row, None)).collect::<Result<_, _>>()?;
    Ok(Page { data, pagination: rows.pagination })
}

/// Attach a gift card to your account.
///
/// Any signed-in buyer, when gift cards are on. The card still works only in this shop, and from
/// then on only for your account; a card attached to another account answers like a wrong code.
#[utoipa::path(
    post, path = "/me/gift-cards/attach", extensions(("x-permissions" = json!([]))),
    operation_id = "attach_my_gift_card", security(("bearer" = [])), request_body = GiftCardCodeRequest,
    responses(
        (status = 200, description = "Attached.", body = Data<GiftCard>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Gift cards are off; code `customers.stored_value_disabled`.", body = Problem),
        (status = 404, description = "Wrong code; code `customers.gift_card_not_found`.", body = Problem),
        (status = 429, description = "Too many lookups; code `customers.lookup_throttled`.", body = Problem),
    )
)]
pub(crate) async fn attach_gift_card(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<GiftCardCodeRequest>,
) -> Result<Data<GiftCard>, Error> {
    let row = service.attach_gift_card(&tenant, principal.user_id(), &input.code).await?;
    Ok(Data { data: GiftCard::new(row, None)? })
}

/// Get your store credit.
///
/// Any signed-in buyer, when store credit is on. Pay with it at checkout with the tender
/// `store_credit`.
#[utoipa::path(
    get, path = "/me/store-credit", extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_store_credit", security(("bearer" = [])),
    responses(
        (status = 200, description = "Your store credit (absent: none yet).", body = Data<Option<StoreCredit>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Store credit is off; code `customers.stored_value_disabled`.", body = Problem),
    )
)]
pub(crate) async fn my_store_credit(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Option<StoreCredit>>, Error> {
    // Kiểm tính năng trước khi `me` tạo hồ sơ, để khi tắt không để lại dấu vết nào.
    service.stored_value_allowed(&tenant, &STORE_CREDIT).await?;
    let profile = service.me(&tenant, principal.user_id()).await?;
    let row = service.store_credit_of(&tenant, profile.id).await?;
    Ok(Data { data: row.map(StoreCredit::new).transpose()? })
}
