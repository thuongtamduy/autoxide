//! Chính sách tách / gộp của bảng cấp khách của module nội bộ `kernel`, và hai bảng kernel tự làm:
//!
//! - Cây tổ chức (`kernel_org_units` + `kernel_org_closure`): tách → nút gốc nhánh thành gốc của
//!   khách mới (độ sâu trừ đi, bảng tổ tiên – con cháu chỉ giữ cặp trong nhánh); gộp → gốc của khách
//!   bị gộp thành con của nút nhận (độ sâu cộng thêm, thêm cặp với mọi tổ tiên của nút nhận).
//! - Dãy số (`kernel_sequences`): dãy có tên chứa ID của thứ đi theo (nút, sổ, ký hiệu hoá đơn…)
//!   đi cùng — số tiếp theo vẫn liên tục ở khách mới.

use std::sync::Arc;

use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, Value};
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};
use uuid::Uuid;

use super::{RelocateTable, Relocation, RelocationContext, RelocationKind, TableReport};
use crate::error::Error;
use crate::tenancy::org_tree::{closure, unit};
use crate::transfer::{TransferError, tenant_is};

fn transfer(error: TransferError) -> Error {
    Error::internal(error.to_string())
}

/// Chính sách của các bảng cấp khách của `kernel`.
pub(crate) fn kernel_relocation() -> Relocation {
    Relocation::new()
        .custom("kernel_org_units", Arc::new(OrgTreeTables))
        .handled_by("kernel_org_closure", "kernel_org_units")
        .custom("kernel_sequences", Arc::new(Sequences))
        .by_org(&["kernel_job_runs", "kernel_saved_views", "kernel_search_docs"])
        .follow("kernel_search_filters", "doc_id", "kernel_search_docs")
        // Cấu hình của khách: khách mới bắt đầu bằng bản sao, gộp thì giữ bản của khách nhận.
        .copy_on_split(&[
            "kernel_reason_codes",
            "kernel_tags",
            "kernel_text_overrides",
            "kernel_custom_fields",
        ])
        // Cài đặt (có secret của đối tác): ở lại khách nguồn — khách mới tự kết nối đối tác của mình.
        .stay(&["kernel_tenant_settings", "kernel_record_notes", "kernel_record_tags"])
        .sealed("kernel_tenant_settings", "value", crate::tenancy::settings::PURPOSE, "name")
        .ephemeral(&[
            "kernel_user_flags",
            "kernel_work_states",
            "kernel_approval_requests",
            "kernel_connection_status",
            "kernel_pins",
            "kernel_request_log",
            "kernel_throttle",
        ])
}

/// Cây tổ chức của kernel.
struct OrgTreeTables;

impl OrgTreeTables {
    /// Dòng nút của khách nguồn sẽ đi.
    async fn units(&self, context: &RelocationContext) -> Result<Vec<unit::Model>, Error> {
        let mut select = unit::Entity::find();
        select = match context.kind() {
            RelocationKind::Merge => {
                select.filter(unit::Column::TenantId.eq(context.source_tenant().as_uuid()))
            }
            RelocationKind::Split => select.filter(context.branch_condition("id")),
        };
        Ok(select.all(context.source().write()).await?)
    }
}

#[async_trait::async_trait]
impl RelocateTable for OrgTreeTables {
    async fn plan(&self, context: &RelocationContext) -> Result<TableReport, Error> {
        Ok(TableReport::rows(self.units(context).await?.len() as u64))
    }

    async fn copy(
        &self,
        context: &RelocationContext,
        target: &DatabaseTransaction,
    ) -> Result<u64, Error> {
        let to = context.target_tenant().as_uuid();
        let units = self.units(context).await?;
        let mut closures = closure::Entity::find()
            .filter(closure::Column::TenantId.eq(context.source_tenant().as_uuid()))
            .all(context.source().write())
            .await?;
        let (units, rows) = match context.kind() {
            RelocationKind::Split => {
                let root = context.org().as_uuid();
                let base = units.iter().find(|row| row.id == root).map_or(0, |row| row.depth);
                let inside: std::collections::HashSet<Uuid> =
                    units.iter().map(|row| row.id).collect();
                closures.retain(|row| {
                    inside.contains(&row.ancestor_id) && inside.contains(&row.descendant_id)
                });
                let units: Vec<unit::Model> = units
                    .into_iter()
                    .map(|row| unit::Model {
                        tenant_id: to,
                        parent_id: if row.id == root { None } else { row.parent_id },
                        depth: row.depth - base,
                        ..row
                    })
                    .collect();
                (units, closures)
            }
            RelocationKind::Merge => {
                // Gốc của khách bị gộp (ID = ID khách) thành con của nút nhận `org`.
                let root = context.source_tenant().as_uuid();
                let under = context.org().as_uuid();
                let parent =
                    unit::Entity::find_by_id((to, under)).one(target).await?.ok_or_else(|| {
                        Error::internal(format!("unit {under} is gone from the target"))
                    })?;
                let above = closure::Entity::find()
                    .filter(closure::Column::TenantId.eq(to))
                    .filter(closure::Column::DescendantId.eq(under))
                    .all(target)
                    .await?;
                let units: Vec<unit::Model> = units
                    .into_iter()
                    .map(|row| unit::Model {
                        tenant_id: to,
                        parent_id: if row.id == root { Some(under) } else { row.parent_id },
                        depth: row.depth + parent.depth + 1,
                        ..row
                    })
                    .collect();
                let below: Vec<(Uuid, i32)> = closures
                    .iter()
                    .filter(|row| row.ancestor_id == root)
                    .map(|row| (row.descendant_id, row.depth))
                    .collect();
                for ancestor in &above {
                    for (descendant, depth) in &below {
                        closures.push(closure::Model {
                            tenant_id: to,
                            ancestor_id: ancestor.ancestor_id,
                            descendant_id: *descendant,
                            depth: ancestor.depth + 1 + depth,
                        });
                    }
                }
                (units, closures)
            }
        };
        let written = units.len() as u64;
        for chunk in units.chunks(100) {
            let models = chunk.iter().cloned().map(Into::<unit::ActiveModel>::into);
            unit::Entity::insert_many(models)
                .on_conflict_do_nothing()
                .exec_without_returning(target)
                .await?;
        }
        for chunk in rows.chunks(100) {
            let models = chunk
                .iter()
                .map(|row| closure::Model { tenant_id: to, ..*row })
                .map(Into::<closure::ActiveModel>::into);
            closure::Entity::insert_many(models)
                .on_conflict_do_nothing()
                .exec_without_returning(target)
                .await?;
        }
        Ok(written)
    }

