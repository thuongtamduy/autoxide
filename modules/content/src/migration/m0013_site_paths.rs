//! Đường dẫn công khai riêng theo website (bước 34, D-307): hai cửa hàng cùng có `/about`. Khoá
//! unique của bản công khai (đường dẫn) và của bản nháp (slug cùng chỗ) thêm `org_id`; website giải
//! đường dẫn theo nút gần nhất trên chuỗi website → gốc (`ContentService::resolve`).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0013_site_paths"
    }
}

/// (bảng, tên khoá unique, cột sau `org_id` ở bản mới).
const UNIQUES: [(&str, &str, &[&str]); 2] = [
    ("content_published", "content_published_path_unique", &["locale", "path"]),
    ("content_entries", "content_entries_slug_unique", &["locale", "kind", "parent_key", "slug"]),
];

fn unique(table: &str, name: &str, columns: &[&str]) -> IndexCreateStatement {
    let mut index = Index::create();
    index.name(name).table(Alias::new(table)).unique();
    for column in columns {
        index.col(Alias::new(*column));
    }
    index.to_owned()
}

fn drop_unique(table: &str, name: &str) -> IndexDropStatement {
    Index::drop().name(name).table(Alias::new(table)).to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, rest) in UNIQUES {
            manager.drop_index(drop_unique(table, name)).await?;
            let columns: Vec<&str> =
                ["tenant_id", "org_id"].into_iter().chain(rest.iter().copied()).collect();
            manager.create_index(unique(table, name, &columns)).await?;
        }
        Ok(())
    }

    // Lùi lại khi hai website đã cùng dùng một slug / đường dẫn thì vấp unique: đổi một bên trước.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, rest) in UNIQUES {
            manager.drop_index(drop_unique(table, name)).await?;
            let columns: Vec<&str> =
                ["tenant_id"].into_iter().chain(rest.iter().copied()).collect();
            manager.create_index(unique(table, name, &columns)).await?;
        }
        Ok(())
    }
}
