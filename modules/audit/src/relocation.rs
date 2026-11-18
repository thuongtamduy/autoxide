//! Audit log khi tách / gộp khách (bước 38, D-20261118-step38-relocation).
//!
//! Chuỗi băm không di chuyển được dòng (đổi khách của một dòng làm gãy cả hai chuỗi), nên audit tự
//! làm phần của mình: các dòng **về thứ đi theo** (`subject` chứa ID của một nút trong nhánh hay của
//! một dòng đi / được chép) được **ghi lại** vào chuỗi của khách nhận bằng đúng cách băm của bộ ghi —
//! giữ `id`, `occurred_at`, `recorded_at`, phần dữ liệu — còn dòng gốc ở lại nguồn, nên chuỗi nguồn
//! không gãy. Gộp: mọi dòng của khách bị gộp được ghi lại vào chuỗi của khách nhận, trừ dòng mà khách
//! nhận đã có (cùng `id` — bản đã ghi lại khi tách: cùng một lịch sử).

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{RelocateTable, RelocationContext, RelocationKind, TableReport};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, Set,
};
use uuid::Uuid;

use crate::chain::{self, Link};
use crate::entity;

/// Chính sách tách / gộp của các bảng của `audit`.
pub(crate) fn relocation() -> autoxide_kernel::tenancy::Relocation {
    autoxide_kernel::tenancy::Relocation::new()
        .custom("audit_entry", std::sync::Arc::new(AuditRelocation))
        .handled_by("audit_chain", "audit_entry")
}

struct AuditRelocation;

/// Mọi UUID viết trong `text`.
fn uuids_in(text: &str) -> Vec<Uuid> {
    let mut found = Vec::new();
    let mut start = 0;
    while start + 36 <= text.len() {
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

impl AuditRelocation {
    /// Dòng của khách nguồn về thứ đi theo, theo thứ tự trong chuỗi.
    async fn entries(&self, context: &RelocationContext) -> Result<Vec<entity::Model>, Error> {
        let rows = entity::Entity::find()
            .filter(entity::Column::TenantId.eq(context.source_tenant().as_uuid()))
            .order_by_asc(entity::Column::Sequence)
            .order_by_asc(entity::Column::Id)
            .all(context.source().write())
            .await?;
        Ok(match context.kind() {
            RelocationKind::Merge => rows,
            RelocationKind::Split => rows
                .into_iter()
                .filter(|row| {
                    row.subject.as_deref().is_some_and(|subject| {
                        uuids_in(subject).into_iter().any(|id| context.moved(id))
                    })
                })
                .collect(),
        })
    }
}

#[async_trait::async_trait]
impl RelocateTable for AuditRelocation {
    async fn plan(&self, context: &RelocationContext) -> Result<TableReport, Error> {
        Ok(TableReport::rows(self.entries(context).await?.len() as u64))
    }

    async fn copy(
        &self,
        context: &RelocationContext,
        target: &DatabaseTransaction,
    ) -> Result<u64, Error> {
        let tenant = context.target_tenant().as_uuid();
        chain::seed_head(target, tenant).await?;
        let mut head = chain::lock_head(target, tenant).await?;
        let mut written = 0;
        for row in self.entries(context).await? {
            // Khoá chính chỉ là `id`: dòng đã có ở đích (ghi lại khi tách) là cùng một lịch sử.
            if entity::Entity::find_by_id(row.id).one(target).await?.is_some() {
                continue;
            }
            let payload = match &row.payload_hash {
                Some(hash) => hash.clone(),
                None => chain::payload_hash(
                    row.before.as_deref(),
                    row.after.as_deref(),
                    row.details.as_deref(),
                ),
            };
            let sequence = head.sequence + 1;
            let moved = entity::Model { tenant_id: tenant, ..row };
            let hash = chain::chain_hash(&head.hash, &Link::of(&moved, sequence, &payload));
            let mut active = moved.into_active_model();
            active.sequence = Set(Some(sequence));
            active.payload_hash = Set(Some(payload));
            active.chain_hash = Set(Some(hash.clone()));
            active.insert(target).await?;
            head.sequence = sequence;
            head.hash = hash;
            written += 1;
        }
        chain::advance(target, tenant, head.sequence, &head.hash).await?;
        Ok(written)
    }

    async fn clean(
        &self,
        _context: &RelocationContext,
        _source: &DatabaseTransaction,
    ) -> Result<u64, Error> {
        // Dòng gốc ở lại nguồn: chuỗi của nguồn không gãy (Luật Kế toán, D-183).
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tim_uuid_trong_subject() {
        let id = Uuid::now_v7();
        assert_eq!(uuids_in(&format!("notes.note:{id}")), [id]);
        assert!(uuids_in("kernel.setting:public_url").is_empty());
    }
}
