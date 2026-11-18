//! Tài khoản hãng (`shipping.carriers.manage`) và webhook công khai của hãng.

use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::carriers::{AccountDraft, AccountView};
use crate::permissions::CARRIERS_MANAGE;
use crate::service::ShippingService;

/// A new carrier account.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = NewShippingCarrierAccount)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewAccountRequest {
    /// Organization unit owning the account; defaults to where you work when that is one unit.
    org_id: Option<Uuid>,
    /// Code of an installed carrier connector, e.g. `ghn`.
    #[schema(example = "ghn")]
    carrier_code: String,
    /// Your name for the account (1 to 200 characters).
    name: String,
    /// Login fields the connector needs (e.g. `token`, `shop_id`, `webhook_secret`). Stored
    /// encrypted; never returned.
    credentials: BTreeMap<String, String>,
    /// Settings that are not secret (pickup warehouse at the carrier…).
    #[serde(default)]
    settings: Option<serde_json::Value>,
    /// Test account of the carrier.
    #[serde(default)]
    sandbox: bool,
}

/// Changes of a carrier account; leave out what stays. `credentials` replaces every field.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ShippingCarrierAccountChanges)]
#[serde(deny_unknown_fields)]
pub(crate) struct AccountChangesRequest {
    name: Option<String>,
    credentials: Option<BTreeMap<String, String>>,
    settings: Option<serde_json::Value>,
    sandbox: Option<bool>,
    active: Option<bool>,
}

/// A carrier account. Credentials are never returned: only the names of the fields set.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingCarrierAccount)]
pub(crate) struct AccountResponse {
    id: Uuid,
    org_id: Uuid,
    carrier_code: String,
    name: String,
    credential_fields: Vec<String>,
    settings: serde_json::Value,
    sandbox: bool,
    active: bool,
    /// `never`, `ok` or `failed`: result of the last connection test, or of the last call the
    /// carrier refused for bad credentials.
    check_status: String,
    checked_at: Option<DateTime<Utc>>,
    check_error_code: Option<String>,
    /// Only in the answer of the create call: the address the carrier posts status updates to.
    /// Keep it; it cannot be shown again.
    webhook_path: Option<String>,
    updated_at: DateTime<Utc>,
}

impl From<AccountView> for AccountResponse {
    fn from(view: AccountView) -> Self {
        let row = view.row;
        AccountResponse {
            webhook_path: view
                .webhook_token
                .map(|token| format!("/api/v1/shipping/webhooks/{}/{token}", row.carrier_code)),
            id: row.id,
            org_id: row.org_id,
            carrier_code: row.carrier_code,
            name: row.name,
            credential_fields: view.credential_fields,
            settings: serde_json::from_str(&row.settings).unwrap_or(serde_json::Value::Null),
            sandbox: row.sandbox,
            active: row.active,
            check_status: row.check_status,
            checked_at: row.checked_at,
            check_error_code: row.check_error_code,
            updated_at: row.updated_at,
        }
    }
}

/// List carrier accounts.
///
/// Needs `shipping.carriers.manage`.
#[utoipa::path(
    get,
    path = "/carrier-accounts",
    extensions(("x-permissions" = json!(["shipping.carriers.manage"]))),
    operation_id = "list_shipping_carrier_accounts",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of carrier accounts.", body = Page<AccountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.carriers.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_accounts(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<AccountResponse>, Error> {
    principal.require(&CARRIERS_MANAGE)?;
    let page = PageRequest::try_from(page)?;
    Ok(service.list_accounts(&tenant, &principal, &page).await?.map(AccountResponse::from))
}

