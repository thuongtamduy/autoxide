//! Endpoint REST của module `menus`, dưới `/api/v1/menus`. Handler chỉ: lấy dữ liệu vào → gọi
//! service (service kiểm permission) → trả envelope. Mọi route đăng ký qua `routes!` để có tài
//! liệu OpenAPI (luật kiến trúc 6).

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{Site, SiteQuery, TenantContext};
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::items::{BadgeInput, ItemInput, Node, RefInput};
use crate::public::PublicSiteMenu;
use crate::service::{Menu, MenuService, NewMenu};

pub(crate) fn routes(service: Arc<MenuService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_menus, create_menu))
        .routes(routes!(get_menu, replace_menu, delete_menu))
        .routes(routes!(get_public_menu))
        .routes(routes!(import_content_menus))
        .with_state(service)
}

/// An object of a module, as a resource type (`<module>.<type>`, e.g. `content.entry`,
/// `content.term`) and its id.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MenuTarget {
    #[schema(example = "content.term")]
    kind: String,
    id: String,
}

/// A small label on an item, e.g. "-50%" or "New".
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MenuBadge {
    /// 1 to 20 characters, in the menu's language.
    #[schema(example = "-50%")]
    text: String,
    /// `sale`, `brand`, `new` or `neutral`.
    #[schema(example = "sale")]
    tone: String,
    /// Shown from this moment; omitted: at once.
    from: Option<DateTime<Utc>>,
    /// Hidden from this moment on; omitted: never.
    until: Option<DateTime<Utc>>,
}

/// A menu item to save. It links to at most one of `url` and `ref`; with neither it is a
/// heading that groups its sub-items (e.g. a column of a mega menu).
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SiteMenuItemInput {
    /// The item's id from the menu as read, to keep it across this save (the response of
    /// `PUT` shows the same id). Leave it out for a new item, and when creating a menu.
    id: Option<Uuid>,
    /// Up to 200 characters. Needed unless `ref` is given; empty with `ref` shows the title of
    /// the object, in the menu's language.
    label: Option<String>,
    /// An `https://`, `http://` or `mailto:` address, or a path starting with a single `/`.
    url: Option<String>,
    /// The object to link to. It must exist; the public menu leaves the item out (with its
    /// sub-items) while the object is not public.
    #[serde(rename = "ref")]
    target: Option<MenuTarget>,
    /// Also list the object's own sub-items (e.g. the sub-categories of a category) after the
    /// item's children. Only for objects that have sub-items, and only above the 4th level.
    #[serde(default)]
    auto_children: bool,
    /// How many sub-items `auto_children` takes, 1 to 50; default 12.
    auto_children_limit: Option<i32>,
    /// One of: `bag`, `bolt`, `book`, `calendar`, `cart`, `category`, `chat`, `fire`, `gift`,
    /// `grid`, `heart`, `help`, `home`, `info`, `mail`, `map_pin`, `news`, `percent`, `phone`,
    /// `search`, `shop`, `sparkles`, `star`, `store`, `tag`, `truck`, `user`.
    icon: Option<String>,
    /// Sets the item apart: `accent`, `brand`, `sale` or `muted`.
    highlight: Option<String>,
    badge: Option<MenuBadge>,
    /// An object to feature in the item's drop-down panel (title, summary, link, picture).
    feature: Option<MenuTarget>,
    /// Sub-items. Menus nest at most 4 levels and hold at most 200 items in all.
    #[serde(default)]
    #[schema(no_recursion)]
    children: Vec<SiteMenuItemInput>,
}

impl From<MenuTarget> for RefInput {
    fn from(target: MenuTarget) -> Self {
        RefInput { kind: target.kind, id: target.id }
    }
}

impl From<SiteMenuItemInput> for ItemInput {
    fn from(input: SiteMenuItemInput) -> Self {
        ItemInput {
            id: input.id,
            label: input.label,
            url: input.url,
            target: input.target.map(RefInput::from),
            auto_children: input.auto_children,
            auto_children_limit: input.auto_children_limit,
            icon: input.icon,
            highlight: input.highlight,
            badge: input.badge.map(|badge| BadgeInput {
                text: badge.text,
                tone: badge.tone,
                from: badge.from,
                until: badge.until,
            }),
            feature: input.feature.map(RefInput::from),
            children: input.children.into_iter().map(ItemInput::from).collect(),
        }
    }
}

