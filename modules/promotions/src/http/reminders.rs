//! "Nhắc tôi" khi flash sale bắt đầu (D-225).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::reminders::ReminderView;
use crate::service::PromotionsService;

pub(crate) fn routes(service: Arc<PromotionsService>) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(remind_me, forget_reminder)).with_state(service)
}

/// Ask to be reminded when a flash sale starts.
///
/// For buyer accounts; a staff account needs `promotions.use`. Only public flash sales that have
/// not started. One reminder is sent
/// a few minutes before the start (in-app, push or email, as your notification settings allow).
/// Asking is your consent to this one reminder only — not to advertising. Asking again returns the
/// reminder you have (200).
#[utoipa::path(
    post,
    path = "/public/promotions/{id}/reminders",
    extensions(("x-permissions" = json!(["promotions.use"])), ("x-buyers" = json!(true))),
    operation_id = "create_flash_sale_reminder",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the flash sale.")),
    responses(
        (status = 201, description = "You will be reminded.", body = Data<ReminderView>, headers(("Location" = String, description = "Path of the reminder."))),
        (status = 200, description = "You already asked.", body = Data<ReminderView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "A staff account without `promotions.use`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such public flash sale; code `promotions.not_found`.", body = Problem),
        (status = 409, description = "It has already started (`promotions.flash_sale_started`) or you have too many reminders waiting (`promotions.too_many_reminders`).", body = Problem),
    )
)]
async fn remind_me(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Response, Error> {
    super::require_use(&principal)?;
    let location = format!("/api/v1/promotions/public/promotions/{id}/reminders");
    Ok(match service.remind_me(&tenant, &principal, id).await? {
        Submitted::Created(view) => Created::new(location, view).into_response(),
        Submitted::Replayed(view) => Data { data: view }.into_response(),
    })
}

/// Cancel your reminder of a flash sale.
///
/// For buyer accounts; a staff account needs `promotions.use`. Removes the reminder (if not sent
/// yet) and withdraws the consent
/// given for it. Nothing to remove is fine (204).
#[utoipa::path(
    delete,
    path = "/public/promotions/{id}/reminders",
    extensions(("x-permissions" = json!(["promotions.use"])), ("x-buyers" = json!(true))),
    operation_id = "delete_flash_sale_reminder",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the flash sale.")),
    responses(
        (status = 204, description = "No reminder is left."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "A staff account without `promotions.use`; code `permission_denied`.", body = Problem),
    )
)]
async fn forget_reminder(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    super::require_use(&principal)?;
    service.forget_reminder(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
