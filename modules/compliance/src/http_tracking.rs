//! Route REST của mã theo dõi quảng cáo và banner đồng ý cookie (D-235, D-361) dưới
//! `/api/v1/compliance`, kèm hai endpoint công khai cho giao diện: cấu hình đồng ý (chỉ trả mã của nhóm
//! người xem đã đồng ý) và ghi bằng chứng đồng ý ẩn danh.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::i18n::Language;
use autoxide_kernel::mail::MailTemplate;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest, page_of};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{Site, SiteQuery, TenantContext};
use autoxide_kernel::version::Tagged;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::entity::tracking_tag;
use crate::permissions::TRACKING_MANAGE;
use crate::service::ComplianceService;
use crate::tracking::{Banner, BannerChange, PROVIDERS, provider};

pub(crate) fn routes(service: Arc<ComplianceService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_providers))
        .routes(routes!(list_tags, create_tag))
        .routes(routes!(get_tag, update_tag, delete_tag))
        .routes(routes!(get_cookie_settings, set_cookie_settings))
        .routes(routes!(cookie_stats))
        .routes(routes!(public_consent_config))
        .routes(routes!(record_cookie_consent))
        .with_state(service)
}

// Câu chữ của banner theo cặp (tiêu đề, nội dung) dùng `MailTemplate` để có bản dịch ở
// `locales/vi.json` (`<key>.subject` / `<key>.text`) và được `cargo xtask check` kiểm đủ — không phải
// mail. Bước 51 (P21) sẽ cho khách đổi các câu này, trừ câu `legal`.
const INTRO: MailTemplate = MailTemplate::new(
    "compliance.banner.intro",
    "We use cookies",
    "We use necessary cookies to run this site. With your consent we also use cookies to remember \
     your choices, measure visits and show relevant ads. Nothing optional runs before you choose.",
);
const CHOICE: MailTemplate =
    MailTemplate::new("compliance.banner.choice", "Accept all", "Reject all");
const ACTIONS: MailTemplate =
    MailTemplate::new("compliance.banner.actions", "Choose", "Save my choices");
const LEGAL: MailTemplate = MailTemplate::new(
    "compliance.banner.legal",
    "Your rights",
    "You can withdraw or change your consent at any time from the cookie settings link; rejecting is \
     as easy as accepting and does not limit the site.",
);
const NECESSARY: MailTemplate = MailTemplate::new(
    "compliance.banner.necessary",
    "Necessary",
    "Needed for the site to work (session, security, your cookie choice). Always on.",
);
const PREFERENCES: MailTemplate = MailTemplate::new(
    "compliance.banner.preferences",
    "Preferences",
    "Remember your choices such as language or region.",
);
const ANALYTICS: MailTemplate = MailTemplate::new(
    "compliance.banner.analytics",
    "Analytics",
    "Measure visits to improve the site.",
);
const MARKETING: MailTemplate = MailTemplate::new(
    "compliance.banner.marketing",
    "Marketing",
    "Measure ads and show offers that fit you, with advertising partners.",
);

/// A built-in tracking provider.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceTrackingProvider)]
pub(crate) struct ProviderResponse {
    /// Key to use in `provider`, e.g. `ga4`.
    pub key: String,
    pub name: String,
    /// Consent category it needs: `analytics` or `marketing`.
    pub category: String,
    /// Form of the tracking ID, e.g. `G-XXXXXXXXXX`.
    pub example: String,
    /// Hosts to allow in the Content-Security-Policy when it loads.
    pub csp_hosts: Vec<String>,
    /// Country that receives the data (ISO 3166-1 alpha-2).
    pub country: String,
}

