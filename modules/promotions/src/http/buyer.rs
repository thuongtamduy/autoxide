//! Endpoint tính giỏ, lượt giữ, dữ liệu luật, ví voucher của người mua, dưới `/api/v1/promotions`.

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::{Cart, Evaluation, Redemption, Voucher, VoucherStatus};
use crate::http::dto::{LegalLimitDto, NoticeRuleDto};
use crate::permissions::{PUBLISH, READ};
use crate::service::PromotionsService;

pub(crate) fn routes(service: Arc<PromotionsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(evaluate))
        .routes(routes!(legal_limits))
        .routes(routes!(notice_rules))
        .routes(routes!(list_redemptions))
        .routes(routes!(release_redemption))
        .routes(routes!(my_vouchers))
        .routes(routes!(claim_code))
        .routes(routes!(claim))
        .with_state(service)
}

/// Thân là một `Cart` (ValidJson cần `Validate`; service kiểm giỏ).
#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub(crate) struct CartBody(pub Cart);

/// Compute the promotions of a cart, without holding or changing anything.
///
/// Requires `promotions.read` covering the unit of the cart (`org_id`). The same computation `orders` uses: every discount shared out to
/// the lines with a reason the buyer can read, gifts, codes and vouchers that do not apply (with
/// reasons, the cart still works), and the `fingerprint` to send back when the order holds them.
#[utoipa::path(
    post,
    path = "/evaluate",
    extensions(("x-permissions" = json!(["promotions.read"]))),
    operation_id = "evaluate_promotions",
    security(("bearer" = [])),
    request_body = Cart,
    responses(
        (status = 200, description = "The promotions of the cart.", body = Data<Evaluation>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.read` (`permission_denied`) or the unit of the cart is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 422, description = "The cart is invalid (see `errors`) or has too many codes (`promotions.too_many_codes`).", body = Problem),
    )
)]
async fn evaluate(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(cart): ValidJson<CartBody>,
) -> Result<Data<Evaluation>, Error> {
    principal.require(&READ)?;
    let outcome = service.evaluate_for(&tenant, &principal, &cart.0).await?;
    Ok(Data { data: outcome.evaluation })
}

/// Date of the law data to show.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct AtQuery {
    /// Day `YYYY-MM-DD`; defaults to today.
    pub at: Option<Date>,
}

/// List the legal promotion limits in force on a day.
///
/// Requires `promotions.read`. Shared law data with effective dates, legal basis and notes
/// ("to be checked against the original text"); loaded with `autoxide task
/// promotions.load_legal_data`. By promotion kind, then program kind and jurisdiction (A→Z unless
/// `order=desc`); paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/legal-limits",
    extensions(("x-permissions" = json!(["promotions.read"]))),
    operation_id = "list_promotion_legal_limits",
    security(("bearer" = [])),
    params(AtQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the limits in force.", body = Page<LegalLimitDto>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn legal_limits(
    State(service): State<Arc<PromotionsService>>,
    principal: Principal,
    Query(query): Query<AtQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<LegalLimitDto>, Error> {
    principal.require(&READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.legal_limits_at(query.at, &page).await?;
    Ok(rows.map(|row| LegalLimitDto::from(&row)))
}

/// List the promotion notice rules in force on a day.
///
/// Requires `promotions.read`. Shared law data: which promotions must be notified, to whom and
/// how many days before. By promotion kind, then jurisdiction (A→Z unless `order=desc`); paged
/// with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/notice-rules",
    extensions(("x-permissions" = json!(["promotions.read"]))),
    operation_id = "list_promotion_notice_rules",
    security(("bearer" = [])),
    params(AtQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the rules in force.", body = Page<NoticeRuleDto>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn notice_rules(
    State(service): State<Arc<PromotionsService>>,
    principal: Principal,
    Query(query): Query<AtQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<NoticeRuleDto>, Error> {
    principal.require(&READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.notice_rules_at(query.at, &page).await?;
    Ok(rows.map(|row| NoticeRuleDto::from(&row)))
}

/// Filters of the redemption list.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct RedemptionsQuery {
    /// Only the usage held for this order.
    pub order_id: Option<Uuid>,
    /// `held`, `confirmed`, `released` or `expired`.
    pub status: Option<String>,
}

/// Promotion usage held for an order.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsRedemptionItem)]
pub(crate) struct RedemptionItem {
    pub id: Uuid,
    pub order_id: Uuid,
    pub org_id: Uuid,
    pub status: String,
    pub total_discount_minor: i64,
    pub currency: String,
    pub expires_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

/// List the promotion usage held for orders.
///
/// Requires `promotions.read`; its scope decides which stores' orders you see.
#[utoipa::path(
    get,
    path = "/redemptions",
    extensions(("x-permissions" = json!(["promotions.read"]))),
    operation_id = "list_promotion_redemptions",
    security(("bearer" = [])),
    params(RedemptionsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of redemptions.", body = Page<RedemptionItem>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_redemptions(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<RedemptionsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<RedemptionItem>, Error> {
    principal.require(&READ)?;
    let page = PageRequest::try_from(page)?;
    let found =
        service.redemptions(&tenant, &principal, query.order_id, query.status, &page).await?;
    Ok(found.map(|row| RedemptionItem {
        id: row.id,
        order_id: row.order_id,
        org_id: row.org_id,
        status: row.status,
        total_discount_minor: row.total_discount_minor,
        currency: row.currency,
        expires_at: row.expires_at,
        created_at: row.created_at,
    }))
}

/// Give back the promotion usage held for an order by hand (unstick it).
///
/// Requires `promotions.publish` on the store. Same as an order being cancelled: held usage is
/// given back; confirmed usage is given back when the promotion allows it, vouchers return to
/// the wallet while valid.
#[utoipa::path(
    post,
    path = "/redemptions/{id}/release",
    extensions(("x-permissions" = json!(["promotions.publish"]))),
    operation_id = "release_promotion_redemption",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the redemption.")),
    responses(
        (status = 200, description = "Released (or already released).", body = Data<Redemption>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.publish` or the store is outside your scope.", body = Problem),
        (status = 404, description = "No such redemption; code `promotions.redemption_not_found`.", body = Problem),
    )
)]
async fn release_redemption(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Redemption>, Error> {
    principal.require(&PUBLISH)?;
    let released = service.release_by_id(&tenant, &principal, id).await?;
    Ok(Data { data: released })
}

/// Filters of the wallet.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct WalletQuery {
    /// Only vouchers in this state: `available`, `held`, `used`, `expired` or `revoked`.
    pub status: Option<VoucherStatus>,
}

/// List the vouchers in your wallet, soonest to expire first.
///
/// For buyer accounts; a staff account needs `promotions.use`.
#[utoipa::path(
    get,
    path = "/me/vouchers",
    extensions(("x-permissions" = json!(["promotions.use"])), ("x-buyers" = json!(true))),
    operation_id = "list_my_vouchers",
    security(("bearer" = [])),
    params(WalletQuery, PageQuery),
    responses(
        (status = 200, description = "One page of your vouchers.", body = Page<Voucher>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "A staff account without `promotions.use`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn my_vouchers(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<WalletQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<Voucher>, Error> {
    super::require_use(&principal)?;
    let page = PageRequest::try_from(page)?;
    service.wallet(&tenant, principal.user_id(), query.status, &page).await
}

/// A claim code to enter.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsClaimCodeRequest)]
pub(crate) struct ClaimCodeRequest {
    /// The code, spaces and case do not matter.
    pub code: String,
}

fn voucher_response(submitted: Submitted<Voucher>, location: String) -> Response {
    match submitted {
        Submitted::Created(voucher) => Created::new(location, voucher).into_response(),
        Submitted::Replayed(voucher) => Data { data: voucher }.into_response(),
    }
}

/// Enter a claim code to put its voucher into your wallet.
///
/// For buyer accounts; a staff account needs `promotions.use`. Tries are limited
/// (`promotions.too_many_attempts`). Entering the
/// same code again returns the voucher you already have (200).
#[utoipa::path(
    post,
    path = "/me/vouchers/claim-code",
    extensions(("x-permissions" = json!(["promotions.use"])), ("x-buyers" = json!(true))),
    operation_id = "claim_voucher_code",
    security(("bearer" = [])),
    request_body = ClaimCodeRequest,
    responses(
        (status = 201, description = "The voucher is in your wallet.", body = Data<Voucher>, headers(("Location" = String, description = "Path of your wallet."))),
        (status = 200, description = "You already had this voucher.", body = Data<Voucher>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "A staff account without `promotions.use`; code `permission_denied`.", body = Problem),
        (status = 409, description = "No vouchers left (`promotions.voucher_quota_exhausted`), you received all allowed (`promotions.claim_limit_reached`), or the promotion is not running (`promotions.invalid_transition`).", body = Problem),
        (status = 422, description = "The code does not exist; code `promotions.code_invalid`.", body = Problem),
        (status = 429, description = "Too many codes tried; code `promotions.too_many_attempts`.", body = Problem),
    )
)]
async fn claim_code(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(body): ValidJson<ClaimCodeRequest>,
) -> Result<Response, Error> {
    super::require_use(&principal)?;
    let submitted = service.claim_code(&tenant, &principal, &body.code).await?;
    Ok(voucher_response(submitted, "/promotions/me/vouchers".to_owned()))
}

/// Save the voucher of a public promotion into your wallet ("Save" button).
///
/// For buyer accounts; a staff account needs `promotions.use`. Only promotions with `claimable`.
/// Vouchers left are a real count.
#[utoipa::path(
    post,
    path = "/public/promotions/{id}/claim",
    extensions(("x-permissions" = json!(["promotions.use"])), ("x-buyers" = json!(true))),
    operation_id = "claim_promotion_voucher",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion.")),
    responses(
        (status = 201, description = "The voucher is in your wallet.", body = Data<Voucher>, headers(("Location" = String, description = "Path of your wallet."))),
        (status = 200, description = "You already saved it.", body = Data<Voucher>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "A staff account without `promotions.use` (`permission_denied`) or a verified phone number is needed (`promotions.claim_requires_verified_phone`).", body = Problem),
        (status = 404, description = "No such public promotion to save; code `promotions.not_found`.", body = Problem),
        (status = 409, description = "No vouchers left (`promotions.voucher_quota_exhausted`), you saved all allowed (`promotions.claim_limit_reached`), or the promotion is not running (`promotions.invalid_transition`).", body = Problem),
    )
)]
async fn claim(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Response, Error> {
    super::require_use(&principal)?;
    let submitted = service.claim(&tenant, &principal, id).await?;
    Ok(voucher_response(submitted, "/promotions/me/vouchers".to_owned()))
}

impl Validate for CartBody {
    fn validate(&self) -> Result<(), validator::ValidationErrors> {
        Ok(())
    }
}
