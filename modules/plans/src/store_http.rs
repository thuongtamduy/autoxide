//! Endpoint mua trong ứng dụng (bước 69 P4): nhà vận hành nối sản phẩm của cửa hàng với gói; app của
//! khách gửi bằng chứng mua; cửa hàng gửi thông báo máy chủ. Logic ở `store.rs`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::store::AppStore;
use autoxide_kernel::tenancy::TenantContext;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::http::{SubscriptionResponse, operator};
use crate::permissions::{BILLING_MANAGE, BILLING_READ, PLANS_MANAGE, PLANS_READ};
use crate::service::{PlansService, subscription_not_found};
use crate::store::entity::{product, purchase};
use crate::store::unknown_store;

pub(crate) fn routes() -> OpenApiRouter<Arc<PlansService>> {
    OpenApiRouter::new()
        .routes(routes!(list_store_products))
        .routes(routes!(link_store_product, unlink_store_product))
        .routes(routes!(claim_store_purchase, list_store_purchases))
        .routes(routes!(receive_store_notification))
}

fn store_of(name: &str) -> Result<AppStore, Error> {
    AppStore::parse(name).ok_or_else(|| unknown_store(name))
}

/// A store product linked to a plan.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct StoreProductResponse {
    /// `app_store` or `google_play`.
    #[schema(example = "app_store")]
    store: String,
    /// The product id set up in the store.
    #[schema(example = "pro_monthly")]
    product_id: String,
    /// The base plan a purchase of this product gives.
    plan_id: Uuid,
    updated_at: DateTime<Utc>,
}

impl From<product::Model> for StoreProductResponse {
    fn from(row: product::Model) -> Self {
        StoreProductResponse {
            store: row.store,
            product_id: row.product_id,
            plan_id: row.plan_id,
            updated_at: row.updated_at,
        }
    }
}

/// Which plan a store product gives.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinkProductRequest {
    /// A base plan.
    plan_id: Uuid,
}

/// List the app store products linked to plans.
///
/// Platform staff with `plans.plans.read`.
#[utoipa::path(
    get,
    path = "/store/products",
    extensions(("x-permissions" = json!(["plans.plans.read"]))),
    operation_id = "list_store_products",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every linked product, by store and product id.", body = Data<Vec<StoreProductResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`platform_only`) or missing `plans.plans.read` (`permission_denied`).", body = Problem),
    )
)]
async fn list_store_products(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
) -> Result<Data<Vec<StoreProductResponse>>, Error> {
    operator(&principal, &PLANS_READ)?;
    let rows = service.store_products().await?;
    Ok(Data { data: rows.into_iter().map(StoreProductResponse::from).collect() })
}