/// A tracking tag of a website.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceTrackingTag)]
pub(crate) struct TagResponse {
    pub id: Uuid,
    /// The unit of the website (the root unit is the whole tenant).
    pub org_id: Uuid,
    pub provider: String,
    pub tracking_id: String,
    /// `active` or `paused`.
    pub status: String,
    /// Consent category it needs.
    pub category: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl From<tracking_tag::Model> for TagResponse {
    fn from(row: tracking_tag::Model) -> Self {
        let category = provider(&row.provider).map_or("marketing", |item| item.category).to_owned();
        TagResponse {
            id: row.id,
            org_id: row.org_id,
            provider: row.provider,
            tracking_id: row.tracking_id,
            status: row.status,
            category,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// The cookie banner of a website.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceCookieSettings)]
pub(crate) struct SettingsResponse {
    pub org_id: Uuid,
    /// `false`: no banner and no tag ever loads.
    pub enabled: bool,
    /// Optional categories the tenant turned on.
    pub categories: Vec<String>,
    /// Categories the banner asks about: those turned on plus those of active tags (`necessary` is
    /// always there and not listed).
    pub effective_categories: Vec<String>,
    /// Version of the cookie policy; changes when categories or active providers change, and
    /// visitors are asked again. `0` until the banner is set.
    pub policy_version: i32,
    /// `bottom`, `top` or `center`.
    pub position: String,
    pub updated_at: Timestamp,
}

impl From<Banner> for SettingsResponse {
    fn from(banner: Banner) -> Self {
        SettingsResponse {
            org_id: banner.org_id,
            enabled: banner.enabled,
            categories: banner.configured,
            effective_categories: banner.categories,
            policy_version: banner.policy_version,
            position: banner.position,
            updated_at: banner.updated_at,
        }
    }
}

/// List the built-in tracking providers.
///
/// Requires `compliance.tracking.manage`. Tenants only enter the tracking ID of these; no script.
/// By key (A→Z unless `order=desc`); paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/tracking-providers",
    extensions(("x-permissions" = json!(["compliance.tracking.manage"]))),
    operation_id = "list_compliance_tracking_providers",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the providers.", body = Page<ProviderResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.tracking.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_providers(
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<ProviderResponse>, Error> {
    principal.require(&TRACKING_MANAGE)?;
    let page = PageRequest::try_from(page)?;
    // Một trang (`page_of`): nhà cung cấp dựng sẵn trong code (`PROVIDERS`), không nằm trong database.
    page_of(providers(), |item| item.key.clone(), &page)
}

pub(crate) fn providers() -> Vec<ProviderResponse> {
    PROVIDERS
        .iter()
        .map(|item| ProviderResponse {
            key: item.key.to_owned(),
            name: item.name.to_owned(),
            category: item.category.to_owned(),
            example: item.example.to_owned(),
            csp_hosts: item.hosts.iter().map(|host| (*host).to_owned()).collect(),
            country: item.country.to_owned(),
        })
        .collect()
}

/// Which tags.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TagsQuery {
    /// Only the tags of this unit (website).
    org_id: Option<Uuid>,
}

/// A tag to add.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceTrackingTagRequest)]
pub(crate) struct TagRequest {
    /// The unit of the website; defaults to the root unit (the whole tenant).
    pub org_id: Option<Uuid>,
    /// `ga4`, `gtm`, `google_ads`, `meta`, `tiktok` or `zalo`.
    pub provider: String,
    /// The ID the provider gave, e.g. `G-XXXXXXXXXX`.
    pub tracking_id: String,
}

/// Changes to a tag.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceTrackingTagChange)]
pub(crate) struct TagChange {
    /// `active` or `paused`.
    pub status: Option<String>,
    pub tracking_id: Option<String>,
}

/// List the tracking tags of websites.
///
/// Requires `compliance.tracking.manage`; only units in your scope.
#[utoipa::path(
    get,
    path = "/tracking-tags",
    extensions(("x-permissions" = json!(["compliance.tracking.manage"]))),
    operation_id = "list_compliance_tracking_tags",
    security(("bearer" = [])),
    params(TagsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of tags.", body = Page<TagResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.tracking.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_tags(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<TagsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<TagResponse>, Error> {
    principal.require(&TRACKING_MANAGE)?;
    let page =
        service.list_tags(&tenant, &principal, query.org_id, &PageRequest::try_from(page)?).await?;
    Ok(page.map(TagResponse::from))
}

/// Add a tracking tag to a website.
///
/// Requires `compliance.tracking.manage` on the unit. Only the ID; it loads only after the visitor
/// consents to its category, and the provider is listed as a recipient of personal data in
/// `GET /public/purposes`. The cookie policy version changes when a new provider becomes active.
#[utoipa::path(
    post,
    path = "/tracking-tags",
    extensions(("x-permissions" = json!(["compliance.tracking.manage"]))),
    operation_id = "create_compliance_tracking_tag",
    security(("bearer" = [])),
    request_body = TagRequest,
    responses(
        (status = 201, description = "The tag.", body = Data<TagResponse>, headers(("Location" = String, description = "Path of the tag."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.tracking.manage` (code `permission_denied`) or the unit is out of your scope.", body = Problem),
        (status = 404, description = "No such unit; code `site_not_found`.", body = Problem),
        (status = 409, description = "The website already has this ID; code `compliance.tracking_tag_exists`.", body = Problem),
        (status = 422, description = "Unknown provider (code `compliance.tracking_provider_unknown`) or ID of the wrong form (code `compliance.tracking_id_invalid`, with `expected`).", body = Problem),
    )
)]
async fn create_tag(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<TagRequest>,
) -> Result<Created<TagResponse>, Error> {
    let row = service
        .create_tag(&tenant, &principal, input.org_id, &input.provider, &input.tracking_id)
        .await?;
    Ok(Created::new(format!("/compliance/tracking-tags/{}", row.id), row.into()))
}

/// Get a tracking tag.
///
/// Requires `compliance.tracking.manage` on its unit.
#[utoipa::path(
    get,
    path = "/tracking-tags/{id}",
    extensions(("x-permissions" = json!(["compliance.tracking.manage"]))),
    operation_id = "get_compliance_tracking_tag",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the tag.")),
    responses(
        (status = 200, description = "The tag.", body = Data<TagResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.tracking.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such tag in your scope; code `compliance.tracking_tag_not_found`.", body = Problem),
    )
)]
async fn get_tag(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<TagResponse>, Error> {
    Ok(Data { data: service.tag(&tenant, &principal, id).await?.into() })
}

/// Pause, resume or change the ID of a tracking tag.
///
/// Requires `compliance.tracking.manage` on its unit.
#[utoipa::path(
    patch,
    path = "/tracking-tags/{id}",
    extensions(("x-permissions" = json!(["compliance.tracking.manage"]))),
    operation_id = "update_compliance_tracking_tag",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the tag.")),
    request_body = TagChange,
    responses(
        (status = 200, description = "The tag after the change.", body = Data<TagResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.tracking.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such tag in your scope; code `compliance.tracking_tag_not_found`.", body = Problem),
        (status = 409, description = "The website already has this ID; code `compliance.tracking_tag_exists`.", body = Problem),
        (status = 422, description = "A value is invalid (code `compliance.tracking_id_invalid` for the ID).", body = Problem),
    )
)]
async fn update_tag(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<TagChange>,
) -> Result<Data<TagResponse>, Error> {
    let row = service.update_tag(&tenant, &principal, id, input.status, input.tracking_id).await?;
    Ok(Data { data: row.into() })
}

/// Remove a tracking tag.
///
/// Requires `compliance.tracking.manage` on its unit.
#[utoipa::path(
    delete,
    path = "/tracking-tags/{id}",
    extensions(("x-permissions" = json!(["compliance.tracking.manage"]))),
    operation_id = "delete_compliance_tracking_tag",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the tag.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.tracking.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such tag in your scope; code `compliance.tracking_tag_not_found`.", body = Problem),
    )
)]
async fn delete_tag(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_tag(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Get the cookie banner settings of a website.
///
/// Requires `compliance.tracking.manage` on the unit.
#[utoipa::path(
    get,
    path = "/cookie-settings/{org_id}",
    extensions(("x-permissions" = json!(["compliance.tracking.manage"]))),
    operation_id = "get_compliance_cookie_settings",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "Unit of the website.")),
    responses(
        (status = 200, description = "The settings.", body = Data<SettingsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.tracking.manage` (code `permission_denied`) or the unit is out of your scope.", body = Problem),
        (status = 404, description = "No such unit; code `site_not_found`.", body = Problem),
    )
)]
async fn get_cookie_settings(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
) -> Result<Data<SettingsResponse>, Error> {
    Ok(Data { data: service.cookie_settings(&tenant, &principal, org_id).await?.into() })
}

/// Cookie banner settings to save.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceCookieSettingsRequest)]
pub(crate) struct SettingsRequest {
    pub enabled: Option<bool>,
    /// Optional categories to ask about: `preferences`, `analytics`, `marketing`.
    pub categories: Option<Vec<String>>,
    /// `bottom`, `top` or `center`.
    pub position: Option<String>,
}

/// Set the cookie banner of a website.
///
/// Requires `compliance.tracking.manage` on the unit. Changing the categories asked about raises
/// the policy version (visitors are asked again).
#[utoipa::path(
    put,
    path = "/cookie-settings/{org_id}",
    extensions(("x-permissions" = json!(["compliance.tracking.manage"]))),
    operation_id = "set_compliance_cookie_settings",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "Unit of the website.")),
    request_body = SettingsRequest,
    responses(
        (status = 200, description = "The settings now.", body = Data<SettingsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.tracking.manage` (code `permission_denied`) or the unit is out of your scope.", body = Problem),
        (status = 404, description = "No such unit; code `site_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn set_cookie_settings(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
    ValidJson(input): ValidJson<SettingsRequest>,
) -> Result<Data<SettingsResponse>, Error> {
    let change = BannerChange {
        enabled: input.enabled,
        categories: input.categories,
        position: input.position,
    };
    Ok(Data {
        data: service.set_cookie_settings(&tenant, &principal, org_id, change).await?.into(),
    })
}

/// Which website.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct StatsQuery {
    /// Unit of the website.
    org_id: Uuid,
}

/// Consent counts of a website.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceCookieStats)]
pub(crate) struct StatsResponse {
    pub org_id: Uuid,
    /// Start of the period (the last 30 days).
    pub since: Timestamp,
    /// Choices recorded.
    pub total: u64,
    /// Choices that accepted each category.
    pub accepted: Vec<CategoryCount>,
}

/// How many accepted a category.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceCookieCategoryCount)]
pub(crate) struct CategoryCount {
    pub category: String,
    pub count: u64,
}

/// Get cookie consent counts of a website.
///
/// Requires `compliance.tracking.manage` on the unit. Totals of the last 30 days only, never single
/// records.
#[utoipa::path(
    get,
    path = "/cookie-consents/stats",
    extensions(("x-permissions" = json!(["compliance.tracking.manage"]))),
    operation_id = "get_compliance_cookie_stats",
    security(("bearer" = [])),
    params(StatsQuery),
    responses(
        (status = 200, description = "The counts.", body = Data<StatsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.tracking.manage` (code `permission_denied`) or the unit is out of your scope.", body = Problem),
        (status = 404, description = "No such unit; code `site_not_found`.", body = Problem),
    )
)]
async fn cookie_stats(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<StatsQuery>,
) -> Result<Data<StatsResponse>, Error> {
    let stats = service.cookie_stats(&tenant, &principal, query.org_id).await?;
    Ok(Data {
        data: StatsResponse {
            org_id: query.org_id,
            since: stats.since,
            total: stats.total,
            accepted: stats
                .by_category
                .into_iter()
                .map(|(category, count)| CategoryCount { category, count })
                .collect(),
        },
    })
}

/// What the interface needs for the cookie banner.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ConfigQuery {
    /// Categories the visitor already accepted (comma-separated, e.g. `analytics,marketing`), read
    /// from the necessary cookie. Only the tags of these categories are returned; none by default.
    granted: Option<String>,
    /// `vi` or `en` (default).
    locale: Option<String>,
}

/// A text of the banner.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceBannerText)]
pub(crate) struct BannerText {
    pub text: String,
    /// Whether the tenant may reword it (later, through text overrides); legal texts never.
    pub overridable: bool,
}

/// A consent category of the banner.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceConsentCategory)]
pub(crate) struct CategoryResponse {
    /// `necessary`, `preferences`, `analytics` or `marketing`.
    pub key: String,
    /// `true` only for `necessary` (always on, no tracking tag).
    pub required: bool,
    /// Always `false`: no box is ticked beforehand.
    pub preselected: bool,
    pub title: String,
    pub description: String,
    /// Providers that load when the category is accepted (names and countries, no IDs).
    pub providers: Vec<CategoryProvider>,
}

/// A provider listed in a category.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceConsentProvider)]
pub(crate) struct CategoryProvider {
    pub key: String,
    pub name: String,
    pub country: String,
}

/// A tag to load now.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceTagToLoad)]
pub(crate) struct LoadTag {
    pub provider: String,
    pub tracking_id: String,
    pub category: String,
    /// Script to load (`async`, after the page has rendered); `null` when the interface builds it
    /// from the provider's guide.
    pub script: Option<String>,
}

/// The cookie banner and the tags to load.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceConsentConfig)]
pub(crate) struct ConsentConfigResponse {
    /// `false`: show no banner and load no tag.
    pub enabled: bool,
    /// Store it with the visitor's choice; ask again when it changes.
    pub policy_version: i32,
    /// `bottom`, `top` or `center`.
    pub position: String,
    pub categories: Vec<CategoryResponse>,
    /// Tags to load now: only the active tags of the categories in `granted`; empty by default.
    pub tags: Vec<LoadTag>,
    /// Hosts of every active provider of the website, for the Content-Security-Policy.
    pub csp_hosts: Vec<String>,
    /// `title`, `body`, `accept_all`, `reject_all` (show it as prominently as `accept_all`),
    /// `customize`, `save`, `legal_title`, `legal`.
    #[schema(value_type = Object)]
    pub texts: std::collections::BTreeMap<String, BannerText>,
    /// How to load tags: `after_consent_deferred` — after consent, `async`, after the page has
    /// rendered; never on sign-in or account pages.
    pub loading: String,
}

fn render(
    template: MailTemplate,
    translations: &autoxide_kernel::i18n::Translations,
    language: Language,
) -> (String, String) {
    let rendered = template.render(translations, language, &[]);
    (rendered.subject, rendered.text)
}

/// Get the cookie banner and the tags to load, for a website.
///
/// No sign-in (the tenant and website come from the domain or `?site=`). Without `granted` no tag is
/// returned: nothing loads before consent. Rejecting all is as easy as accepting all, and no
/// category is preselected. Load the tags after the page renders.
#[utoipa::path(
    get,
    path = "/public/consent-config",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_compliance_consent_config",
    params(ConfigQuery, SiteQuery),
    responses(
        (status = 200, description = "The banner.", body = Data<ConsentConfigResponse>, headers(("ETag" = String, description = "Version; send it in `If-None-Match` to get 304."))),
    )
)]
async fn public_consent_config(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    site: Site,
    Query(query): Query<ConfigQuery>,
) -> Result<Tagged<ConsentConfigResponse>, Error> {
    let banner = service.public_banner(&tenant, site.org()).await?;
    let language = query.locale.as_deref().and_then(Language::from_tag).unwrap_or(Language::En);
    let translations = &service.translations;
    let granted: Vec<&str> =
        query.granted.as_deref().unwrap_or_default().split(',').map(str::trim).collect();
    let version =
        banner.tags.iter().map(|tag| tag.updated_at).fold(banner.updated_at, Timestamp::max);
    let mut categories = Vec::new();
    for key in std::iter::once("necessary").chain(banner.categories.iter().map(String::as_str)) {
        let template = match key {
            "necessary" => NECESSARY,
            "preferences" => PREFERENCES,
            "analytics" => ANALYTICS,
            _ => MARKETING,
        };
        let (title, description) = render(template, translations, language);
        let mut listed: Vec<CategoryProvider> = Vec::new();
        for tag in &banner.tags {
            if let Some(item) = provider(&tag.provider).filter(|item| item.category == key)
                && !listed.iter().any(|known| known.key == item.key)
            {
                listed.push(CategoryProvider {
                    key: item.key.to_owned(),
                    name: item.name.to_owned(),
                    country: item.country.to_owned(),
                });
            }
        }
        categories.push(CategoryResponse {
            key: key.to_owned(),
            required: key == "necessary",
            preselected: false,
            title,
            description,
            providers: listed,
        });
    }
    let tags: Vec<LoadTag> = if banner.enabled {
        banner
            .tags
            .iter()
            .filter_map(|tag| provider(&tag.provider).map(|item| (tag, item)))
            .filter(|(_, item)| granted.contains(&item.category))
            .map(|(tag, item)| LoadTag {
                provider: tag.provider.clone(),
                tracking_id: tag.tracking_id.clone(),
                category: item.category.to_owned(),
                script: item.script.map(|script| script.replace("{id}", &tag.tracking_id)),
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut csp_hosts: Vec<String> = banner
        .tags
        .iter()
        .filter_map(|tag| provider(&tag.provider))
        .flat_map(|item| item.hosts.iter().map(|host| (*host).to_owned()))
        .collect();
    csp_hosts.sort();
    csp_hosts.dedup();
    let mut texts = std::collections::BTreeMap::new();
    for (template, first, second, overridable) in [
        (INTRO, "title", "body", true),
        (CHOICE, "accept_all", "reject_all", true),
        (ACTIONS, "customize", "save", true),
        (LEGAL, "legal_title", "legal", false),
    ] {
        let (a, b) = render(template, translations, language);
        texts.insert(first.to_owned(), BannerText { text: a, overridable });
        texts.insert(second.to_owned(), BannerText { text: b, overridable });
    }
    Ok(Tagged {
        version,
        data: ConsentConfigResponse {
            enabled: banner.enabled,
            policy_version: banner.policy_version,
            position: banner.position,
            categories,
            tags,
            csp_hosts,
            texts,
            loading: "after_consent_deferred".to_owned(),
        },
    })
}

/// A visitor's choice to record.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceCookieConsentRequest)]
pub(crate) struct ConsentRequest {
    /// A random value (16 to 64 letters, digits, `-`, `_`) the interface creates and keeps in a
    /// necessary cookie.
    pub consent_id: String,
    /// Categories accepted; an empty list rejects all (`necessary` is always on).
    pub categories: Vec<String>,
    /// The `policy_version` of the banner shown.
    pub policy_version: i32,
}

/// Record a visitor's cookie choice (anonymous proof of consent).
///
/// No sign-in (the tenant and website come from the domain or `?site=`). Stores the random id, the
/// categories, the policy version and a hash of the network block — never the raw IP. A signed-in
/// visitor's choice is also written to the consent ledger (purposes `analytics` / `marketing`).
/// Limited per network block per hour.
#[utoipa::path(
    post,
    path = "/public/cookie-consents",
    extensions(("x-public" = json!(true))),
    operation_id = "record_public_compliance_cookie_consent",
    params(SiteQuery),
    request_body = ConsentRequest,
    responses(
        (status = 204, description = "Recorded."),
        (status = 409, description = "The banner changed or is off; show it again (code `compliance.cookie_policy_changed`, with `version`).", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
        (status = 429, description = "Too many records from this address; code `compliance.cookie_consents_throttled`, wait `Retry-After` seconds.", body = Problem),
    )
)]
async fn record_cookie_consent(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    site: Site,
    principal: Option<Principal>,
    client: ClientInfo,
    ValidJson(input): ValidJson<ConsentRequest>,
) -> Result<StatusCode, Error> {
    service
        .record_cookie_consent(
            &tenant,
            site.org(),
            principal.as_ref(),
            client.ip,
            &input.consent_id,
            input.categories,
            input.policy_version,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
