//! Cấu hình đang dùng của module, cho trang cài đặt của giao diện (đề bài giao diện B7, D-269).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::service::NewsletterService;

/// The newsletter settings in use for this tenant (the tenant's own settings, otherwise the
/// server's; no secrets).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct NewsletterSettingsResponse {
    /// Put at the start of the subject of advertising campaigns
    /// (`AUTOXIDE_NEWSLETTER_ADVERTISING_LABEL`); empty: no label.
    #[schema(example = "[QC]")]
    advertising_label: String,
    /// Advertising mails one address gets at most per day
    /// (`AUTOXIDE_NEWSLETTER_ADVERTISING_MAX_PER_DAY`); `0`: no limit.
    #[schema(example = 3)]
    advertising_max_per_day: u32,
    /// The sender's postal address shown in every mail footer: the tenant setting
    /// `newsletter.postal_address`, otherwise the owner's name and address from the tenant's
    /// legal profile, otherwise `AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS`; `null`: none — advertising
    /// campaigns cannot be sent until one is set.
    postal_address: Option<String>,
    /// The server's public address is set (`AUTOXIDE_HTTP_PUBLIC_URL`); needed to send any
    /// campaign (the one-click unsubscribe link).
    public_url_set: bool,
    /// The website's preferences page is set (tenant setting `newsletter.link.preferences` or
    /// `AUTOXIDE_NEWSLETTER_PREFERENCES_URL`); needed to send any campaign (the footer link).
    preferences_url_set: bool,
    /// The website's confirmation page is set (tenant setting `newsletter.link.confirm` or
    /// `AUTOXIDE_NEWSLETTER_CONFIRM_URL`); needed by lists with double opt-in.
    confirm_url_set: bool,
    /// Campaign mails handed to the mail queue per minute
    /// (`AUTOXIDE_NEWSLETTER_SEND_RATE_PER_MINUTE`).
    #[schema(example = 300)]
    send_rate_per_minute: u32,
    /// Settings still to set before campaigns can be sent: names of the server's environment
    /// variables, empty when everything needed is set (`AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS` is
    /// listed: only advertising campaigns need it). Each can instead be set for this tenant
    /// alone with the tenant setting named in its field above.
    #[schema(example = json!(["AUTOXIDE_NEWSLETTER_PREFERENCES_URL"]))]
    missing: Vec<String>,
}

/// Get the newsletter settings in use.
///
/// Requires `newsletter.read`. The advertising label and daily limit, the sender's postal
/// address, and whether the addresses campaign mails need are set — so an interface can say
/// what to set before sending. The tenant changes its own with `PUT /api/v1/kernel/settings/{name}`
/// (`newsletter.postal_address`, `newsletter.link.*`); the operator sets the defaults in the
/// server's environment.
#[utoipa::path(
    get,
    path = "/settings",
    extensions(("x-permissions" = json!(["newsletter.read"]))),
    operation_id = "get_newsletter_settings",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The settings in use.", body = Data<NewsletterSettingsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.read`; code `permission_denied`.", body = Problem),
    )
)]
pub(crate) async fn get_settings(
    State(service): State<Arc<NewsletterService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<NewsletterSettingsResponse>, Error> {
    let settings = service.settings(&tenant, &principal).await?;
    let mut missing = Vec::new();
    for (set, name) in [
        (settings.public_url_set, "AUTOXIDE_HTTP_PUBLIC_URL"),
        (settings.preferences_url_set, "AUTOXIDE_NEWSLETTER_PREFERENCES_URL"),
        (settings.confirm_url_set, "AUTOXIDE_NEWSLETTER_CONFIRM_URL"),
        (settings.postal_address.is_some(), "AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS"),
    ] {
        if !set {
            missing.push(name.to_owned());
        }
    }
    Ok(Data {
        data: NewsletterSettingsResponse {
            advertising_label: settings.advertising_label,
            advertising_max_per_day: settings.advertising_max_per_day,
            postal_address: settings.postal_address,
            public_url_set: settings.public_url_set,
            preferences_url_set: settings.preferences_url_set,
            confirm_url_set: settings.confirm_url_set,
            send_rate_per_minute: settings.send_rate_per_minute,
            missing,
        },
    })
}