/// Link an app store product to a plan.
///
/// Platform staff with `plans.plans.manage`. A purchase of this product in the app (App Store or
/// Google Play) puts the buyer's tenant on the plan for as long as the store says the purchase is
/// active. Linking again replaces the plan.
#[utoipa::path(
    put,
    path = "/store/products/{store}/{product_id}",
    extensions(("x-permissions" = json!(["plans.plans.manage"]))),
    operation_id = "link_store_product",
    security(("bearer" = [])),
    params(
        ("store" = String, Path, description = "`app_store` or `google_play`."),
        ("product_id" = String, Path, description = "The product id set up in the store."),
    ),
    request_body = LinkProductRequest,
    responses(
        (status = 200, description = "The link.", body = Data<StoreProductResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`platform_only`) or missing `plans.plans.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such store (`plans.store_unknown`) or plan (`plans.plan_not_found`).", body = Problem),
        (status = 422, description = "The plan is an add-on (`plans.addon_not_allowed`), or the product id is too long (see `errors`).", body = Problem),
    )
)]
async fn link_store_product(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path((store, product_id)): Path<(String, String)>,
    ValidJson(input): ValidJson<LinkProductRequest>,
) -> Result<Data<StoreProductResponse>, Error> {
    operator(&principal, &PLANS_MANAGE)?;
    let store = store_of(&store)?;
    let row = service.link_store_product(store, &product_id, input.plan_id).await?;
    Ok(Data { data: row.into() })
}

/// Unlink an app store product.
///
/// Platform staff with `plans.plans.manage`. Subscriptions already bought keep their plan until
/// the store period ends; new purchases of the product are refused.
#[utoipa::path(
    delete,
    path = "/store/products/{store}/{product_id}",
    extensions(("x-permissions" = json!(["plans.plans.manage"]))),
    operation_id = "unlink_store_product",
    security(("bearer" = [])),
    params(
        ("store" = String, Path, description = "`app_store` or `google_play`."),
        ("product_id" = String, Path, description = "The product id set up in the store."),
    ),
    responses(
        (status = 204, description = "Unlinked (or was not linked)."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`platform_only`) or missing `plans.plans.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such store; code `plans.store_unknown`.", body = Problem),
    )
)]
async fn unlink_store_product(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path((store, product_id)): Path<(String, String)>,
) -> Result<StatusCode, Error> {
    operator(&principal, &PLANS_MANAGE)?;
    service.unlink_store_product(store_of(&store)?, &product_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A purchase made in an app store.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimRequest {
    /// `app_store` or `google_play`.
    #[schema(example = "app_store")]
    store: String,
    /// What the store gave the app for the purchase: the signed transaction of StoreKit 2
    /// (App Store) or the `purchaseToken` (Google Play), as is.
    #[validate(length(min = 1, max = 20000))]
    proof: String,
}

/// Pay your subscription with an in-app purchase.
///
/// Requires `plans.billing.manage`. Buy a linked product in the App Store or Google Play from your
/// app, then send what the store returned: the server checks it with the store and puts your tenant
/// on the product's plan until the store period ends. The store renews, expires or refunds it later
/// on its own (a refund ends access at once). Sending the same purchase again changes nothing. Apple
/// and Google collect and keep the payment; change or cancel the subscription in the store.
#[utoipa::path(
    post,
    path = "/store/purchases",
    extensions(("x-permissions" = json!(["plans.billing.manage"]))),
    operation_id = "claim_store_purchase",
    security(("bearer" = [])),
    request_body = ClaimRequest,
    responses(
        (status = 200, description = "Your subscription after the purchase.", body = Data<SubscriptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `plans.billing.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such store; code `plans.store_unknown`.", body = Problem),
        (status = 409, description = "The purchase pays for another tenant (`plans.store_purchase_taken`), is no longer active (`plans.store_purchase_inactive`), or the plan is not sold (`plans.plan_not_active`).", body = Problem),
        (status = 422, description = "The store does not confirm the purchase (`errors` on `proof`), its product is not linked to a plan (`plans.store_product_unknown`), or the tenant is the platform, which no plan limits (`plans.platform_exempt`).", body = Problem),
        (status = 503, description = "This store is not set up on the server; code `plans.store_unavailable`.", body = Problem),
    )
)]
async fn claim_store_purchase(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ClaimRequest>,
) -> Result<Data<SubscriptionResponse>, Error> {
    principal.require(&BILLING_MANAGE)?;
    let store = store_of(&input.store)?;
    service.claim_store_purchase(Actor::from(&principal), tenant.id(), store, &input.proof).await?;
    let row = service.subscription(tenant.id()).await?.ok_or_else(subscription_not_found)?;
    Ok(Data { data: row.into() })
}

/// An app store purchase that pays (or paid) for your subscription.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct StorePurchaseResponse {
    id: Uuid,
    /// `app_store` or `google_play`.
    store: String,
    product_id: String,
    /// `active`, `expired`, `refunded` or `revoked`.
    #[schema(example = "active")]
    state: String,
    /// When the store period ends.
    expires_at: Option<DateTime<Utc>>,
    /// Made in the store's test environment.
    sandbox: bool,
    updated_at: DateTime<Utc>,
}

impl From<purchase::Model> for StorePurchaseResponse {
    fn from(row: purchase::Model) -> Self {
        StorePurchaseResponse {
            id: row.id,
            store: row.store,
            product_id: row.product_id,
            state: row.state,
            expires_at: row.expires_at,
            sandbox: row.sandbox,
            updated_at: row.updated_at,
        }
    }
}

/// List the app store purchases of your tenant.
///
/// Requires `plans.billing.read`. Newest change first.
#[utoipa::path(
    get,
    path = "/store/purchases",
    extensions(("x-permissions" = json!(["plans.billing.read"]))),
    operation_id = "list_store_purchases",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The purchases.", body = Data<Vec<StorePurchaseResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `plans.billing.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_store_purchases(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<StorePurchaseResponse>>, Error> {
    principal.require(&BILLING_READ)?;
    let rows = service.store_purchases(tenant.id()).await?;
    Ok(Data { data: rows.into_iter().map(StorePurchaseResponse::from).collect() })
}

/// Receive a server notification from an app store.
///
/// No sign-in: point App Store Server Notifications (v2) or Google Play Real-time Developer
/// Notifications here. The store's signature is checked on the raw body before anything changes;
/// each notification is applied once, even when the store sends it again.
#[utoipa::path(
    post,
    path = "/store/notifications/{store}",
    extensions(("x-public" = json!(true))),
    operation_id = "receive_store_notification",
    params(("store" = String, Path, description = "`app_store` or `google_play`.")),
    request_body(content = String, description = "The store's notification, as sent.", content_type = "application/json"),
    responses(
        (status = 204, description = "Applied (or already applied)."),
        (status = 401, description = "The signature is not valid; code `store_signature_invalid`.", body = Problem),
        (status = 404, description = "No such store; code `plans.store_unknown`.", body = Problem),
        (status = 503, description = "This store is not set up on the server; code `plans.store_unavailable`.", body = Problem),
    )
)]
async fn receive_store_notification(
    State(service): State<Arc<PlansService>>,
    Path(store): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, Error> {
    service.store_notification(store_of(&store)?, &headers, &body).await?;
    Ok(StatusCode::NO_CONTENT)
}
