//! Chủ đề theo lịch (bước 67, D-234): bản chủ đề, thư viện chủ đề mẫu, xem trước giao diện của website
//! ở một thời điểm. Logic ở `theme/`.

use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, Pagination};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{Site, TenantContext};
use autoxide_kernel::time::Timestamp;
use autoxide_kernel::version::{IfMatch, Tagged};
use autoxide_spike_macros::Schema as ToSchema;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::IntoParams;
use uuid::Uuid;
use validator::Validate;

use super::page_request;
use crate::service::{ContentService, now};
use crate::theme::tokens::ContrastWarning;
use crate::theme::variants::{Variant, VariantInput, overrides_of};
use crate::theme::{SiteTheme, presets};

/// A theme variant: part of a website's theme swapped in on a schedule (a holiday, a season, an hour).
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ThemeVariantResponse {
    id: Uuid,
    /// The website (organization unit). Variants of a unit also apply to the websites below it.
    org_id: Uuid,
    name: String,
    /// Theme tokens this variant overrides (same names and rules as `PUT /themes/{org_id}`).
    #[schema(value_type = Object)]
    overrides: BTreeMap<String, Value>,
    /// Decoration the website may draw: `snow`, `petals`, `lanterns`, `fireworks`, `confetti`,
    /// `hearts`; `null`: none. Never on cart, checkout, account or sign-in pages, and off when the
    /// visitor asks for reduced motion (`docs/PAGES.md`).
    effect: Option<String>,
    /// `low` or `medium`.
    effect_intensity: Option<String>,
    /// `{"kind": "once", "starts_at", "ends_at"}` (UTC), `{"kind": "daily", "start": "HH:MM", "end":
    /// "HH:MM", "from"?, "until"?}` or `{"kind": "weekly", …, "days": ["sat", "sun"]}`; times are local
    /// to the website (`organizations.timezone`), an `end` before `start` runs past midnight.
    #[schema(value_type = Object)]
    schedule: Value,
    /// When several variants run at once, the highest wins (then the one that started later).
    priority: i32,
    /// `draft`, `active` or `paused`. Only `active` variants are shown.
    status: String,
    /// The preset it came from, if any.
    preset_key: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    updated_at: Timestamp,
}

pub(crate) fn variant_response(row: Variant) -> ThemeVariantResponse {
    ThemeVariantResponse {
        overrides: overrides_of(&row),
        schedule: serde_json::from_str(&row.schedule).unwrap_or(Value::Null),
        id: row.id,
        org_id: row.org_id,
        name: row.name,
        effect: row.effect,
        effect_intensity: row.effect_intensity,
        priority: row.priority,
        status: row.status,
        preset_key: row.preset_key,
        updated_at: row.updated_at,
    }
}

fn tagged(row: Variant) -> Tagged<ThemeVariantResponse> {
    Tagged { version: row.updated_at, data: variant_response(row) }
}

/// A theme variant to create or replace.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ThemeVariantRequest {
    /// The website (create only); left out: the whole tenant.
    org_id: Option<Uuid>,
    /// 1 to 120 characters; left out with `preset_key`: the preset's name.
    name: Option<String>,
    /// Tokens to override; left out with `preset_key`: the preset's.
    #[schema(value_type = Option<Object>)]
    overrides: Option<Map<String, Value>>,
    /// One of `snow`, `petals`, `lanterns`, `fireworks`, `confetti`, `hearts`.
    effect: Option<String>,
    /// `low` (default) or `medium`.
    effect_intensity: Option<String>,
    /// See `ThemeVariantResponse.schedule`; left out with `preset_key`: the preset's dates for `year`.
    #[schema(value_type = Option<Object>)]
    schedule: Option<Value>,
    /// 0 to 1000, default 0.
    priority: Option<i32>,
    /// Start from a preset of `GET /theme-presets` (create only).
    preset_key: Option<String>,
    /// Year of the preset's dates; default: this year in the website's time zone.
    year: Option<i16>,
}

impl From<ThemeVariantRequest> for VariantInput {
    fn from(input: ThemeVariantRequest) -> Self {
        VariantInput {
            org_id: input.org_id,
            name: input.name,
            overrides: input.overrides,
            effect: input.effect,
            effect_intensity: input.effect_intensity,
            schedule: input.schedule,
            priority: input.priority,
            preset_key: input.preset_key,
            year: input.year,
        }
    }
}

/// Which website.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct OrgQuery {
    /// The website (organization unit); left out: the whole tenant.
    org_id: Option<Uuid>,
}

