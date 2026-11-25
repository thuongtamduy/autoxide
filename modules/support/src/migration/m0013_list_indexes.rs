//! Index cho danh sách có phân trang (bước 70 P9 mục 3): mỗi danh sách duyệt bằng keyset có index đúng cột lọc +
//! cột sắp + id, để trang thứ nghìn nhanh như trang đầu.

use autoxide_kernel::migration::prelude::*;

use super::m0001_create_support::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0013_list_indexes"
    }
}

/// `(bảng, tên index, cột)`.
const INDEXES: [(&str, &str, &[&str]); 3] = [
    ("support_macros", "support_macros_name_page_idx", &["tenant_id", "org_id", "name", "id"]),
    ("support_reply_drafts", "support_reply_drafts_page_idx", &["tenant_id", "ticket_id", "id"]),
    (
        "support_routing_rules",
        "support_routing_rules_page_idx",
        &["tenant_id", "org_id", "position", "id"],
    ),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in INDEXES {
            index(manager, table, name, columns, false).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, _) in INDEXES {
            manager
                .drop_index(Index::drop().name(name).table(Alias::new(table)).to_owned())
                .await?;
        }
        Ok(())
    }
}