/// A new menu.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateSiteMenuRequest {
    /// 1 to 64 lower-case letters, digits, `-` or `_`; unique per language. The website asks
    /// for a menu by its key.
    #[schema(example = "main")]
    key: String,
    /// The menu's language; omitted: the default language.
    #[schema(example = "vi")]
    locale: Option<String>,
    /// The website (organization unit) the menu is for; you need `menus.manage` there. Omitted:
    /// the website of the whole tenant, used by every website without a menu of its own.
    org_id: Option<Uuid>,
    #[serde(default)]
    items: Vec<SiteMenuItemInput>,
}

impl From<CreateSiteMenuRequest> for NewMenu {
    fn from(input: CreateSiteMenuRequest) -> Self {
        NewMenu {
            key: input.key,
            locale: input.locale,
            org_id: input.org_id,
            items: input.items.into_iter().map(ItemInput::from).collect(),
        }
    }
}

/// The whole item tree of a menu.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplaceSiteMenuItemsRequest {
    items: Vec<SiteMenuItemInput>,
}

impl ReplaceSiteMenuItemsRequest {
    pub(crate) fn into_items(self) -> Vec<ItemInput> {
        self.items.into_iter().map(ItemInput::from).collect()
    }
}

/// A website menu as saved.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct SiteMenu {
    id: Uuid,
    /// The website (organization unit) of the menu; the id of the tenant for the whole tenant.
    org_id: Uuid,
    #[schema(example = "main")]
    key: String,
    #[schema(example = "vi")]
    locale: String,
    items: Vec<SiteMenuItem>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// The object a saved item links to. With `include=targets`, also how it stands now.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct SavedMenuTarget {
    #[schema(example = "content.term")]
    kind: String,
    id: String,
    /// Only with `include=targets`: `public` (shown on the website), `not_public` (exists but
    /// strangers cannot see it, e.g. a draft: the public menu leaves the item out), `missing` (no
    /// such object any more) or `unknown_kind` (no installed module provides this type).
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<TargetState>,
    /// Only with `include=targets` and `state` = `public`: its title in the menu's language.
    /// Objects that are not public do not give their title to other modules.
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    /// Only with `include=targets` and `state` = `public`: its public path, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

pub(crate) use crate::public::TargetState;

/// A menu item as saved.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct SiteMenuItem {
    id: Uuid,
    /// Empty when the item shows the title of its `ref`.
    label: String,
    url: Option<String>,
    #[serde(rename = "ref")]
    target: Option<SavedMenuTarget>,
    auto_children: bool,
    auto_children_limit: Option<i32>,
    icon: Option<String>,
    highlight: Option<String>,
    badge: Option<MenuBadge>,
    feature: Option<SavedMenuTarget>,
    #[schema(no_recursion)]
    children: Vec<SiteMenuItem>,
}

fn items(nodes: Vec<Node>, targets: &Targets) -> Vec<SiteMenuItem> {
    nodes
        .into_iter()
        .map(|node| {
            let row = node.item;
            let pair = |kind: Option<String>, id: Option<String>| {
                let (kind, id) = (kind?, id?);
                let found = targets.get(&(kind.clone(), id.clone()));
                Some(SavedMenuTarget {
                    state: found.map(|info| info.state),
                    title: found.and_then(|info| info.title.clone()),
                    path: found.and_then(|info| info.path.clone()),
                    kind,
                    id,
                })
            };
            SiteMenuItem {
                id: row.id,
                label: row.label,
                url: row.url,
                target: pair(row.ref_kind, row.ref_id),
                auto_children: row.auto_children,
                auto_children_limit: row.auto_children_limit,
                icon: row.icon,
                highlight: row.highlight,
                badge: match (row.badge_text, row.badge_tone) {
                    (Some(text), Some(tone)) => {
                        Some(MenuBadge { text, tone, from: row.badge_from, until: row.badge_until })
                    }
                    _ => None,
                },
                feature: pair(row.feature_kind, row.feature_id),
                children: items(node.children, targets),
            }
        })
        .collect()
}

