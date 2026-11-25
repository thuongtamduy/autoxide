//! Trang chi tiết dựng sẵn của loại nội dung tự định nghĩa (bước 67, D-235, D-20261123-content-types-later):
//! loại có trang giữ một bố cục mẫu (`detail_layout`) mọi mục của loại dùng chung. Khối
//! `content.entry_field {field}` hiện một trường của mục đang xem — trường có sẵn (`title`,
//! `excerpt`…) hay trường tự định nghĩa `public` của loại. Dữ liệu của khối do `content` điền lúc đọc
//! công khai (nó biết mục đang xem), không qua `BlockData` của kernel.

use std::collections::{BTreeMap, BTreeSet};

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::{ScopeExt, Site, TenantContext};
use sea_orm::EntityTrait;
use serde_json::{Value, json};

use super::{ContentType, field_target};
use crate::blocks::ENTRY_FIELD;
use crate::entries::layout;
use crate::publishing::blocks::PublicLayout;
use crate::service::ContentService;

/// Số loại nội dung tối đa đưa cho mô hình AI dựng trang.
const TYPES_FOR_AI: u64 = 20;

/// Trường có sẵn của mục mà khối `content.entry_field` hiện được (tên như trong
/// `GET /public/resolve`).
pub(crate) const BUILTIN_FIELDS: [&str; 6] =
    ["title", "excerpt", "body", "featured_media", "published_at", "authors"];

impl ContentService {
    /// Kiểm bố cục trang chi tiết `value` của loại `row` (lỗi vào `problems`, đường dẫn
    /// `detail_layout…`); hợp lệ thì trả chữ JSON để lưu. Chỉ loại có trang; khối
    /// `content.entry_field` chỉ nhận trường có sẵn hay trường tự định nghĩa đang bật của loại.
    pub(crate) async fn check_detail_layout(
        &self,
        tenant: &TenantContext,
        row: &ContentType,
        value: &Value,
        problems: &mut Vec<FieldError>,
    ) -> Result<Option<String>, Error> {
        if !row.routable {
            problems.push(FieldError::new(
                "detail_layout",
                "not_allowed",
                "only a content type with pages (`routable`) has a detail layout",
            ));
            return Ok(None);
        }
        let before = problems.len();
        let usable = self.usable_blocks(tenant).await?;
        let embed_hosts = self.embed_hosts(tenant).await?;
        if let Err(errors) = layout::check(value, &self.blocks, &usable, &embed_hosts) {
            problems.extend(errors.into_iter().map(|mut error| {
                error.field = error.field.replacen("body", "detail_layout", 1);
                error
            }));
        }
        let known: BTreeSet<String> = if row.key.is_empty() {
            BTreeSet::new()
        } else {
            self.custom_fields
                .definitions(tenant, &field_target(&row.key))
                .await?
                .into_iter()
                .map(|field| field.key)
                .collect()
        };
        for (name, id, props) in layout::blocks_of(value) {
            if name != ENTRY_FIELD.name {
                continue;
            }
            let field = props.get("field").and_then(Value::as_str).unwrap_or_default();
            if !BUILTIN_FIELDS.contains(&field) && !known.contains(field) {
                problems.push(
                    FieldError::new(
                        "detail_layout",
                        "unknown_field",
                        "block `{id}` shows `{field}`, which is neither {builtin} nor an active custom field of this type",
                    )
                    .with_param("id", id)
                    .with_param("field", field)
                    .with_param("builtin", BUILTIN_FIELDS.join(", ")),
                );
            }
        }
        Ok((problems.len() == before).then(|| value.to_string()))
    }

    /// Bố cục trang chi tiết của loại `type_id` (mục của loại tự định nghĩa), khi loại đang bật, có
    /// trang và có bố cục mẫu.
    pub(crate) async fn type_detail(
        &self,
        tenant: &TenantContext,
        type_id: Option<uuid::Uuid>,
    ) -> Result<Option<String>, Error> {
        let Some(id) = type_id else { return Ok(None) };
        let row = crate::entity::content_type::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        Ok(row.filter(|row| row.active && row.routable).and_then(|row| row.detail_layout))
    }

    /// Loại đang bật và `listable` cho mô hình AI dựng trang: khoá, nhãn, trường công khai (và trường
    /// lọc được). Tối đa [`TYPES_FOR_AI`] loại.
    pub(crate) async fn types_for_ai(&self, tenant: &TenantContext) -> Result<Vec<Value>, Error> {
        use sea_orm::{ColumnTrait, QueryFilter, QuerySelect};
        let rows = crate::entity::content_type::Entity::find()
            .scoped(tenant)
            .filter(crate::entity::content_type::Column::Active.eq(true))
            .filter(crate::entity::content_type::Column::Listable.eq(true))
            .limit(TYPES_FOR_AI)
            .all(tenant.database().read())
            .await?;
        let mut found = Vec::with_capacity(rows.len());
        for row in rows {
            let fields = self.custom_fields.definitions(tenant, &field_target(&row.key)).await?;
            let public: Vec<Value> = fields
                .iter()
                .filter(|field| field.visibility == "public")
                .map(|field| json!({"key": field.key, "filterable": field.filterable, "labels": field.labels}))
                .collect();
            found.push(json!({
                "key": row.key,
                "labels": serde_json::from_str::<Value>(&row.labels).unwrap_or(Value::Null),
                "public_fields": public,
            }));
        }
        Ok(found)
    }

    /// Trang chi tiết của mục `entry` (đã ở dạng như `GET /public/resolve` trả) theo bố cục mẫu
    /// `detail` của loại: bố cục công khai và dữ liệu khối, kèm giá trị của mỗi khối
    /// `content.entry_field` (`{"field", "value"}`; trường không có giá trị → `value: null`).
    pub(crate) async fn detail_page(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: &str,
        detail: &str,
        entry: &Value,
    ) -> Result<Option<PublicLayout>, Error> {
        let Some(mut page) = self.public_layout(tenant, site, locale, detail).await? else {
            return Ok(None);
        };
        let mut values = BTreeMap::new();
        for (name, id, props) in layout::blocks_of(&page.body) {
            if name != ENTRY_FIELD.name {
                continue;
            }
            let field = props.get("field").and_then(Value::as_str).unwrap_or_default();
            values
                .insert(id.to_owned(), json!({"field": field, "value": field_value(entry, field)}));
        }
        page.data.extend(values);
        Ok(Some(page))
    }
}

/// Giá trị trường `field` của mục `entry`: trường có sẵn, rồi trường tự định nghĩa công khai.
fn field_value(entry: &Value, field: &str) -> Value {
    let found = if BUILTIN_FIELDS.contains(&field) {
        entry.get(field)
    } else {
        entry.get("custom_fields").and_then(|fields| fields.get(field))
    };
    found.cloned().unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gia_tri_truong_co_san_va_tu_dinh_nghia() {
        let entry = json!({"title": "Ada", "custom_fields": {"role": "CTO"}});
        assert_eq!(field_value(&entry, "title"), json!("Ada"));
        assert_eq!(field_value(&entry, "role"), json!("CTO"));
        assert_eq!(field_value(&entry, "missing"), Value::Null);
        // Trường tự định nghĩa trùng tên trường có sẵn không thay được trường có sẵn.
        assert_eq!(field_value(&json!({"custom_fields": {"title": "x"}}), "title"), Value::Null);
    }
}
