//! Dữ liệu khối của `content` lúc đọc công khai:
//!
//! - `content.placement`: banner đang chạy của các vị trí, đọc MỘT lần cho mọi khối của trang (như
//!   `GET /public/placements?keys=…`). Vị trí không có hay đang tắt → `None` (giao diện ẩn khối).
//! - `content.type_list`: mục đã đăng của một loại nội dung tự định nghĩa (như
//!   `GET /public/types/{key}/entries`); loại không có, tắt hay không `listable` → `None`.

use std::sync::Arc;

use autoxide_kernel::blocks::{BlockData, BlockRef, BlockRequest};
use autoxide_kernel::custom_fields::CustomFields;
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Order, PageRequest};
use serde_json::{Map, Value, json};

use crate::types::field_target;

use crate::banners::MAX_KEYS;
use crate::banners::public::PageContext;
use crate::http::public_placement;
use crate::service::{ContentService, now};

/// Bộ trả dữ liệu cho khối của `content`.
pub(crate) struct ContentBlocks(pub(crate) Arc<ContentService>);

fn key_of(item: &BlockRef) -> Option<&str> {
    item.props.get("key").and_then(Value::as_str)
}

#[async_trait::async_trait]
impl BlockData for ContentBlocks {
    async fn resolve(
        &self,
        request: &BlockRequest<'_>,
        block: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        if block == super::TYPE_LIST.name {
            return self.type_lists(request, items).await;
        }
        let mut keys: Vec<String> = Vec::new();
        for key in items.iter().filter_map(key_of) {
            if !keys.iter().any(|known| known == key) {
                keys.push(key.to_owned());
            }
        }
        // Trang có nhiều vị trí hơn trần của một lần đọc: đọc theo từng nhóm.
        let page = PageContext {
            kind: Some("landing".to_owned()),
            reference: None,
            locale: Some(request.locale.to_owned()),
        };
        let mut found = Vec::new();
        for chunk in keys.chunks(MAX_KEYS) {
            let read = self
                .0
                .public_placements(request.tenant, request.site, chunk, &page, now(), false)
                .await?;
            found.extend(read.placements.into_iter().map(|ready| {
                let key = ready.placement.key.clone();
                (key, serde_json::to_value(public_placement(ready)).unwrap_or(Value::Null))
            }));
        }
        Ok(items
            .iter()
            .map(|item| {
                let key = key_of(item)?;
                found.iter().find(|(found, _)| found == key).map(|(_, value)| value.clone())
            })
            .collect())
    }
}

/// Số mục mặc định của khối `content.type_list`.
const TYPE_LIST_LIMIT: u32 = 6;

impl ContentBlocks {
    /// Dữ liệu các khối `content.type_list` của trang: mỗi khối một lần đọc danh sách (bộ lọc riêng).
    async fn type_lists(
        &self,
        request: &BlockRequest<'_>,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        let mut found = Vec::with_capacity(items.len());
        for item in items {
            found.push(self.type_list(request, &item.props).await?);
        }
        Ok(found)
    }

    async fn type_list(
        &self,
        request: &BlockRequest<'_>,
        props: &Value,
    ) -> Result<Option<Value>, Error> {
        let Some(key) = props.get("type").and_then(Value::as_str) else { return Ok(None) };
        let filters: Vec<(String, String)> = props
            .get("filters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|filter| {
                let field = filter.get("field")?.as_str()?;
                let value = filter.get("value")?.as_str()?;
                Some((field.to_owned(), value.to_owned()))
            })
            .collect();
        let limit = props
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|limit| u32::try_from(limit).ok())
            .unwrap_or(TYPE_LIST_LIMIT);
        let order = match props.get("sort").and_then(Value::as_str) {
            Some("oldest") => Order::Asc,
            _ => Order::Desc,
        };
        let page = PageRequest::first(limit, order);
        let read = self
            .0
            .public_type_entries(request.tenant, request.site, key, request.locale, &filters, &page)
            .await;
        // Loại không còn công khai hay bộ lọc không còn hợp lệ (trường bị đổi): ẩn khối, trang vẫn trả.
        let (row, page) = match read {
            Ok(found) => found,
            Err(error) if error.kind().status() < 500 => return Ok(None),
            Err(error) => return Err(error),
        };
        let card_fields: Vec<&str> = props
            .get("card_fields")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let public =
            self.0.custom_fields.public_keys(request.tenant, &field_target(&row.key)).await?;
        let entries: Vec<Value> = page
            .data
            .iter()
            .map(|entry| {
                let stored = CustomFields::pick_public(entry.custom_fields.as_deref(), &public);
                let fields: Map<String, Value> = stored
                    .into_iter()
                    .filter(|(key, _)| card_fields.contains(&key.as_str()))
                    .collect();
                json!({
                    "id": entry.entry_id,
                    "title": entry.title,
                    "excerpt": entry.excerpt,
                    // Loại không có trang: đường dẫn nội bộ `~<key>/<slug>` không phải địa chỉ.
                    "path": row.routable.then(|| entry.path.clone()),
                    "featured_media_id": entry.featured_media_id,
                    "published_at": entry.published_at,
                    "fields": fields,
                })
            })
            .collect();
        Ok(Some(json!({
            "type": row.key,
            "labels": serde_json::from_str::<Value>(&row.labels).unwrap_or(Value::Null),
            "entries": entries,
            "has_more": page.pagination.next_cursor.is_some(),
        })))
    }
}