/// Tình trạng đích của từng mục, theo (`kind`, `id`); rỗng khi không xin `include=targets`.
pub(crate) type Targets = HashMap<(String, String), crate::public::TargetInfo>;

pub(crate) fn menu_response(menu: Menu, nodes: Vec<Node>) -> SiteMenu {
    menu_response_with(menu, nodes, &Targets::new())
}

fn menu_response_with(menu: Menu, nodes: Vec<Node>, targets: &Targets) -> SiteMenu {
    SiteMenu {
        id: menu.id,
        org_id: menu.org_id.unwrap_or(menu.tenant_id),
        key: menu.key,
        locale: menu.locale,
        items: items(nodes, targets),
        created_at: menu.created_at,
        updated_at: menu.updated_at,
    }
}

/// A menu in the list.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct SiteMenuSummary {
    id: Uuid,
    /// The website (organization unit) of the menu.
    org_id: Uuid,
    #[schema(example = "main")]
    key: String,
    #[schema(example = "vi")]
    locale: String,
    /// How many items the menu holds, at every level.
    #[schema(example = 12)]
    item_count: u64,
    /// Who created the menu or last replaced its items; `null` when unknown (menus saved
    /// before this field existed, or changed by the system).
    updated_by: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl SiteMenuSummary {
    pub(crate) fn new(menu: Menu, item_count: u64) -> Self {
        SiteMenuSummary {
            id: menu.id,
            org_id: menu.org_id.unwrap_or(menu.tenant_id),
            key: menu.key,
            locale: menu.locale,
            item_count,
            updated_by: menu.updated_by,
            created_at: menu.created_at,
            updated_at: menu.updated_at,
        }
    }
}

/// List website menus.
///
/// Requires `menus.read`. Every menu in every language, newest first by default; follow
/// `pagination.next_cursor` for more.
#[utoipa::path(
    get,
    path = "/",
    extensions(("x-permissions" = json!(["menus.read"]))),
    operation_id = "list_menus",
    security(("bearer" = [])),
    params(MenusQuery, PageQuery),
    responses(
        (status = 200, description = "One page of menus.", body = Page<SiteMenuSummary>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `menus.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "The paging parameters are invalid; see `errors`.", body = Problem),
    )
)]
async fn list_menus(
    State(service): State<Arc<MenuService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<MenusQuery>,
    Query(query): Query<PageQuery>,
) -> Result<Page<SiteMenuSummary>, Error> {
    let page = PageRequest::try_from(query)?;
    let page = service.list(&tenant, &principal, filter.owner_org, &page).await?;
    Ok(page.map(|(menu, items)| SiteMenuSummary::new(menu, items)))
}

/// Create a website menu.
///
/// Requires `menus.manage`. One menu per key and language: the website shows the menu `main`
/// in Vietnamese from the menu with key `main` and locale `vi`. All problems with the items
/// are reported at once, each with its place (e.g. `items[0].children[1].ref.id`).
#[utoipa::path(
    post,
    path = "/",
    extensions(("x-permissions" = json!(["menus.manage"]))),
    operation_id = "create_menu",
    security(("bearer" = [])),
    request_body = CreateSiteMenuRequest,
    responses(
        (status = 201, description = "Created; `Location` is the new menu.", body = Data<SiteMenu>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `menus.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "A menu with this key and language exists; code `menus.menu_exists`.", body = Problem),
        (status = 422, description = "Invalid values; see `errors`. Code `menus.ref_unknown_kind` when every problem is a `ref` of a kind no module provides, `menus.ref_not_found` when every problem is a missing object, `menus.children_unsupported` when every problem is `auto_children` on a kind without sub-items; otherwise `validation_failed`.", body = Problem),
    )
)]
async fn create_menu(
    State(service): State<Arc<MenuService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateSiteMenuRequest>,
) -> Result<Created<SiteMenu>, Error> {
    let (menu, nodes) = service.create(&tenant, &principal, input.into()).await?;
    let version = menu.updated_at;
    Ok(Created::new(format!("/menus/{}", menu.id), menu_response(menu, nodes)).tagged(version))
}

