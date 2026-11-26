//! Index cho danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): cuộc gọi của một cuộc trò
//! chuyện theo id (`tenant_id, thread_id, id`). Index cũ `conversations_calls_thread_idx`
//! (`tenant_id, thread_id, created_at`) không phải tiền tố của nó: giữ.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0009_list_indexes"
    }
}

/// `(bảng, tên index, cột)` thêm mới.
const ADDED: [(&str, &str, &[&str]); 1] = [(
    "conversations_calls",
    "conversations_calls_thread_page_idx",
    &["tenant_id", "thread_id", "id"],
)];

async fn index(
    manager: &SchemaManager<'_>,
    table: &str,
    name: &str,
    columns: &[&str],
) -> Result<(), DbErr> {
    let mut index = Index::create();
    index.name(name).table(Alias::new(table));
    for column in columns {
        index.col(Alias::new(*column));
    }
    manager.create_index(index.to_owned()).await
}

async fn drop(manager: &SchemaManager<'_>, table: &str, name: &str) -> Result<(), DbErr> {
    manager.drop_index(Index::drop().name(name).table(Alias::new(table)).to_owned()).await
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in ADDED {
            index(manager, table, name, columns).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, _) in ADDED {
            drop(manager, table, name).await?;
        }
        Ok(())
    }
}
