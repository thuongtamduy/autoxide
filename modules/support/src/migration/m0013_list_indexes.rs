//! Index cho danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): mỗi danh sách duyệt bằng keyset
//! có index cột lọc + cột sắp + id, để database nhảy thẳng tới điểm bắt đầu trang. Index cũ là tiền tố của index
//! mới thì thừa: xoá (`down` tạo lại).
//!
//! Giới hạn đã biết: câu trả lời mẫu lọc theo CHUỖI nút (`org_id IN (…)`), nên index không cho sẵn thứ tự trên
//! cả chuỗi — database gộp các nút rồi sắp (SQLite dùng `TEMP B-TREE`); vẫn đúng, nhưng chi phí tăng theo số
//! câu của chuỗi nút chứ không chỉ theo cỡ trang.

use autoxide_kernel::migration::prelude::*;

use super::m0001_create_support::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0013_list_indexes"
    }
}

/// `(bảng, tên index, cột)` thêm mới.
const ADDED: [(&str, &str, &[&str]); 3] = [
    ("support_macros", "support_macros_name_page_idx", &["tenant_id", "org_id", "name", "id"]),
    ("support_reply_drafts", "support_reply_drafts_page_idx", &["tenant_id", "ticket_id", "id"]),
    (
        "support_routing_rules",
        "support_routing_rules_page_idx",
        &["tenant_id", "org_id", "position", "id"],
    ),
];

/// Index cũ là tiền tố của index mới cùng bảng: xoá khi lên, tạo lại khi xuống.
const REPLACED: [(&str, &str, &[&str]); 3] = [
    ("support_macros", "support_macros_org_idx", &["tenant_id", "org_id"]),
    ("support_reply_drafts", "support_reply_drafts_ticket_idx", &["tenant_id", "ticket_id"]),
    (
        "support_routing_rules",
        "support_routing_rules_org_idx",
        &["tenant_id", "org_id", "position"],
    ),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in ADDED {
            index(manager, table, name, columns, false).await?;
        }
        for (table, name, _) in REPLACED {
            manager
                .drop_index(Index::drop().name(name).table(Alias::new(table)).to_owned())
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in REPLACED {
            index(manager, table, name, columns, false).await?;
        }
        for (table, name, _) in ADDED {
            manager
                .drop_index(Index::drop().name(name).table(Alias::new(table)).to_owned())
                .await?;
        }
        Ok(())
    }
}