/// List the theme variants of a website.
///
/// Requires `content.themes.manage` at the unit. Variants of units above it apply too but are listed
/// at their own unit.
#[utoipa::path(
    get,
    path = "/theme-variants",
    extensions(("x-permissions" = json!(["content.themes.manage"]))),
    operation_id = "list_content_theme_variants",
    security(("bearer" = [])),
    params(OrgQuery, PageQuery),
    responses(
        (status = 200, description = "A page of variants.", body = Page<ThemeVariantResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.themes.manage` at the unit; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 404, description = "No unit has this id; code `site_not_found`.", body = Problem),
        (status = 422, description = "A paging value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_theme_variants(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<OrgQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<ThemeVariantResponse>, Error> {
    let page = page_request(paging, Vec::new())?;
    let org =
        query.org_id.unwrap_or_else(|| autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid());
    Ok(service.list_variants(&tenant, &principal, org, &page).await?.map(variant_response))
}

/// Create a theme variant (as a draft).
///
/// Requires `content.themes.manage` at the unit. Turn it on with `POST /theme-variants/{id}/activate`.
/// Start from a preset with `preset_key` (and `year` for its dates). Every invalid value is listed at
/// once; at most `AUTOXIDE_CONTENT_THEME_VARIANTS_MAX` variants per website.
#[utoipa::path(
    post,
    path = "/theme-variants",
    extensions(("x-permissions" = json!(["content.themes.manage"]))),
    operation_id = "create_content_theme_variant",
    security(("bearer" = [])),
    request_body = ThemeVariantRequest,
    responses(
        (status = 201, description = "Created as a draft.", body = Data<ThemeVariantResponse>, headers(("Location" = String, description = "Address of the new variant."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.themes.manage` at the unit; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 404, description = "No unit has this id; code `site_not_found`.", body = Problem),
        (status = 409, description = "The website has the most variants allowed; code `content.theme_variants_limit`.", body = Problem),
        (status = 422, description = "A value is invalid (schedule, token, font, effect, preset); see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_theme_variant(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ThemeVariantRequest>,
) -> Result<Created<ThemeVariantResponse>, Error> {
    let row = service.create_variant(&tenant, &principal, input.into()).await?;
    let location = format!("/api/v1/content/theme-variants/{}", row.id);
    Ok(Created::new(location, variant_response(row)))
}

/// Get a theme variant.
///
/// Requires `content.themes.manage` at its unit.
#[utoipa::path(
    get,
    path = "/theme-variants/{id}",
    extensions(("x-permissions" = json!(["content.themes.manage"]))),
    operation_id = "get_content_theme_variant",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the variant.")),
    responses(
        (status = 200, description = "The variant.", body = Data<ThemeVariantResponse>, headers(("ETag" = String, description = "Version: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.themes.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No variant has this id; code `content.theme_variant_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_theme_variant(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<ThemeVariantResponse>, Error> {
    Ok(tagged(service.variant(&tenant, &principal, id).await?))
}

/// Replace a theme variant.
///
/// Requires `content.themes.manage` at its unit; `If-Match` is required. Keeps its status; `org_id`
/// and `preset_key` are ignored.
#[utoipa::path(
    put,
    path = "/theme-variants/{id}",
    extensions(("x-permissions" = json!(["content.themes.manage"]))),
    operation_id = "put_content_theme_variant",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the variant."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`). Otherwise `412`.")),
    request_body = ThemeVariantRequest,
    responses(
        (status = 200, description = "The variant after the change.", body = Data<ThemeVariantResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.themes.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No variant has this id; code `content.theme_variant_not_found`.", body = Problem),
        (status = 412, description = "The variant changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn put_theme_variant(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<ThemeVariantRequest>,
) -> Result<Tagged<ThemeVariantResponse>, Error> {
    let expected = if_match.required()?;
    let input = VariantInput { org_id: None, preset_key: None, ..input.into() };
    Ok(tagged(service.put_variant(&tenant, &principal, id, input, expected).await?))
}

/// Delete a theme variant.
///
/// Requires `content.themes.manage` at its unit; `If-Match` is required. Pause an active variant
/// first (`409`).
#[utoipa::path(
    delete,
    path = "/theme-variants/{id}",
    extensions(("x-permissions" = json!(["content.themes.manage"]))),
    operation_id = "delete_content_theme_variant",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the variant."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.themes.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No variant has this id; code `content.theme_variant_not_found`.", body = Problem),
        (status = 409, description = "The variant is on; code `content.theme_variant_active`.", body = Problem),
        (status = 412, description = "The variant changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn delete_theme_variant(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<StatusCode, Error> {
    let expected = if_match.required()?;
    service.delete_variant(&tenant, &principal, id, expected).await?;
    Ok(StatusCode::NO_CONTENT)
}

macro_rules! status_route {
    ($name:ident, $path:literal, $operation:literal, $status:literal, $summary:literal) => {
        #[doc = $summary]
        ///
        /// Requires `content.themes.manage` at its unit; `If-Match` is required.
        #[utoipa::path(
            post,
            path = $path,
            extensions(("x-permissions" = json!(["content.themes.manage"]))),
            operation_id = $operation,
            security(("bearer" = [])),
            params(("id" = Uuid, Path, description = "Id of the variant."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
            responses(
                (status = 200, description = "The variant after the change.", body = Data<ThemeVariantResponse>, headers(("ETag" = String, description = "New version."))),
                (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
                (status = 403, description = "Missing permission `content.themes.manage`; code `permission_denied`.", body = Problem),
                (status = 404, description = "No variant has this id; code `content.theme_variant_not_found`.", body = Problem),
                (status = 412, description = "The variant changed since you read it; code `precondition_failed`.", body = Problem),
                (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
            )
        )]
        pub(crate) async fn $name(
            State(service): State<Arc<ContentService>>,
            tenant: TenantContext,
            principal: Principal,
            Path(id): Path<Uuid>,
            if_match: IfMatch,
        ) -> Result<Tagged<ThemeVariantResponse>, Error> {
            let expected = if_match.required()?;
            Ok(tagged(service.set_variant_status(&tenant, &principal, id, $status, expected).await?))
        }
    };
}

status_route!(
    activate_theme_variant,
    "/theme-variants/{id}/activate",
    "activate_content_theme_variant",
    "active",
    "Turn a theme variant on: it shows whenever its schedule runs."
);
status_route!(
    pause_theme_variant,
    "/theme-variants/{id}/pause",
    "pause_content_theme_variant",
    "paused",
    "Pause a theme variant: it no longer shows until turned on again."
);

/// A copy with a new schedule.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CopyVariantRequest {
    /// The copy's schedule (see `ThemeVariantResponse.schedule`).
    #[schema(value_type = Object)]
    schedule: Value,
}

/// Copy a theme variant with a new schedule (e.g. to next year).
///
/// Requires `content.themes.manage` at its unit. The copy is a draft with the same overrides,
/// effect and priority.
#[utoipa::path(
    post,
    path = "/theme-variants/{id}/copy",
    extensions(("x-permissions" = json!(["content.themes.manage"]))),
    operation_id = "copy_content_theme_variant",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the variant to copy.")),
    request_body = CopyVariantRequest,
    responses(
        (status = 201, description = "The copy, a draft.", body = Data<ThemeVariantResponse>, headers(("Location" = String, description = "Address of the copy."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.themes.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No variant has this id; code `content.theme_variant_not_found`.", body = Problem),
        (status = 409, description = "The website has the most variants allowed; code `content.theme_variants_limit`.", body = Problem),
        (status = 422, description = "The schedule is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn copy_theme_variant(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CopyVariantRequest>,
) -> Result<Created<ThemeVariantResponse>, Error> {
    let row = service.copy_variant(&tenant, &principal, id, input.schedule).await?;
    let location = format!("/api/v1/content/theme-variants/{}", row.id);
    Ok(Created::new(location, variant_response(row)))
}

/// A ready-made theme variant shipped with the server.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ThemePresetResponse {
    /// Stable id of the preset in this list (for paging).
    id: Uuid,
    /// Give it as `preset_key` to `POST /theme-variants`.
    key: String,
    /// Name by language (`en`, `vi`).
    #[schema(value_type = Object)]
    name: BTreeMap<String, String>,
    #[schema(value_type = Object)]
    overrides: Map<String, Value>,
    effect: Option<String>,
    effect_intensity: Option<String>,
    /// Suggested local dates by year (`{"2027": ["2027-01-27", "2027-02-14"]}`); holidays on a fixed
    /// day give the same month and day every year. Empty for an every-day preset.
    #[schema(value_type = Object)]
    dates: BTreeMap<String, [String; 2]>,
    /// Schedule of an every-day or every-week preset (e.g. golden hour); `null` otherwise.
    #[schema(value_type = Option<Object>)]
    schedule: Option<Value>,
}

/// List the theme presets shipped with the server.
///
/// Requires `content.themes.manage`. Lunar holidays (Lunar New Year, Mid-Autumn) move every year; their dates
/// are data that ships with each release.
#[utoipa::path(
    get,
    path = "/theme-presets",
    extensions(("x-permissions" = json!(["content.themes.manage"]))),
    operation_id = "list_content_theme_presets",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "A page of presets.", body = Page<ThemePresetResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.themes.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_theme_presets(
    principal: Principal,
    Query(paging): Query<PageQuery>,
) -> Result<Page<ThemePresetResponse>, Error> {
    principal.require(&crate::permissions::CONTENT_THEMES)?;
    let page = page_request(paging, Vec::new())?;
    let year = now().date_utc().year();
    let all: Vec<ThemePresetResponse> = presets::all()
        .iter()
        .enumerate()
        .map(|(index, preset)| {
            let mut dates = preset.dates.clone();
            if let (Some([first, last]), true) = (&preset.yearly, dates.is_empty()) {
                for year in year..year + 3 {
                    dates.insert(
                        year.to_string(),
                        [format!("{year}-{first}"), format!("{year}-{last}")],
                    );
                }
            }
            ThemePresetResponse {
                id: Uuid::from_u128(index as u128 + 1),
                key: preset.key.clone(),
                name: preset.name.clone(),
                overrides: preset.overrides.clone(),
                effect: preset.effect.clone(),
                effect_intensity: preset.effect_intensity.clone(),
                dates,
                schedule: preset.schedule.clone(),
            }
        })
        .collect();
    // Danh sách cố định nhỏ: phân trang trong bộ nhớ theo `id` cố định.
    let after = page.cursor();
    let mut rows: Vec<ThemePresetResponse> =
        all.into_iter().filter(|row| after.is_none_or(|after| row.id > after)).collect();
    let limit = page.limit() as usize;
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next = more.then(|| rows.last().map(|row| row.id.to_string())).flatten();
    Ok(Page { data: rows, pagination: Pagination::new(page.limit(), next) })
}

/// The look of a website at a moment.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct SiteThemeResponse {
    /// Every token the website uses at that moment: its theme, then the running variant's overrides.
    #[schema(value_type = Object)]
    tokens: BTreeMap<String, Value>,
    effect: Option<String>,
    effect_intensity: Option<String>,
    /// The running variant (`id`, `name`); `null`: the base theme.
    #[schema(value_type = Option<Object>)]
    variant: Option<Value>,
    /// Next start or end of a variant within 7 days; `null`: none.
    #[schema(value_type = Option<String>, format = DateTime)]
    next_change_at: Option<Timestamp>,
    warnings: Vec<ContrastWarning>,
}

pub(crate) fn site_theme_response(theme: SiteTheme) -> SiteThemeResponse {
    SiteThemeResponse {
        tokens: theme.tokens,
        effect: theme.effect,
        effect_intensity: theme.effect_intensity,
        variant: theme.variant.map(|row| serde_json::json!({"id": row.id, "name": row.name})),
        next_change_at: theme.next_change_at,
        warnings: theme.warnings,
    }
}

/// When to preview.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PreviewQuery {
    /// The moment to show (RFC 3339); default: now.
    #[param(value_type = Option<String>, format = DateTime)]
    at: Option<Timestamp>,
    /// A variant to show as if it were on (even a draft).
    variant_id: Option<Uuid>,
}

/// Preview the look of a website at a moment.
///
/// Requires `content.themes.manage` at the unit. Shows which variant would run at `at` and the tokens
/// the website would use, optionally with one more variant (a draft) as if it were on.
#[utoipa::path(
    get,
    path = "/site/{org_id}/preview",
    extensions(("x-permissions" = json!(["content.themes.manage"]))),
    operation_id = "preview_content_site_theme",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "The website (organization unit)."), PreviewQuery),
    responses(
        (status = 200, description = "The look at that moment.", body = Data<SiteThemeResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.themes.manage` at the unit; code `permission_denied`.", body = Problem),
        (status = 404, description = "No unit or no variant has this id; code `site_not_found` or `content.theme_variant_not_found`.", body = Problem),
    )
)]
pub(crate) async fn preview_site_theme(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
    Query(query): Query<PreviewQuery>,
) -> Result<Data<SiteThemeResponse>, Error> {
    let site: Site = service.themes_site(&tenant, &principal, org_id).await?;
    let preview = match query.variant_id {
        Some(id) => Some(service.variant(&tenant, &principal, id).await?),
        None => None,
    };
    let at = query.at.unwrap_or_else(now);
    let theme = service.site_theme(&tenant, &site, at, preview.as_ref()).await?;
    Ok(Data { data: site_theme_response(theme) })
}
