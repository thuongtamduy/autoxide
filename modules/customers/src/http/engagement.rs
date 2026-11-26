//! Endpoint giữ chân người mua: yêu thích, vừa xem, giới thiệu bạn (người mua, `/me`); quy tắc nhắc
//! giỏ bỏ dở và thưởng giới thiệu (quản trị).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use sea_orm::Set;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::engagement::WishlistEntry;
use crate::entity::{recent_view, referral_rule, reminder_rule, wishlist_item};
use crate::permissions::ENGAGEMENT_MANAGE;
use crate::service::CustomersService;

/// A product (or one variant of it) in the wishlist.
#[derive(Debug, Serialize, Deserialize, Validate, ToSchema)]
#[schema(as = WishlistItemInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct WishlistItemRequest {
    product_id: Uuid,
    /// Omitted: the product as a whole.
    variant_id: Option<Uuid>,
}

/// Items kept in the browser before signing in.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeWishlistRequest {
    /// At most 1000; duplicates and items already in the wishlist are skipped.
    #[validate(length(max = 1000))]
    items: Vec<WishlistItemRequest>,
}

/// A wishlist item.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct WishlistItem {
    pub product_id: Uuid,
    pub variant_id: Option<Uuid>,
    pub added_at: Timestamp,
}

impl From<wishlist_item::Model> for WishlistItem {
    fn from(row: wishlist_item::Model) -> Self {
        WishlistItem {
            product_id: row.product_id,
            variant_id: row.variant_id,
            added_at: row.added_at,
        }
    }
}

/// A recently viewed product.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct RecentlyViewed {
    pub product_id: Uuid,
    pub viewed_at: Timestamp,
}

impl From<recent_view::Model> for RecentlyViewed {
    fn from(row: recent_view::Model) -> Self {
        RecentlyViewed { product_id: row.product_id, viewed_at: row.viewed_at }
    }
}

/// A product the buyer just looked at.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ViewRequest {
    product_id: Uuid,
}

/// Your referral code.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct MyReferral {
    /// Share it: a friend enters it within the first days after signing up.
    pub code: String,
    /// Friends whose first order earned the reward.
    pub rewarded: u64,
}

/// A referral code to enter.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyReferralRequest {
    #[validate(length(min = 4, max = 16))]
    code: String,
}

/// A referral entered by the buyer.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ReferralResponse {
    pub id: Uuid,
    /// `pending` until your first order completes, then `rewarded`.
    pub status: String,
}

/// Abandoned cart reminder rule.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ReminderRule {
    pub id: Uuid,
    /// Minutes after a cart is abandoned at which reminders go out.
    pub delays_minutes: Vec<i64>,
    pub enabled: bool,
    pub effective_from: Timestamp,
}

impl From<reminder_rule::Model> for ReminderRule {
    fn from(row: reminder_rule::Model) -> Self {
        ReminderRule {
            delays_minutes: serde_json::from_str(&row.delays_minutes).unwrap_or_default(),
            id: row.id,
            enabled: row.enabled,
            effective_from: row.effective_from,
        }
    }
}

/// A new reminder rule; it replaces the current one from now on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateReminderRuleRequest {
    /// 1 to 5 delays of 15 to 10080 minutes, e.g. `[60, 1440]`.
    delays_minutes: Vec<i64>,
    enabled: bool,
}

/// Referral reward rule.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ReferralRule {
    pub id: Uuid,
    /// Points for the buyer who shared the code.
    pub referrer_points: i64,
    /// Points for the new buyer.
    pub referee_points: i64,
    /// Voucher promotion for the buyer who shared the code.
    pub referrer_promotion_id: Option<Uuid>,
    /// Voucher promotion for the new buyer.
    pub referee_promotion_id: Option<Uuid>,
    /// Goods of the first order must reach this amount (smallest currency unit).
    pub min_first_order_minor: i64,
    pub currency: String,
    pub effective_from: Timestamp,
}

/// A new referral reward rule; it replaces the current one from now on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateReferralRuleRequest {
    /// Points for the buyer who shared the code (0 to 1000000).
    referrer_points: i64,
    /// Points for the new buyer (0 to 1000000).
    referee_points: i64,
    referrer_promotion_id: Option<Uuid>,
    referee_promotion_id: Option<Uuid>,
    /// Smallest currency unit; 0 or more.
    min_first_order_minor: i64,
    /// ISO 4217 code, e.g. `VND`.
    currency: String,
}

impl From<referral_rule::Model> for ReferralRule {
    fn from(row: referral_rule::Model) -> Self {
        ReferralRule {
            id: row.id,
            referrer_points: row.referrer_points,
            referee_points: row.referee_points,
            referrer_promotion_id: row.referrer_promotion_id,
            referee_promotion_id: row.referee_promotion_id,
            min_first_order_minor: row.min_first_order_minor,
            currency: row.currency,
            effective_from: row.effective_from,
        }
    }
}

