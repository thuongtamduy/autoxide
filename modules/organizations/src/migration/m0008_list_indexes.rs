//! Index cho danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): `GET /public/stores` duyệt
//! bằng keyset theo (`position`, `org_id`), nên index cột lọc + cột sắp + khoá phụ. Index cũ
//! `organizations_profile_public_idx` (`tenant_id`, `public`, `position`) là tiền tố của index mới: xoá
//! (`down` tạo lại).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0008_list_indexes"
    }
}

const TABLE: &str = "organizations_profile";
const ADDED: &str = "organizations_profile_public_page_idx";
const REPLACED: &str = "organizations_profile_public_idx";

fn create(name: &str, columns: &[&str]) -> IndexCreateStatement {
    let mut index = Index::create();
    index.name(name).table(Alias::new(TABLE));
    for column in columns {
        index.col(Alias::new(*column));
    }
    index.to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_index(create(ADDED, &["tenant_id", "public", "position", "org_id"])).await?;
        manager.drop_index(Index::drop().name(REPLACED).table(Alias::new(TABLE)).to_owned()).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_index(create(REPLACED, &["tenant_id", "public", "position"])).await?;
        manager.drop_index(Index::drop().name(ADDED).table(Alias::new(TABLE)).to_owned()).await
    }
}
