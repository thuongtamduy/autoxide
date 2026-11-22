//! Khối `forms.embed` cho trang dựng từ khối (bước 67): nhúng một form theo `key`, dữ liệu như
//! `GET /api/v1/forms/public/{key}` (trường, nhãn theo ngôn ngữ, trạng thái — form đóng vẫn trả để
//! website báo "đã đóng"). Mọi form của trang đọc trong MỘT câu truy vấn; form không có → `None`.

use std::sync::Arc;

use autoxide_kernel::blocks::{BlockCategory, BlockData, BlockRef, BlockRequest, BlockType};
use autoxide_kernel::error::Error;
use serde_json::Value;

use crate::forms::fields::Locale;
use crate::http::PublicFormResponse;
use crate::service::FormsService;

pub(crate) const EMBED: BlockType = BlockType::new(
    "forms.embed",
    BlockCategory::Lead,
    "Form",
    "Use to put a form of the website on the page (contact, quote request, registration) by its key.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["key"],
        "properties": {
            "key": {"type": "string", "minLength": 1, "maxLength": 64, "description": "Key of a form, e.g. `contact`."},
            "show_title": {"type": "boolean"},
            "button_label": {"type": "string", "maxLength": 60, "description": "Replaces the form's own submit label on this page."}
        }
    }"#,
)
.with_data();

pub(crate) fn all() -> Vec<BlockType> {
    vec![EMBED]
}

/// Bộ trả dữ liệu cho `forms.embed`.
pub(crate) struct FormBlocks(pub(crate) Arc<FormsService>);

fn key_of(item: &BlockRef) -> Option<&str> {
    item.props.get("key").and_then(Value::as_str).map(str::trim)
}

#[async_trait::async_trait]
impl BlockData for FormBlocks {
    async fn resolve(
        &self,
        request: &BlockRequest<'_>,
        _block: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        let keys: Vec<String> = items.iter().filter_map(key_of).map(str::to_owned).collect();
        let locale = Locale::parse(request.locale).unwrap_or_default();
        let found: Vec<(String, Value)> = self
            .0
            .find_by_keys(request.tenant, &keys)
            .await?
            .into_iter()
            .map(|form| {
                let value = serde_json::to_value(PublicFormResponse::new(&form, locale))
                    .unwrap_or(Value::Null);
                (form.row.key.clone(), value)
            })
            .collect();
        Ok(items
            .iter()
            .map(|item| {
                let key = key_of(item)?;
                found.iter().find(|(known, _)| known == key).map(|(_, value)| value.clone())
            })
            .collect())
    }
}
