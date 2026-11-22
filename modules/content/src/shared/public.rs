//! Đọc công khai: thay khối `content.shared_section` bằng nội dung hiện tại của khu, MỘT câu truy vấn
//! cho mọi khu của trang. Khối con của khu mang `id` = `<id khối khu>.<id trong khu>`: dữ liệu khối
//! (`blocks_data`) theo `id` không trùng giữa hai lần đặt cùng một khu. Khu đã xoá → khối không có
//! con (giao diện bỏ qua).

use std::collections::HashMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::Value;
use uuid::Uuid;

use super::KIND;
use crate::entity::shared_section;
use crate::service::ContentService;

fn section_nodes(body: &mut Value) -> Vec<&mut Value> {
    let mut found = Vec::new();
    let mut pending: Vec<&mut Value> = match body.get_mut("sections") {
        Some(Value::Array(sections)) => sections.iter_mut().collect(),
        _ => Vec::new(),
    };
    while let Some(node) = pending.pop() {
        if node.get("block").and_then(Value::as_str) == Some(KIND) {
            found.push(node);
            continue;
        }
        if let Some(Value::Array(children)) = node.get_mut("children") {
            pending.extend(children.iter_mut());
        }
    }
    found
}

/// Đổi `id` của mọi khối trong `nodes` thành `<prefix>.<id>`.
fn prefix_ids(nodes: &mut [Value], prefix: &str) {
    for node in nodes {
        if let Some(Value::String(id)) = node.get_mut("id") {
            *id = format!("{prefix}.{id}");
        }
        if let Some(Value::Array(children)) = node.get_mut("children") {
            prefix_ids(children, prefix);
        }
    }
}

impl ContentService {
    /// Thay khối khu dùng chung trong bố cục `body` bằng nội dung khu (làm tại chỗ).
    pub(crate) async fn expand_shared_sections(
        &self,
        tenant: &TenantContext,
        body: &mut Value,
    ) -> Result<(), Error> {
        let ids: Vec<Uuid> = section_nodes(body)
            .iter()
            .filter_map(|node| node.pointer("/props/id").and_then(Value::as_str))
            .filter_map(|id| Uuid::parse_str(id).ok())
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        let sections: HashMap<Uuid, Value> = shared_section::Entity::find()
            .scoped(tenant)
            .filter(shared_section::Column::Id.is_in(ids))
            .all(tenant.database().read())
            .await?
            .into_iter()
            .filter_map(|row| {
                let layout: Value = serde_json::from_str(&row.layout).ok()?;
                Some((row.id, layout.get("sections").cloned().unwrap_or(Value::Array(Vec::new()))))
            })
            .collect();
        for node in section_nodes(body) {
            let id = node
                .pointer("/props/id")
                .and_then(Value::as_str)
                .and_then(|id| Uuid::parse_str(id).ok());
            let prefix = node.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
            let mut children = match id.and_then(|id| sections.get(&id)) {
                Some(Value::Array(children)) => children.clone(),
                _ => Vec::new(),
            };
            prefix_ids(&mut children, &prefix);
            if let Some(fields) = node.as_object_mut() {
                fields.insert("children".to_owned(), Value::Array(children));
            }
        }
        Ok(())
    }
}
