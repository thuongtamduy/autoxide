//! Báo khi có hàng lại, công khai (D-201): đăng ký, xác nhận, dừng. Xem `crate::alerts`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{OptionalTenant, TenantContext};
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::alerts::AlertRequest;
use crate::service::InventoryService;

pub(crate) fn routes(service: Arc<InventoryService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(subscribe_restock_alert))
        .routes(routes!(confirm_restock_alert))
        .routes(routes!(stop_restock_alert))
        .with_state(service)
}

/// A request for a back-in-stock alert.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestockAlertBody {
    pub variant_id: Uuid,
    /// The store (organization unit) to watch; omit it for the online shop.
    pub store_id: Option<Uuid>,
    /// Email to alert. Give this or `phone`; a signed-in buyer may give neither (alerted on their
    /// account).
    #[schema(example = "ada@example.com")]
    pub email: Option<String>,
    /// Phone number to alert by SMS (`0912345678` or `+84912345678`).
    pub phone: Option<String>,
    /// Must be `true`: the visitor agrees to get a message for this purpose only.
    pub consent: bool,
    /// The exact consent text shown next to the form (kept as proof), up to 2000 characters.
    pub consent_text: Option<String>,
    /// Version of that text, if you keep versions.
    #[validate(length(max = 50))]
    pub consent_version: Option<String>,
    /// Language of the messages: `en` or `vi`.
    pub locale: Option<String>,
    /// Token from `GET /api/v1/antispam/token?scope=inventory:restock_alerts`; needed when not
    /// signed in.
    pub form_token: Option<String>,
    /// The hidden trap field: people leave it empty.
    pub trap: Option<String>,
    /// The value of the `altcha` widget, when the server uses a challenge.
    pub challenge: Option<String>,
}

/// The answer to an alert request: always the same.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct RestockAlertAccepted {
    /// Always `true`. Visitors who are not signed in get a link to confirm.
    pub accepted: bool,
}

/// Ask for a back-in-stock alert (public).
///
/// No sign-in needed. A signed-in buyer who gives no email or phone is alerted on their account at
/// once. Others give an email or a phone number and get a link to confirm (the website page in
/// `AUTOXIDE_INVENTORY_RESTOCK_CONFIRM_URL`), then one message each time the item comes back in
/// stock at that store, with a link to stop. The answer is the same `202` whether the contact is
/// new, waiting, already confirmed or dropped by the anti-spam checks.
#[utoipa::path(
    post,
    path = "/public/restock-alerts",
    extensions(("x-public" = json!(true))),
    operation_id = "create_inventory_restock_alert",
    security((), ("bearer" = [])),
    request_body = RestockAlertBody,
    responses(
        (status = 202, description = "Received.", body = Data<RestockAlertAccepted>),
        (status = 400, description = "The form token or challenge is missing, invalid or expired (codes `antispam.token_missing`, `antispam.token_invalid`, `antispam.token_expired`, `antispam.challenge_required`, `antispam.challenge_failed`): reload the form.", body = Problem),
        (status = 422, description = "A value is invalid (no consent, no or bad contact, unknown language); see `errors`.", body = Problem),
        (status = 429, description = "Too many requests from this account; code `antispam.rate_limited`.", body = Problem),
        (status = 503, description = "Alerts are not set up on this server (code `inventory.restock_alerts_unavailable`) or the anti-spam check cannot run (`antispam.verifier_unavailable`).", body = Problem),
    )
)]
async fn subscribe_restock_alert(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Option<Principal>,
    client: ClientInfo,
    ValidJson(input): ValidJson<RestockAlertBody>,
) -> Result<(StatusCode, Data<RestockAlertAccepted>), Error> {
    let request = AlertRequest {
        variant_id: input.variant_id,
        store_id: input.store_id,
        email: input.email,
        phone: input.phone,
        consent: input.consent,
        consent_text: input.consent_text,
        consent_version: input.consent_version,
        locale: input.locale,
        form_token: input.form_token,
        trap: input.trap,
        challenge: input.challenge,
    };
    service.subscribe_restock(&tenant, principal.as_ref(), &client, request).await?;
    Ok((StatusCode::ACCEPTED, Data { data: RestockAlertAccepted { accepted: true } }))
}

/// The token of a link in an alert message.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestockTokenBody {
    /// The `{token}` of the link, exactly as received (it may contain a `.`).
    #[validate(length(min = 1, max = 200))]
    pub token: String,
}

/// The state of an alert after a link was used.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct RestockAlertState {
    /// `confirmed` or `stopped`.
    pub status: String,
}

/// Confirm a back-in-stock alert (public).
///
/// No sign-in needed. The confirmation message links to the website page; the page sends the
/// token here with `POST` (opening the link alone changes nothing). Confirming twice is fine.
#[utoipa::path(
    post,
    path = "/public/restock-alerts/confirm",
    extensions(("x-public" = json!(true))),
    operation_id = "confirm_inventory_restock_alert",
    security((), ("bearer" = [])),
    request_body = RestockTokenBody,
    responses(
        (status = 200, description = "Confirmed.", body = Data<RestockAlertState>),
        (status = 400, description = "The link is invalid or the alert was stopped; code `inventory.restock_token_invalid`.", body = Problem),
        (status = 422, description = "`token` is empty or too long; see `errors`.", body = Problem),
    )
)]
async fn confirm_restock_alert(
    State(service): State<Arc<InventoryService>>,
    tenant: OptionalTenant,
    _principal: Option<Principal>,
    ValidJson(input): ValidJson<RestockTokenBody>,
) -> Result<Data<RestockAlertState>, Error> {
    service.restock_token(tenant.0.as_ref(), &input.token, true).await?;
    Ok(Data { data: RestockAlertState { status: crate::alerts::CONFIRMED.to_owned() } })
}

/// Stop a back-in-stock alert (public).
///
/// No sign-in needed. Every alert message links to the website page with a token; the page sends
/// it here. The token of the confirmation message works too.
#[utoipa::path(
    post,
    path = "/public/restock-alerts/stop",
    extensions(("x-public" = json!(true))),
    operation_id = "stop_inventory_restock_alert",
    security((), ("bearer" = [])),
    request_body = RestockTokenBody,
    responses(
        (status = 200, description = "Stopped.", body = Data<RestockAlertState>),
        (status = 400, description = "The link is invalid or was already used; code `inventory.restock_token_invalid`.", body = Problem),
        (status = 422, description = "`token` is empty or too long; see `errors`.", body = Problem),
    )
)]
async fn stop_restock_alert(
    State(service): State<Arc<InventoryService>>,
    tenant: OptionalTenant,
    _principal: Option<Principal>,
    ValidJson(input): ValidJson<RestockTokenBody>,
) -> Result<Data<RestockAlertState>, Error> {
    service.restock_token(tenant.0.as_ref(), &input.token, false).await?;
    Ok(Data { data: RestockAlertState { status: crate::alerts::STOPPED.to_owned() } })
}
