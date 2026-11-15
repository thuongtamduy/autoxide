//! Tool MCP của module `menus`. Mỗi tool: một kiểu đầu vào (kiểm như thân JSON của REST),
//! permission như endpoint REST tương ứng, thân là MỘT lời gọi service — không có logic riêng
//! (luật kiến trúc 8). Tool thay mục nhận `version` (giá trị `ETag` mà tool đọc trả về) và kiểm
//! y như `If-Match` của REST.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, etag};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::http::{SiteMenu, SiteMenuItemInput, SiteMenuSummary, menu_response};
use crate::items::ItemInput;
use crate::permissions::{MENUS_MANAGE, MENUS_READ};
use crate::service::{MenuService, NewMenu};

/// Menu kèm `version` (giá trị `ETag`) để gửi lại khi thay mục.
#[derive(Debug, Serialize)]
struct Versioned {
    version: String,
    #[serde(flatten)]
    menu: SiteMenu,
}

/// A menu to act on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MenuId {
    /// Id of the menu.
    id: Uuid,
}

// Hình dạng một mục menu, chép vào mô tả của tool tạo / thay mục (client MCP không đọc được
// `$ref`, nên mục nhận dạng JSON thô rồi kiểm như thân REST). Macro vì `concat!` chỉ nhận chữ.
macro_rules! item_shape {
    () => {
        " Each item is an object: `label` (up to 200 characters; may be empty with `ref` to show \
         the object's title); at most one of `url` (`https://`, `http://`, `mailto:` or a path \
         starting with one `/`) and `ref` (`{\"kind\": \"content.term\", \"id\": \"...\"}`, an \
         object of any module), with neither the item is a heading; `auto_children` (true to \
         also list the object's sub-items) with `auto_children_limit` (1 to 50, default 12); \
         `icon`; `highlight` (`accent`, `brand`, `sale`, `muted`); `badge` (`{\"text\", \"tone\": \
         sale|brand|new|neutral, \"from\"?, \"until\"?}`); `feature` (an object to feature, \
         same shape as `ref`); `children` (same shape; 4 levels and 200 items at most)."
    };
}

/// A new website menu.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateMenuInput {
    /// 1 to 64 lower-case letters, digits, `-` or `_`, e.g. `main` or `footer`.
    key: String,
    /// The menu's language, e.g. `vi`; omitted: the default language.
    locale: Option<String>,
    /// The item tree (see the tool description for the shape of an item).
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    items: Vec<Value>,
}

/// The new item tree of a menu.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplaceItemsInput {
    /// Id of the menu.
    id: Uuid,
    /// The `version` returned by `get_menu` (or `*` to replace whatever the version).
    version: String,
    /// The whole new item tree (see the tool description for the shape of an item).
    #[schema(value_type = Vec<Object>)]
    items: Vec<Value>,
}

/// Mục dạng JSON thô → mục đã đọc, kiểm như thân REST; sai dạng thì lỗi kiểm dữ liệu.
fn parse_items(items: Vec<Value>) -> Result<Vec<ItemInput>, Error> {
    let parsed: Vec<SiteMenuItemInput> =
        serde_json::from_value(Value::Array(items)).map_err(|error| {
            Error::validation(vec![FieldError::new("items", "invalid", error.to_string())])
        })?;
    Ok(parsed.into_iter().map(ItemInput::from).collect())
}

pub(crate) fn all(service: &Arc<MenuService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let (list, get, create, replace, delete) =
        (service.clone(), service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "list_menus",
            "List the website menus (every key and language), newest first; follow \
             `pagination.next_cursor` for more.",
            move |_tenant: TenantContext, principal: Principal, input: PageInput| {
                let service = list.clone();
                async move {
                    let page = service.list(&principal, &input.page()?).await?;
                    Ok::<Page<SiteMenuSummary>, Error>(page.map(SiteMenuSummary::from))
                }
            },
        )
        .requires(MENUS_READ)
        .read_only(),
        Tool::new(
            "get_menu",
            "Get a website menu with its items as saved, and its `version` for `replace_menu_items`.",
            move |_tenant: TenantContext, principal: Principal, input: MenuId| {
                let service = get.clone();
                async move {
                    let (menu, nodes) = service.get(&principal, input.id).await?;
                    let version = etag(menu.updated_at);
                    Ok::<_, Error>(Versioned { version, menu: menu_response(menu, nodes) })
                }
            },
        )
        .requires(MENUS_READ)
        .read_only(),
        Tool::new(
            "create_menu",
            concat!(
                "Create a website menu: a key (e.g. `main`, `footer`), a language and an item tree.",
                item_shape!()
            ),
            move |_tenant: TenantContext, principal: Principal, input: CreateMenuInput| {
                let service = create.clone();
                async move {
                    let new = NewMenu { key: input.key, locale: input.locale, items: parse_items(input.items)? };
                    let (menu, nodes) = service.create(&principal, new).await?;
                    let version = etag(menu.updated_at);
                    Ok::<_, Error>(Versioned { version, menu: menu_response(menu, nodes) })
                }
            },
        )
        .requires(MENUS_MANAGE),
        Tool::new(
            "replace_menu_items",
            concat!(
                "Replace the whole item tree of a website menu. Send the `version` from \
                 `get_menu`; if the menu changed since, nothing is saved (`precondition_failed`): \
                 read it again.",
                item_shape!()
            ),
            move |_tenant: TenantContext, principal: Principal, input: ReplaceItemsInput| {
                let service = replace.clone();
                async move {
                    let expected = IfMatch::from_value(&input.version).expected()?;
                    let items = parse_items(input.items)?;
                    let (menu, nodes) = service.replace(&principal, input.id, items, expected).await?;
                    let version = etag(menu.updated_at);
                    Ok::<_, Error>(Versioned { version, menu: menu_response(menu, nodes) })
                }
            },
        )
        .requires(MENUS_MANAGE),
        Tool::new(
            "delete_menu",
            "Delete a website menu and all its items. It cannot be restored.",
            move |_tenant: TenantContext, principal: Principal, input: MenuId| {
                let service = delete.clone();
                async move {
                    service.delete(&principal, input.id).await?;
                    Ok::<_, Error>(serde_json::json!({ "deleted": input.id }))
                }
            },
        )
        .requires(MENUS_MANAGE)
        .destructive(),
    ]
}
