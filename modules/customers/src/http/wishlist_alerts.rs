//! `/me/wishlist/alerts`: người mua tự bật báo khi sản phẩm yêu thích có hàng lại hay giảm giá (D-233).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use validator::Validate;

use crate::service::CustomersService;
use crate::wishlist_alerts::{AlertChoice, AlertState};

/// Your wishlist alert settings.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct WishlistAlerts {
    /// Hear when a wishlist product is back in stock.
    pub back_in_stock: bool,
    /// Hear when a wishlist product is really cheaper (at least the shop's minimum drop since you
    /// added it, never against a crossed-out price).
    pub price_drop: bool,
    /// Whether this shop can send back-in-stock alerts at all.
    pub back_in_stock_available: bool,
}

impl From<AlertState> for WishlistAlerts {
    fn from(state: AlertState) -> Self {
        WishlistAlerts {
            back_in_stock: state.back_in_stock,
            price_drop: state.price_drop,
            back_in_stock_available: state.back_in_stock_available,
        }
    }
}

/// Turn wishlist alerts on or off. Turning any on needs your consent: send the consent text and its
/// version exactly as shown to you. Turning both off withdraws the consent.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetWishlistAlertsRequest {
    back_in_stock: bool,
    price_drop: bool,
    /// The consent sentence shown to the buyer (up to 2000 characters); required to turn on.
    consent_text: Option<String>,
    /// Version of that sentence (up to 50 characters); required to turn on.
    consent_version: Option<String>,
}

/// Get your wishlist alert settings.
///
/// Any signed-in buyer. Both alerts are off until you turn them on.
#[utoipa::path(
    get, path = "/me/wishlist/alerts", extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_wishlist_alerts", security(("bearer" = [])),
    responses(
        (status = 200, description = "Your settings.", body = Data<WishlistAlerts>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
pub(crate) async fn get_wishlist_alerts(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<WishlistAlerts>, Error> {
    Ok(Data { data: service.wishlist_alerts(&tenant, principal.user_id()).await?.into() })
}

/// Turn wishlist alerts on or off.
///
/// Any signed-in buyer, when the shop's plan has wishlist alerts. Alerts are advertising: at most one
/// message a day by default with up to 10 products, only in the allowed hours, and each product at
/// most once every 7 days for each kind. Every message has a link to stop them.
#[utoipa::path(
    put, path = "/me/wishlist/alerts", extensions(("x-permissions" = json!([]))),
    operation_id = "set_my_wishlist_alerts", security(("bearer" = [])), request_body = SetWishlistAlertsRequest,
    responses(
        (status = 200, description = "Your settings after the change.", body = Data<WishlistAlerts>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The shop's plan has no wishlist alerts; code `feature_not_enabled`.", body = Problem),
        (status = 422, description = "Consent missing (`customers.wishlist_alert_consent_required`), the shop cannot send this kind (`customers.wishlist_alert_unavailable`), or a value is invalid (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn set_wishlist_alerts(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<SetWishlistAlertsRequest>,
) -> Result<Data<WishlistAlerts>, Error> {
    let choice = AlertChoice {
        back_in_stock: input.back_in_stock,
        price_drop: input.price_drop,
        consent_text: input.consent_text,
        consent_version: input.consent_version,
    };
    Ok(Data {
        data: service.set_wishlist_alerts(&tenant, principal.user_id(), choice).await?.into(),
    })
}