/// Get a website menu.
///
/// Requires `menus.read`. The items as saved. With `include=targets`, each `ref` and `feature`
/// also says whether its object is shown on the website, with the title and path of public ones
/// (see `GET /api/v1/menus/public/{key}` for the menu exactly as the website shows it).
#[utoipa::path(
    get,
    path = "/{id}",
    extensions(("x-permissions" = json!(["menus.read"]))),
    operation_id = "get_menu",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the menu."), MenuInclude),
    responses(
        (status = 200, description = "The menu.", body = Data<SiteMenu>, headers(("ETag" = String, description = "Version of the menu: send it in `If-Match` when replacing its items."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `menus.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No menu has this id; code `menus.menu_not_found`.", body = Problem),
    )
)]
async fn get_menu(
    State(service): State<Arc<MenuService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(include): Query<MenuInclude>,
) -> Result<Tagged<SiteMenu>, Error> {
    let (menu, nodes) = service.get(&tenant, &principal, id).await?;
    let targets = match include.include.as_deref().map(str::trim) {
        None | Some("") => Targets::new(),
        Some("targets") => service.target_states(&tenant, &nodes, &menu.locale).await?,
        Some(_) => {
            return Err(Error::validation(vec![autoxide_kernel::error::FieldError::new(
                "include",
                "invalid",
                "must be `targets`",
            )]));
        }
    };
    Ok(Tagged { version: menu.updated_at, data: menu_response_with(menu, nodes, &targets) })
}

/// What to add to a menu.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct MenuInclude {
    /// `targets`: how the object of each `ref` and `feature` stands now (`state`, and `title`,
    /// `path` of public ones), in one request.
    include: Option<String>,
}

/// Replace the items of a website menu.
///
/// Requires `menus.manage`. Replaces the whole item tree. `If-Match` is required: the menu's
/// `ETag` (or `*` to replace it whatever its version). If someone changed the menu since you
/// read it, nothing is saved and the answer is 412.
#[utoipa::path(
    put,
    path = "/{id}",
    extensions(("x-permissions" = json!(["menus.manage"]))),
    operation_id = "replace_menu_items",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the menu."), ("If-Match" = String, Header, description = "Required: the `ETag` you read, or `*`.")),
    request_body = ReplaceSiteMenuItemsRequest,
    responses(
        (status = 200, description = "The menu as saved.", body = Data<SiteMenu>, headers(("ETag" = String, description = "New version of the menu."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `menus.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No menu has this id; code `menus.menu_not_found`.", body = Problem),
        (status = 412, description = "The menu changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "Invalid values; see `errors` and the codes listed for `POST /api/v1/menus`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
async fn replace_menu(
    State(service): State<Arc<MenuService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<ReplaceSiteMenuItemsRequest>,
) -> Result<Tagged<SiteMenu>, Error> {
    let expected = if_match.required()?;
    let (menu, nodes) =
        service.replace(&tenant, &principal, id, input.into_items(), expected).await?;
    Ok(Tagged { version: menu.updated_at, data: menu_response(menu, nodes) })
}

/// Delete a website menu.
///
/// Requires `menus.manage`. Deletes the menu and all its items; it cannot be restored.
#[utoipa::path(
    delete,
    path = "/{id}",
    extensions(("x-permissions" = json!(["menus.manage"]))),
    operation_id = "delete_menu",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the menu.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `menus.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No menu has this id (or it was already deleted); code `menus.menu_not_found`.", body = Problem),
    )
)]
async fn delete_menu(
    State(service): State<Arc<MenuService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Which menus to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct MenusQuery {
    /// Only the menus of this website (organization unit).
    owner_org: Option<Uuid>,
}

/// The language of a public menu.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PublicQuery {
    /// Omitted: the default language. A menu missing in this language falls back to the
    /// default language; `locale` in the answer says which one was returned.
    #[param(example = "vi")]
    locale: Option<String>,
}

/// Get a website menu, ready to show (public).
///
/// No sign-in needed. Labels, links and pictures are worked out in the menu's language. Items
/// pointing to objects that are not public are left out with their sub-items, headings with
/// nothing left under them are left out, `auto_children` are filled in, and badges show only
/// in their date range. Best sellers of a category are not part of the menu: call the
/// product list of the catalog with the item's `ref`.
#[utoipa::path(
    get,
    path = "/public/{key}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_menu",
    security((), ("bearer" = [])),
    params(("key" = String, Path, description = "Key of the menu, e.g. `main`."), PublicQuery, SiteQuery),
    responses(
        (status = 200, description = "The menu.", body = Data<PublicSiteMenu>, headers(("ETag" = String, description = "Fingerprint of the whole answer; send it in `If-None-Match` to get 304."), ("Cache-Control" = String, description = "How long the answer may be reused: until the next badge starts or ends, at most `AUTOXIDE_MENUS_PUBLIC_CACHE_SECONDS`."))),
        (status = 304, description = "The menu has not changed since the `ETag` sent in `If-None-Match`."),
        (status = 404, description = "No menu has this key, in this language or the default one; code `menus.public_menu_not_found`.", body = Problem),
        (status = 422, description = "The language is not supported; see `errors`.", body = Problem),
    )
)]
async fn get_public_menu(
    State(service): State<Arc<MenuService>>,
    tenant: TenantContext,
    site: Site,
    Path(key): Path<String>,
    Query(query): Query<PublicQuery>,
) -> Result<Response, Error> {
    let now = Utc::now();
    let ready = service.public_menu(&tenant, &site, &key, query.locale.as_deref(), now).await?;
    let seconds = (ready.expires_at - now).num_seconds();
    let cache =
        if seconds > 0 { format!("public, max-age={seconds}") } else { "no-cache".to_owned() };
    let mut response = Data { data: &ready.menu }.into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&ready.etag) {
        headers.insert(header::ETAG, value);
    }
    if let Ok(value) = HeaderValue::from_str(&cache) {
        headers.insert(header::CACHE_CONTROL, value);
    }
    Ok(response)
}

/// What to do when importing the old menus.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImportContentMenusRequest {
    /// `true`: only say what would be imported; nothing is saved. Defaults to `false`.
    #[serde(default)]
    dry_run: bool,
}

/// A menu of the import, by key and language.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ImportedMenu {
    #[schema(example = "main")]
    key: String,
    #[schema(example = "vi")]
    locale: String,
}

/// A menu that could not be imported, and why.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct FailedMenu {
    key: String,
    locale: String,
    /// Why, in English (e.g. the language is not one of the menu languages).
    reason: String,
}

/// What the import did (or, with `dry_run`, would do).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ImportContentMenusReport {
    dry_run: bool,
    /// Imported (with `dry_run`: would be imported).
    imported: Vec<ImportedMenu>,
    /// Already here with the same key and language: left as they are.
    skipped: Vec<ImportedMenu>,
    failed: Vec<FailedMenu>,
}

/// Import the old menus of module `content`.
///
/// Requires `menus.manage` for the whole tenant: the old menus become menus of the whole
/// tenant. Copies the old website menus kept by module `content` (before module `menus`) into
/// this module, like `autoxide task menus.import_content_menus`. A menu that already exists with
/// the same key and language is left as it is, so running it again is safe. Try it first with
/// `dry_run`: nothing is saved, and the answer lists what would be imported. When a dry run
/// imports nothing and skips everything, the import was already done.
#[utoipa::path(
    post,
    path = "/import-content",
    extensions(("x-permissions" = json!(["menus.manage"]))),
    operation_id = "import_content_menus",
    security(("bearer" = [])),
    request_body = ImportContentMenusRequest,
    responses(
        (status = 200, description = "What was imported (or would be, with `dry_run`).", body = Data<ImportContentMenusReport>),
        (status = 400, description = "Module `content` is not installed; code `menus.import_source_missing`.", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `menus.manage` for the whole tenant; code `permission_denied` or `org_out_of_scope`.", body = Problem),
    )
)]
async fn import_content_menus(
    State(service): State<Arc<MenuService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ImportContentMenusRequest>,
) -> Result<Data<ImportContentMenusReport>, Error> {
    let report = service.import_content_menus(&tenant, Some(&principal), input.dry_run).await?;
    let menus = |pairs: Vec<(String, String)>| {
        pairs.into_iter().map(|(key, locale)| ImportedMenu { key, locale }).collect()
    };
    Ok(Data {
        data: ImportContentMenusReport {
            dry_run: input.dry_run,
            imported: menus(report.imported),
            skipped: menus(report.skipped),
            failed: report
                .failed
                .into_iter()
                .map(|(key, locale, reason)| FailedMenu { key, locale, reason })
                .collect(),
        },
    })
}
