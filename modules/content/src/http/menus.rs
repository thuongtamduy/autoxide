//! Endpoint quản lý menu: `/menus`. Đọc cần `content.read`; `PUT` cần `content.manage`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::service::ContentService;
use crate::taxonomy::menus::{ItemInput, Menu, Node};

/// A navigation menu.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct MenuResponse {
    id: Uuid,
    #[schema(example = "main")]
    key: String,
    locale: String,
    items: Vec<MenuItemResponse>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// A menu item: points to an entry, a term or an address.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct MenuItemResponse {
    id: Uuid,
    label: String,
    entry_id: Option<Uuid>,
    term_id: Option<Uuid>,
    url: Option<String>,
    #[schema(no_recursion)]
    children: Vec<MenuItemResponse>,
}

fn items(nodes: Vec<Node>) -> Vec<MenuItemResponse> {
    nodes
        .into_iter()
        .map(|node| MenuItemResponse {
            id: node.item.id,
            label: node.item.label,
            entry_id: node.item.target_entry_id,
            term_id: node.item.target_term_id,
            url: node.item.target_url,
            children: items(node.children),
        })
        .collect()
}

fn menu_response(menu: Menu, nodes: Vec<Node>) -> MenuResponse {
    MenuResponse {
        id: menu.id,
        key: menu.key,
        locale: menu.locale,
        items: items(nodes),
        created_at: menu.created_at,
        updated_at: menu.updated_at,
    }
}

/// A menu item to set. Give exactly one of `entry_id`, `term_id` and `url`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MenuItemInput {
    /// 1 to 200 characters.
    label: String,
    /// An entry in the menu's language; shown only while published.
    entry_id: Option<Uuid>,
    /// A category or tag.
    term_id: Option<Uuid>,
    /// An `https://` or `mailto:` address, or a path starting with `/`.
    url: Option<String>,
    /// Sub-items; menus nest at most 4 levels.
    #[serde(default)]
    #[schema(no_recursion)]
    children: Vec<MenuItemInput>,
}

impl From<MenuItemInput> for ItemInput {
    fn from(input: MenuItemInput) -> Self {
        ItemInput {
            label: input.label,
            entry_id: input.entry_id,
            term_id: input.term_id,
            url: input.url,
            children: input.children.into_iter().map(ItemInput::from).collect(),
        }
    }
}

/// The whole item tree of a menu.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PutMenuRequest {
    /// At most 200 items in all.
    items: Vec<MenuItemInput>,
}

/// The language of a menu.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct MenuQuery {
    /// Omitted: the default language.
    locale: Option<String>,
}

/// A menu in the list.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct MenuSummary {
    id: Uuid,
    key: String,
    locale: String,
    updated_at: DateTime<Utc>,
}

/// List menus.
///
/// Requires `content.read`. Every menu in every language, by key.
#[utoipa::path(
    get,
    path = "/menus",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "list_content_menus",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The menus.", body = Data<Vec<MenuSummary>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
    )
)]
pub(crate) async fn list_menus(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
) -> Result<Data<Vec<MenuSummary>>, Error> {
    let menus = service.list_menus(&principal).await?;
    let data = menus
        .into_iter()
        .map(|menu| MenuSummary {
            id: menu.id,
            key: menu.key,
            locale: menu.locale,
            updated_at: menu.updated_at,
        })
        .collect();
    Ok(Data { data })
}

/// Get a menu.
///
/// Requires `content.read`.
#[utoipa::path(
    get,
    path = "/menus/{key}",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "get_content_menu",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the menu, e.g. `main`."), MenuQuery),
    responses(
        (status = 200, description = "The menu.", body = Data<MenuResponse>, headers(("ETag" = String, description = "Version of the menu: send it in `If-Match` when replacing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such menu in this language; code `content.menu_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_menu(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(key): Path<String>,
    Query(query): Query<MenuQuery>,
) -> Result<Tagged<MenuResponse>, Error> {
    let (menu, nodes) = service.get_menu(&principal, &key, query.locale.as_deref()).await?;
    Ok(Tagged { version: menu.updated_at, data: menu_response(menu, nodes) })
}

/// Create or replace a menu.
///
/// Requires `content.manage`. Replaces the whole item tree. `If-Match` is required: the
/// menu's `ETag`, or `*` (needed to create a new menu).
#[utoipa::path(
    put,
    path = "/menus/{key}",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "put_content_menu",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the menu: 1 to 64 lower-case letters, digits, `-` or `_`."), MenuQuery, ("If-Match" = String, Header, description = "Required: the `ETag` you read, or `*`.")),
    request_body = PutMenuRequest,
    responses(
        (status = 200, description = "The menu as saved.", body = Data<MenuResponse>, headers(("ETag" = String, description = "New version of the menu."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 412, description = "The menu changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "An item is invalid (each listed with its place, e.g. `items[0].children[1].url`); see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn put_menu(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(key): Path<String>,
    Query(query): Query<MenuQuery>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<PutMenuRequest>,
) -> Result<Tagged<MenuResponse>, Error> {
    let expected = if_match.required()?;
    let items = input.items.into_iter().map(ItemInput::from).collect();
    let (menu, nodes) =
        service.put_menu(&principal, &key, query.locale.as_deref(), items, expected).await?;
    Ok(Tagged { version: menu.updated_at, data: menu_response(menu, nodes) })
}
