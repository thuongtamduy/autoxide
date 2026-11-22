//! Khối `menus.menu` cho trang dựng từ khối (bước 67, D-238): menu của website theo `key`, dữ liệu
//! như `GET /api/v1/menus/public/{key}` (cây đã giải, nhãn theo ngôn ngữ). Menu đọc qua cache trong
//! bộ nhớ của module nên mỗi khoá khác nhau tốn nhiều nhất một lần đọc database; menu không có →
//! `None` (giao diện ẩn khối).

use std::sync::Arc;

use autoxide_kernel::blocks::{BlockCategory, BlockData, BlockRef, BlockRequest, BlockType};
use autoxide_kernel::error::Error;
use autoxide_kernel::time::Timestamp;
use serde_json::Value;

use crate::service::MenuService;

pub(crate) const MENU: BlockType = BlockType::new(
    "menus.menu",
    BlockCategory::Navigation,
    "Menu",
    "Use to show a menu of the website by its key, usually `main` in the header and `footer` in the footer.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["key"],
        "properties": {
            "key": {"type": "string", "minLength": 1, "maxLength": 64, "description": "Key of a menu, e.g. `main`."},
            "layout": {"enum": ["horizontal", "vertical", "dropdown"], "x-responsive": true},
            "depth": {"type": "integer", "minimum": 1, "maximum": 3, "description": "Levels shown."}
        }
    }"#,
)
.with_data();

pub(crate) fn all() -> Vec<BlockType> {
    vec![MENU]
}

/// Bộ trả dữ liệu cho `menus.menu`.
pub(crate) struct MenuBlocks(pub(crate) Arc<MenuService>);

fn key_of(item: &BlockRef) -> Option<&str> {
    item.props.get("key").and_then(Value::as_str)
}

#[async_trait::async_trait]
impl BlockData for MenuBlocks {
    async fn resolve(
        &self,
        request: &BlockRequest<'_>,
        _block: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        let now = Timestamp::now();
        let mut found: Vec<(&str, Option<Value>)> = Vec::new();
        for key in items.iter().filter_map(key_of) {
            if found.iter().any(|(known, _)| *known == key) {
                continue;
            }
            // Menu không có (404) chỉ ẩn khối của nó; lỗi khác (database) báo lên như mọi khối.
            let menu = match self
                .0
                .public_menu(request.tenant, request.site, key, Some(request.locale), now)
                .await
            {
                Ok(ready) => serde_json::to_value(&ready.menu).ok(),
                Err(error) if error.code() == "menus.public_menu_not_found" => None,
                Err(error) => return Err(error),
            };
            found.push((key, menu));
        }
        Ok(items
            .iter()
            .map(|item| {
                let key = key_of(item)?;
                found.iter().find(|(known, _)| *known == key).and_then(|(_, menu)| menu.clone())
            })
            .collect())
    }
}
