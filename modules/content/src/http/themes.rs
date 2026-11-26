//! Bộ giao diện của website (bước 67): `GET` / `PUT /themes/{org_id}`. Logic ở `theme/`.

use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::Problem;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use autoxide_spike_macros::Schema as ToSchema;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;
use validator::Validate;

use crate::service::ContentService;
use crate::theme::ThemeView;
use crate::theme::tokens::ContrastWarning;

/// The theme of a website: what this unit sets, and what its website ends up with.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ThemeResponse {
    /// The organization unit (website); the root unit is the whole tenant.
    org_id: Uuid,
    /// Tokens this unit sets itself, overriding the units above it. Empty: it inherits all.
    #[schema(value_type = Object)]
    tokens: BTreeMap<String, Value>,
    /// Every token the website uses: the server defaults, then the root unit's tokens, … then
    /// this unit's. Flat names, ready to become CSS variables (`color.primary` → `--color-primary`).
    #[schema(value_type = Object, example = json!({"color.primary": "#1d4ed8", "font.body": "Be Vietnam Pro", "radius": "medium"}))]
    effective: BTreeMap<String, Value>,
    /// Text and background colors of `effective` below the WCAG AA contrast of 4.5:1. A warning:
    /// the theme is saved anyway.
    warnings: Vec<ContrastWarning>,
}

fn respond(view: ThemeView) -> Tagged<ThemeResponse> {
    Tagged {
        version: view.version,
        data: ThemeResponse {
            org_id: view.org_id,
            tokens: view.tokens,
            effective: view.effective,
            warnings: view.warnings,
        },
    }
}

/// The tokens a unit sets.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ThemeRequest {
    /// Replaces every token this unit sets; a token left out or `null` is inherited. Colors
    /// (`color.primary`, `color.on_primary`, `color.secondary`, `color.on_secondary`,
    /// `color.background`, `color.surface`, `color.text`, `color.muted`, `color.success`,
    /// `color.error`) are `#rrggbb`; `font.heading` / `font.body` one of the allowed fonts
    /// (`system`, `Inter`, `Roboto`, `Open Sans`, `Noto Sans`, `Noto Serif`, `Be Vietnam Pro`,
    /// `Montserrat`, `Lora`, `Merriweather`, `Playfair Display`, `Source Sans 3`);
    /// `font.size_base` 12–24 (px); `content_width` 640–1920 (px); `radius` `none` / `small` /
    /// `medium` / `large` / `full`; `spacing` `compact` / `normal` / `relaxed`; `button.style`
    /// `solid` / `outline` / `soft`; `logo_media_id` / `background_media_id` a public media item.
    #[schema(value_type = Object, example = json!({"color.primary": "#b91c1c", "font.heading": "Playfair Display"}))]
    pub tokens: Map<String, Value>,
}

/// Get the theme of a website.
///
/// Requires `content.themes.manage` at the unit. `effective` is what the website shows, after
/// inheriting from the units above.
#[utoipa::path(
    get,
    path = "/themes/{org_id}",
    extensions(("x-permissions" = json!(["content.themes.manage"]))),
    operation_id = "get_content_theme",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "The organization unit (website); the root unit for the whole tenant.")),
    responses(
        (status = 200, description = "The theme.", body = autoxide_kernel::response::Data<ThemeResponse>, headers(("ETag" = String, description = "Version of this unit's tokens: send it back in `If-Match` when changing them."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.themes.manage` (code `permission_denied`), or the unit is out of your reach (`org_out_of_scope`).", body = Problem),
        (status = 404, description = "No unit has this id; code `site_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_theme(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
) -> Result<Tagged<ThemeResponse>, Error> {
    Ok(respond(service.theme(&tenant, &principal, org_id).await?))
}

/// Replace the theme tokens of a website.
///
/// Requires `content.themes.manage` at the unit. `If-Match` is required (the `ETag` of `GET`, or
/// `*`). Every invalid token is listed at once in `errors`; low contrast only gives `warnings`.
#[utoipa::path(
    put,
    path = "/themes/{org_id}",
    extensions(("x-permissions" = json!(["content.themes.manage"]))),
    operation_id = "put_content_theme",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "The organization unit (website); the root unit for the whole tenant."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`). Otherwise `412`.")),
    request_body = ThemeRequest,
    responses(
        (status = 200, description = "The theme after the change.", body = autoxide_kernel::response::Data<ThemeResponse>, headers(("ETag" = String, description = "New version of this unit's tokens."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.themes.manage` (code `permission_denied`), or the unit is out of your reach (`org_out_of_scope`).", body = Problem),
        (status = 404, description = "No unit has this id; code `site_not_found`.", body = Problem),
        (status = 412, description = "The theme changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A token is unknown or its value invalid (font not allowed, color not `#rrggbb`, media item missing or not public); see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn put_theme(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<ThemeRequest>,
) -> Result<Tagged<ThemeResponse>, Error> {
    let expected = if_match.required()?;
    let view = service.put_theme(&tenant, &principal, org_id, &input.tokens, expected).await?;
    Ok(respond(view))
}
