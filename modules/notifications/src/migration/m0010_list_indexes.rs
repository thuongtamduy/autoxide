//! Index cho danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): thiết bị của một người theo id
//! (`tenant_id, user_id, id`); index cũ `notifications_device_tokens_user_idx` (`tenant_id, user_id`) là tiền tố
//! của nó: xoá (`down` tạo lại). Phiên bản mẫu dùng index duy nhất sẵn có
//! (`tenant_id, kind, channel, locale, version`): `version` duy nhất trong một mẫu nên không cần thêm `id`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0010_list_indexes"
    }
}

/// `(bảng, tên index, cột)` thêm mới.
const ADDED: [(&str, &str, &[&str]); 1] = [(
    "notifications_device_tokens",
    "notifications_device_tokens_user_page_idx",
    &["tenant_id", "user_id", "id"],
)];

/// Index cũ là tiền tố của index mới cùng bảng: xoá khi lên, tạo lại khi xuống.
const REPLACED: [(&str, &str, &[&str]); 1] = [(
    "notifications_device_tokens",
    "notifications_device_tokens_user_idx",
    &["tenant_id", "user_id"],
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
        for (table, name, _) in REPLACED {
            drop(manager, table, name).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in REPLACED {
            index(manager, table, name, columns).await?;
        }
        for (table, name, _) in ADDED {
            drop(manager, table, name).await?;
        }
        Ok(())
    }
}