/// List your wishlist, newest first.
///
/// Any signed-in buyer. Most recently added first unless `order=asc`; paged with `limit` /
/// `cursor`.
#[utoipa::path(
    get, path = "/me/wishlist", extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_wishlist", security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of your wishlist, most recently added first.", body = Page<WishlistItem>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_wishlist(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<WishlistItem>, Error> {
    let page = PageRequest::try_from(page)?;
    let rows = service.wishlist_page(&tenant, principal.user_id(), &page).await?;
    Ok(rows.map(WishlistItem::from))
}

/// Add a product to your wishlist.
///
/// Any signed-in buyer. Adding the same product twice does nothing.
#[utoipa::path(
    post, path = "/me/wishlist", extensions(("x-permissions" = json!([]))),
    operation_id = "add_to_my_wishlist", security(("bearer" = [])), request_body = WishlistItemRequest,
    responses(
        (status = 200, description = "Your wishlist after the change.", body = Data<Vec<WishlistItem>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "The wishlist is full; code `customers.wishlist_full`.", body = Problem),
    )
)]
pub(crate) async fn add_wishlist(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<WishlistItemRequest>,
) -> Result<Data<Vec<WishlistItem>>, Error> {
    let entry = WishlistEntry { product_id: input.product_id, variant_id: input.variant_id };
    let rows = service.add_to_wishlist(&tenant, principal.user_id(), &[entry], true).await?;
    Ok(Data { data: rows.into_iter().map(WishlistItem::from).collect() })
}