/// Connect a carrier account.
///
/// Needs `shipping.carriers.manage` at the unit and the plan feature `shipping.carriers`. The
/// answer holds `webhook_path` once: give it to the carrier as the status webhook.
#[utoipa::path(
    post,
    path = "/carrier-accounts",
    extensions(("x-permissions" = json!(["shipping.carriers.manage"]))),
    operation_id = "create_shipping_carrier_account",
    security(("bearer" = [])),
    request_body = NewAccountRequest,
    responses(
        (status = 201, description = "Connected (not tested yet: `check_status` is `never`).", body = Data<AccountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.carriers.manage` at the unit (`permission_denied`, `org_out_of_scope`), the unit is not active (`org_frozen`), or the plan lacks the feature (`feature_not_enabled`).", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or no connector for this carrier is installed (`shipping.carrier_unknown`).", body = Problem),
    )
)]
pub(crate) async fn create_account(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<NewAccountRequest>,
) -> Result<Created<AccountResponse>, Error> {
    principal.require(&CARRIERS_MANAGE)?;
    let draft = AccountDraft {
        org_id: input.org_id,
        carrier_code: Some(input.carrier_code),
        name: Some(input.name),
        credentials: Some(input.credentials),
        settings: input.settings,
        sandbox: Some(input.sandbox),
        active: Some(true),
    };
    let view = service.create_account(&tenant, &principal, draft).await?;
    Ok(Created::new(format!("/shipping/carrier-accounts/{}", view.row.id), view.into()))
}

/// Change a carrier account.
#[utoipa::path(
    patch,
    path = "/carrier-accounts/{id}",
    extensions(("x-permissions" = json!(["shipping.carriers.manage"]))),
    operation_id = "update_shipping_carrier_account",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the carrier account.")),
    request_body = AccountChangesRequest,
    responses(
        (status = 200, description = "The account after the change.", body = Data<AccountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.carriers.manage` at the unit, or the unit is not active; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No carrier account with this id in your scope; code `shipping.carrier_account_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn update_account(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AccountChangesRequest>,
) -> Result<Data<AccountResponse>, Error> {
    principal.require(&CARRIERS_MANAGE)?;
    let draft = AccountDraft {
        name: input.name,
        credentials: input.credentials,
        settings: input.settings,
        sandbox: input.sandbox,
        active: input.active,
        ..AccountDraft::default()
    };
    Ok(Data { data: service.update_account(&tenant, &principal, id, draft).await?.into() })
}

/// Test a carrier account.
///
/// Asks the carrier whether the credentials work and records the answer in `check_status`.
#[utoipa::path(
    post,
    path = "/carrier-accounts/{id}/test",
    extensions(("x-permissions" = json!(["shipping.carriers.manage"]))),
    operation_id = "test_shipping_carrier_account",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the carrier account.")),
    responses(
        (status = 200, description = "The account with the result in `check_status` (`ok` or `failed` with `check_error_code`).", body = Data<AccountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.carriers.manage` at the unit; code `permission_denied`.", body = Problem),
        (status = 404, description = "No carrier account with this id in your scope; code `shipping.carrier_account_not_found`.", body = Problem),
        (status = 422, description = "The connector of this carrier is no longer installed; code `shipping.carrier_unknown`.", body = Problem),
    )
)]
pub(crate) async fn test_account(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<AccountResponse>, Error> {
    principal.require(&CARRIERS_MANAGE)?;
    Ok(Data { data: service.test_account(&tenant, &principal, id).await?.into() })
}

/// Receipt of a carrier webhook.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingWebhookReceipt)]
pub(crate) struct WebhookReceipt {
    id: Uuid,
}

/// Receive a status webhook from a carrier.
///
/// Public: the carrier calls the `webhook_path` given when the account was connected. The
/// signature is checked on the raw body; the body is stored as received and applied in the
/// background. Repeated or late updates are applied once, in order.
#[utoipa::path(
    post,
    path = "/webhooks/{carrier_code}/{token}",
    operation_id = "receive_shipping_webhook",
    extensions(("x-public" = json!(true))),
    security(()),
    params(
        ("carrier_code" = String, Path, description = "Code of the carrier."),
        ("token" = String, Path, description = "Token of the carrier account (from `webhook_path`)."),
    ),
    request_body(content = String, description = "Body as the carrier sends it.", content_type = "application/json"),
    responses(
        (status = 202, description = "Stored; it will be applied shortly.", body = Data<WebhookReceipt>),
        (status = 401, description = "The signature is not valid; nothing was stored. Code `shipping.webhook_signature`.", body = Problem),
        (status = 404, description = "No active account of this carrier has this token; code `shipping.webhook_unknown`.", body = Problem),
        (status = 413, description = "The body is larger than `AUTOXIDE_SHIPPING_WEBHOOK_MAX_BYTES`; code `shipping.webhook_too_large`.", body = Problem),
    )
)]
pub(crate) async fn receive_webhook(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    Path((carrier_code, token)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Data<WebhookReceipt>), Error> {
    let id = service.receive_webhook(&tenant, &carrier_code, &token, &headers, &body).await?;
    Ok((StatusCode::ACCEPTED, Data { data: WebhookReceipt { id } }))
}
