//! `GET /api/v1/kernel/navigation` (bước 51 P1): sổ điều hướng đã lọc cho người gọi ở khách và
//! nút đang chọn. Mục ẩn khi module chưa bật cho khách hay người gọi không có permission nào của
//! mục ở nút; mục **khoá** (vẫn hiện, kèm `required_feature`) khi module bật mà gói thiếu tính
//! năng. Nhóm không còn mục nào thì bỏ. Nhóm của không gian nền tảng chỉ hiện cho nhân sự khách
//! nền tảng.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::{OrgTarget, Principal};
use crate::entitlement::EntitlementsHandle;
use crate::error::Error;
use crate::i18n::{Language, Translations};
use crate::mcp::Tool;
use crate::module::navigation::{NavigationCatalog, Space};
use crate::ops::module_catalog::ModuleCatalog;
use crate::response::{Data, Problem};
use crate::tenancy::{OrgId, TenantContext, root_of};

/// The navigation the caller may use, grouped.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct NavigationView {
    /// Groups in display order; a group without any visible item is left out.
    pub groups: Vec<NavGroupView>,
}

/// One group of the side bar.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct NavGroupView {
    /// Stable key, e.g. `content`.
    #[schema(example = "content")]
    pub id: String,
    /// Label in the language of `Accept-Language`.
    pub label: String,
    /// Neutral icon name (`home`, `cart`, `file`…); interfaces map it to their own icons.
    #[schema(example = "file")]
    pub icon: String,
    /// `tenant` (the tenant's admin) or `platform` (platform staff only).
    #[schema(example = "tenant")]
    pub space: String,
    pub items: Vec<NavItemView>,
}

/// One screen of a group.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct NavItemView {
    /// Stable key of the screen, e.g. `content.entries`.
    #[schema(example = "content.entries")]
    pub id: String,
    pub label: String,
    /// Label of the sub-group in the row of tabs, when there is one.
    pub section: Option<String>,
    /// `available`, or `locked` when the tenant's plan lacks `required_feature` (show it with a
    /// lock and a way to upgrade, do not hide it).
    #[schema(example = "available")]
    pub state: String,
    pub required_feature: Option<String>,
    /// Work source of the inbox that counts pending work for this screen, when there is one.
    pub counter: Option<String>,
}

/// Where the caller works.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct NavigationQuery {
    /// The organization unit chosen in the interface; omitted: the whole tenant. Items show when
    /// the caller has one of their permissions at this unit.
    org_id: Option<Uuid>,
}

#[derive(Clone)]
pub(crate) struct NavigationState {
    pub catalog: ModuleCatalog,
    pub entitlements: EntitlementsHandle,
    pub translations: Translations,
}

impl NavigationState {
    /// Sổ đã lọc cho `principal` ở khách `tenant`, nút `org_id`.
    pub(crate) async fn view(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org_id: Option<Uuid>,
        language: Language,
    ) -> Result<NavigationView, Error> {
        let Some(catalog) = self.catalog.navigation() else {
            return Ok(NavigationView { groups: Vec::new() });
        };
        let platform = crate::tenancy::require_platform(principal).is_ok();
        let entitlements = self.entitlements.snapshot(tenant.id()).await?;
        let target = OrgTarget::new(org_id.map_or(root_of(tenant.id()), OrgId::from_uuid), None);
        let read = tenant.database().read();
        let mut groups = Vec::new();
        for group in visible_groups(catalog, platform) {
            let mut items = Vec::new();
            for (module, item) in catalog.items.iter().filter(|(_, item)| item.group == group.id) {
                if !self.entitlements.module_enabled(tenant.id(), module).await? {
                    continue;
                }
                let mut allowed = item.permissions.is_empty();
                for permission in &item.permissions {
                    if allowed {
                        break;
                    }
                    allowed = principal.require_at(read, permission, target).await.is_ok();
                }
                if !allowed {
                    continue;
                }
                let locked = item.feature.filter(|feature| !entitlements.has_feature(feature));
                items.push(NavItemView {
                    id: item.id.to_owned(),
                    label: self.translations.describe(item.label, language),
                    section: item
                        .section
                        .map(|section| self.translations.describe(section, language)),
                    state: if locked.is_some() { "locked" } else { "available" }.to_owned(),
                    required_feature: locked.map(str::to_owned),
                    counter: item.counter.map(str::to_owned),
                });
            }
            if !items.is_empty() {
                groups.push(NavGroupView {
                    id: group.id.to_owned(),
                    label: self.translations.describe(group.label, language),
                    icon: group.icon.to_owned(),
                    space: group.space.as_str().to_owned(),
                    items,
                });
            }
        }
        Ok(NavigationView { groups })
    }
}