/// Merge the wishlist kept in the browser into your account.
///
/// Any signed-in buyer; call it right after signing in. Duplicates are skipped; items beyond the
/// wishlist limit are left out.
#[utoipa::path(
    post, path = "/me/wishlist/merge", extensions(("x-permissions" = json!([]))),
    operation_id = "merge_my_wishlist", security(("bearer" = [])), request_body = MergeWishlistRequest,
    responses(
        (status = 200, description = "Your wishlist after merging.", body = Data<Vec<WishlistItem>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "Too many items; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn merge_wishlist(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<MergeWishlistRequest>,
) -> Result<Data<Vec<WishlistItem>>, Error> {
    let entries: Vec<WishlistEntry> = input
        .items
        .iter()
        .map(|item| WishlistEntry { product_id: item.product_id, variant_id: item.variant_id })
        .collect();
    let rows = service.add_to_wishlist(&tenant, principal.user_id(), &entries, false).await?;
    Ok(Data { data: rows.into_iter().map(WishlistItem::from).collect() })
}

/// Remove a product (all its variants) from your wishlist.
///
/// Any signed-in buyer. Removing a product not in the wishlist does nothing.
#[utoipa::path(
    delete, path = "/me/wishlist/{product_id}", extensions(("x-permissions" = json!([]))),
    operation_id = "remove_from_my_wishlist", security(("bearer" = [])),
    params(("product_id" = Uuid, Path, description = "Id of the product.")),
    responses(
        (status = 204, description = "Not in the wishlist any more."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
pub(crate) async fn remove_wishlist(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(product): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.remove_from_wishlist(&tenant, principal.user_id(), product).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List the products you looked at recently, newest first.
///
/// Any signed-in buyer. Guests keep this list in the browser. Newest first unless `order=asc`;
/// paged with `limit` / `cursor`.
#[utoipa::path(
    get, path = "/me/recently-viewed", extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_recently_viewed", security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of recently viewed products, newest first.", body = Page<RecentlyViewed>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_views(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<RecentlyViewed>, Error> {
    let page = PageRequest::try_from(page)?;
    let rows = service.recently_viewed(&tenant, principal.user_id(), &page).await?;
    Ok(rows.map(RecentlyViewed::from))
}

/// Record that you looked at a product.
///
/// Any signed-in buyer. Looking again moves it to the top.
#[utoipa::path(
    post, path = "/me/recently-viewed", extensions(("x-permissions" = json!([]))),
    operation_id = "record_my_view", security(("bearer" = [])), request_body = ViewRequest,
    responses(
        (status = 204, description = "Recorded."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
pub(crate) async fn record_view(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ViewRequest>,
) -> Result<StatusCode, Error> {
    service.record_view(&tenant, principal.user_id(), input.product_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Get your referral code.
///
/// Any signed-in buyer; the code is created on first use.
#[utoipa::path(
    get, path = "/me/referral", extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_referral", security(("bearer" = [])),
    responses(
        (status = 200, description = "Your code and how many friends earned the reward.", body = Data<MyReferral>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
pub(crate) async fn my_referral(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<MyReferral>, Error> {
    let (code, rewarded) = service.my_referral(&tenant, principal.user_id()).await?;
    Ok(Data { data: MyReferral { code, rewarded } })
}

/// Enter the referral code of the friend who invited you.
///
/// Any signed-in buyer, within the first days after signing up and before the first order. Both of
/// you are rewarded when your first order completes.
#[utoipa::path(
    post, path = "/me/referral/apply", extensions(("x-permissions" = json!([]))),
    operation_id = "apply_referral_code", security(("bearer" = [])), request_body = ApplyReferralRequest,
    responses(
        (status = 201, description = "Recorded; rewarded after your first order completes.", body = Data<ReferralResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 409, description = "Already referred, already ordered or too late; code `customers.referral_not_eligible`.", body = Problem),
        (status = 422, description = "Unknown code (`customers.referral_code_invalid`) or your own code or contact (`customers.self_referral`).", body = Problem),
    )
)]
pub(crate) async fn apply_referral(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ApplyReferralRequest>,
) -> Result<Created<ReferralResponse>, Error> {
    let row = service.apply_referral(&tenant, principal.user_id(), &input.code).await?;
    Ok(Created::new(
        "/customers/me/referral".to_owned(),
        ReferralResponse { id: row.id, status: row.status },
    ))
}

/// List abandoned cart reminder rules, newest first.
///
/// Requires `customers.engagement.manage`. Paged with `limit` / `cursor`; `order=asc` for oldest
/// first.
#[utoipa::path(
    get, path = "/reminder-rules", extensions(("x-permissions" = json!(["customers.engagement.manage"]))),
    operation_id = "list_cart_reminder_rules", security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of rule versions, newest first.", body = Page<ReminderRule>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.engagement.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_reminder_rules(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<ReminderRule>, Error> {
    principal.require(&ENGAGEMENT_MANAGE)?;
    let page = PageRequest::try_from(page)?;
    Ok(service.reminder_rules(&tenant, &page).await?.map(ReminderRule::from))
}

/// Set abandoned cart reminders.
///
/// Requires `customers.engagement.manage`. Reminders are advertising: they only go to buyers who
/// consented to marketing messages, inside the allowed hours, and stop when consent is withdrawn
/// or the buyer orders.
#[utoipa::path(
    post, path = "/reminder-rules", extensions(("x-permissions" = json!(["customers.engagement.manage"]))),
    operation_id = "create_cart_reminder_rule", security(("bearer" = [])), request_body = CreateReminderRuleRequest,
    responses(
        (status = 201, description = "In force from now.", body = Data<ReminderRule>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.engagement.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Invalid delays; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_reminder_rule(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateReminderRuleRequest>,
) -> Result<Created<ReminderRule>, Error> {
    principal.require(&ENGAGEMENT_MANAGE)?;
    let row = service.add_reminder_rule(&tenant, input.delays_minutes, input.enabled).await?;
    Ok(Created::new("/customers/reminder-rules".to_owned(), row.into()))
}

/// List referral reward rules, newest first.
///
/// Requires `customers.engagement.manage`. Paged with `limit` / `cursor`; `order=asc` for oldest
/// first.
#[utoipa::path(
    get, path = "/referral-rules", extensions(("x-permissions" = json!(["customers.engagement.manage"]))),
    operation_id = "list_referral_rules", security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of rule versions, newest first.", body = Page<ReferralRule>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.engagement.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_referral_rules(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<ReferralRule>, Error> {
    principal.require(&ENGAGEMENT_MANAGE)?;
    let page = PageRequest::try_from(page)?;
    Ok(service.referral_rules(&tenant, &page).await?.map(ReferralRule::from))
}

/// Set referral rewards.
///
/// Requires `customers.engagement.manage`. Points need a loyalty rule; vouchers need voucher
/// promotions of the promotions module.
#[utoipa::path(
    post, path = "/referral-rules", extensions(("x-permissions" = json!(["customers.engagement.manage"]))),
    operation_id = "create_referral_rule", security(("bearer" = [])), request_body = CreateReferralRuleRequest,
    responses(
        (status = 201, description = "In force from now.", body = Data<ReferralRule>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.engagement.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_referral_rule(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateReferralRuleRequest>,
) -> Result<Created<ReferralRule>, Error> {
    principal.require(&ENGAGEMENT_MANAGE)?;
    let rule = referral_rule::ActiveModel {
        referrer_points: Set(input.referrer_points),
        referee_points: Set(input.referee_points),
        referrer_promotion_id: Set(input.referrer_promotion_id),
        referee_promotion_id: Set(input.referee_promotion_id),
        min_first_order_minor: Set(input.min_first_order_minor),
        currency: Set(input.currency.to_ascii_uppercase()),
        ..Default::default()
    };
    let row = service.add_referral_rule(&tenant, rule).await?;
    Ok(Created::new("/customers/referral-rules".to_owned(), row.into()))
}
