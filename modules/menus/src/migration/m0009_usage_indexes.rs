//! Index cho "đang dùng ở đâu" (bước 70 P9 mục 3, D-20261126-list-pages-done): mục menu trỏ tới một bản ghi
//! — đích của mục (`ref_kind`, `ref_id`) hay ô gợi ý (`feature_kind`, `feature_id`) — đọc theo trang keyset
//! trên `id`. Mỗi nhánh của điều kiện `OR` có index (tenant_id, cột loại, cột id, id) để database không quét
//! mọi mục của khách. Không index cũ nào là tiền tố của hai index này.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0009_usage_indexes"
    }
}

/// `(tên index, cột)` thêm vào `menus_items`.
const ADDED: [(&str, [&str; 4]); 2] = [
    ("menus_items_ref_idx", ["tenant_id", "ref_kind", "ref_id", "id"]),
    ("menus_items_feature_idx", ["tenant_id", "feature_kind", "feature_id", "id"]),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (name, columns) in ADDED {
            let mut index = Index::create();
            index.name(name).table(Alias::new("menus_items"));
            for column in columns {
                index.col(Alias::new(column));
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (name, _) in ADDED {
            manager
                .drop_index(Index::drop().name(name).table(Alias::new("menus_items")).to_owned())
                .await?;
        }
        Ok(())
    }
}