fn visible_groups(
    catalog: &NavigationCatalog,
    platform: bool,
) -> impl Iterator<Item = &crate::module::navigation::NavGroup> {
    catalog.groups.iter().filter(move |group| group.space == Space::Tenant || platform)
}

pub(crate) fn routes(state: NavigationState) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(get_navigation)).with_state(Arc::new(state))
}

/// Get the navigation of the admin interface.
///
/// Any signed-in user. The groups and screens modules declare, for the caller in this tenant at
/// `org_id`: a screen shows when its module is on for the tenant and the caller has one of its
/// permissions there; it is `locked` (shown, not hidden) when the plan lacks a feature it needs.
/// Labels follow `Accept-Language`. Send the `ETag` back in `If-None-Match` to get `304` while
/// nothing changed for you.
#[utoipa::path(
    get,
    path = "/navigation",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_navigation",
    security(("bearer" = [])),
    params(NavigationQuery),
    responses(
        (status = 200, description = "The navigation.", body = Data<NavigationView>, headers(("ETag" = String, description = "Fingerprint of this answer."))),
        (status = 304, description = "Unchanged since the `ETag` sent in `If-None-Match`."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn get_navigation(
    State(state): State<Arc<NavigationState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    Query(query): Query<NavigationQuery>,
    headers: HeaderMap,
) -> Result<Response, Error> {
    let view = state.view(&tenant, &principal, query.org_id, language).await?;
    let data = Data { data: view };
    let body = serde_json::to_vec(&data).map_err(Error::internal)?;
    // Vân tay của chính câu trả lời: gồm bản dựng (nhãn, mục), quyền của người gọi ở nút và gói.
    let digest = Sha256::digest(&body);
    let etag = format!(
        "\"{}\"",
        digest.iter().take(16).map(|byte| format!("{byte:02x}")).collect::<String>()
    );
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|sent| sent.split(',').any(|tag| tag.trim() == etag));
    let etag_value = HeaderValue::from_str(&etag).map_err(Error::internal)?;
    if unchanged {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag_value)]).into_response());
    }
    Ok((
        [
            (header::ETAG, etag_value),
            (header::CONTENT_TYPE, HeaderValue::from_static("application/json")),
        ],
        body,
    )
        .into_response())
}

pub(crate) fn tools(state: NavigationState) -> Vec<Tool> {
    vec![
        Tool::new(
            "navigation",
            "Get the screens of the admin interface you may use in this tenant, grouped as in the \
             side bar: `locked` screens need a feature the plan lacks. Use it to tell a person \
             where to go.",
            move |tenant: TenantContext, principal: Principal, input: NavigationInput| {
                let state = state.clone();
                async move {
                    let view = state.view(&tenant, &principal, input.org_id, Language::En).await?;
                    Ok::<Data<NavigationView>, Error>(Data { data: view })
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
    ]
}

/// Where the caller works.
#[derive(Debug, Default, Deserialize, validator::Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NavigationInput {
    /// The organization unit chosen; omitted: the whole tenant.
    org_id: Option<Uuid>,
}