    async fn clean(
        &self,
        context: &RelocationContext,
        source: &DatabaseTransaction,
    ) -> Result<u64, Error> {
        let from = context.source_tenant().as_uuid();
        let ids: Vec<Uuid> = match context.kind() {
            RelocationKind::Merge => Vec::new(),
            RelocationKind::Split => closure::Entity::find()
                .filter(closure::Column::TenantId.eq(from))
                .filter(closure::Column::AncestorId.eq(context.org().as_uuid()))
                .all(source)
                .await?
                .into_iter()
                .map(|row| row.descendant_id)
                .collect(),
        };
        let mut deleted = 0;
        if context.kind() == RelocationKind::Merge {
            deleted += closure::Entity::delete_many()
                .filter(closure::Column::TenantId.eq(from))
                .exec(source)
                .await?
                .rows_affected;
            deleted += unit::Entity::delete_many()
                .filter(unit::Column::TenantId.eq(from))
                .exec(source)
                .await?
                .rows_affected;
            return Ok(deleted);
        }
        for chunk in ids.chunks(500) {
            deleted += closure::Entity::delete_many()
                .filter(closure::Column::TenantId.eq(from))
                .filter(closure::Column::DescendantId.is_in(chunk.iter().copied()))
                .exec(source)
                .await?
                .rows_affected;
            deleted += unit::Entity::delete_many()
                .filter(unit::Column::TenantId.eq(from))
                .filter(unit::Column::Id.is_in(chunk.iter().copied()))
                .exec(source)
                .await?
                .rows_affected;
        }
        Ok(deleted)
    }
}

/// Dãy số đi theo thứ nó đánh số.
struct Sequences;

const SEQUENCES: &str = crate::numbering::TABLE;

/// Mọi UUID viết trong `text`.
fn uuids_in(text: &str) -> Vec<Uuid> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut start = 0;
    while start + 36 <= bytes.len() {
        match text.get(start..start + 36).and_then(|part| part.parse::<Uuid>().ok()) {
            Some(id) => {
                found.push(id);
                start += 36;
            }
            None => start += 1,
        }
    }
    found
}

impl Sequences {
    /// Tên các dãy của khách nguồn sẽ đi.
    async fn scopes<C: sea_orm::ConnectionTrait>(
        &self,
        context: &RelocationContext,
        connection: &C,
    ) -> Result<Vec<String>, Error> {
        let rows = context
            .tables()
            .values(
                connection,
                SEQUENCES,
                &["scope".to_owned()],
                tenant_is(context.source_tenant()),
            )
            .await
            .map_err(transfer)?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.into_iter().next().flatten())
            .filter(|scope| uuids_in(scope).into_iter().any(|id| context.moved(id)))
            .collect())
    }

    fn condition(context: &RelocationContext, scopes: &[String]) -> Condition {
        tenant_is(context.source_tenant())
            .add(Expr::col(Alias::new("scope")).is_in(scopes.iter().map(String::as_str)))
    }
}

#[async_trait::async_trait]
impl RelocateTable for Sequences {
    async fn plan(&self, context: &RelocationContext) -> Result<TableReport, Error> {
        Ok(TableReport::rows(self.scopes(context, context.source().write()).await?.len() as u64))
    }

    async fn copy(
        &self,
        context: &RelocationContext,
        target: &DatabaseTransaction,
    ) -> Result<u64, Error> {
        let mut written = 0;
        let to = context.target_tenant().as_uuid();
        for chunk in self.scopes(context, context.source().write()).await?.chunks(200) {
            let mut rows = context
                .tables()
                .rows(context.source().write(), SEQUENCES, Self::condition(context, chunk))
                .await
                .map_err(transfer)?;
            for row in &mut rows {
                row.set("tenant_id", Value::Uuid(Some(to)));
            }
            written += context
                .tables()
                .insert_rows(target, SEQUENCES, rows, true)
                .await
                .map_err(transfer)?;
        }
        Ok(written)
    }

    async fn clean(
        &self,
        context: &RelocationContext,
        source: &DatabaseTransaction,
    ) -> Result<u64, Error> {
        let mut deleted = 0;
        for chunk in self.scopes(context, source).await?.chunks(200) {
            deleted += context
                .tables()
                .delete_matching(source, SEQUENCES, Self::condition(context, chunk))
                .await
                .map_err(transfer)?;
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tim_uuid_trong_ten_day() {
        let id = Uuid::now_v7();
        assert_eq!(uuids_in(&format!("invoices:{id}")), [id]);
        assert_eq!(uuids_in(&format!("a:{id}:b:{id}")), [id, id]);
        assert!(uuids_in("orders:2026").is_empty());
    }
}
