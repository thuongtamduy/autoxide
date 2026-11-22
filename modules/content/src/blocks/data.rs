//! Dữ liệu của khối `content.placement` lúc đọc công khai: banner đang chạy của các vị trí, đọc
//! MỘT lần cho mọi khối của trang (như `GET /public/placements?keys=…`). Vị trí không có hay đang
//! tắt → `None` (giao diện ẩn khối).

use std::sync::Arc;

use autoxide_kernel::blocks::{BlockData, BlockRef, BlockRequest};
use autoxide_kernel::error::Error;
use serde_json::Value;

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
        _block: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
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
