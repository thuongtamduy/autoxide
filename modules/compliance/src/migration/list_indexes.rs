//! Index cho danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): mỗi danh sách duyệt bằng
//! keyset có index cột lọc + cột sắp + id, để database nhảy thẳng tới điểm bắt đầu trang. Không index cũ
//! nào là tiền tố của index mới, nên không xoá index nào.

use autoxide_kernel::migration::prelude::*;

pub(super) struct ListIndexes;

impl MigrationName for ListIndexes {
    fn name(&self) -> &str {
        "m0022_list_indexes"
    }
}

/// `(bảng, tên index, cột)` thêm mới.
const ADDED: [(&str, &str, &[&str]); 3] = [
    // `GET /documents`: theo loại rồi id.
    ("compliance_documents", "compliance_documents_page_idx", &["tenant_id", "kind", "id"]),
    // `GET /documents/{id}/acceptance-stats`: `GROUP BY version` của một văn bản, keyset theo phiên bản.
    (
        "compliance_acceptances",
        "compliance_acceptances_stats_idx",
        &["tenant_id", "document_id", "version"],
    ),
    // `GET /me/consents`: theo mục đích rồi id.
    (
        "compliance_consents",
        "compliance_consents_subject_page_idx",
        &["tenant_id", "subject_kind", "subject_key", "purpose", "id"],
    ),
];

#[async_trait::async_trait]
impl MigrationTrait for ListIndexes {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in ADDED {
            super::index(manager, table, name, columns, false).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, _) in ADDED {
            manager
                .drop_index(Index::drop().name(name).table(Alias::new(table)).to_owned())
                .await?;
        }
        Ok(())
    }
}
