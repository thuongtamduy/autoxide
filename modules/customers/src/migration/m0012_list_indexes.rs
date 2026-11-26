//! Index cho danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): mỗi danh sách duyệt bằng keyset
//! có index cột lọc + cột sắp + id. Index cũ là tiền tố của index mới thì thừa: xoá (`down` tạo lại). Index theo
//! `effective_from` của hai bảng quy tắc giữ: tìm quy tắc đang hiệu lực dùng nó.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0012_list_indexes"
    }
}

/// `(bảng, tên index, cột)` thêm mới.
const ADDED: [(&str, &str, &[&str]); 4] = [
    (
        "customers_recent_views",
        "customers_recent_views_page",
        &["tenant_id", "profile_id", "viewed_at", "product_id"],
    ),
    ("customers_reminder_rules", "customers_reminder_rules_page", &["tenant_id", "id"]),
    ("customers_referral_rules", "customers_referral_rules_page", &["tenant_id", "id"]),
    (
        "customers_gift_cards",
        "customers_gift_cards_owner_page",
        &["tenant_id", "owner_profile_id", "id"],
    ),
];

/// Index cũ là tiền tố của index mới cùng bảng: xoá khi lên, tạo lại khi xuống.
const REPLACED: [(&str, &str, &[&str]); 2] = [
    (
        "customers_recent_views",
        "customers_recent_views_profile",
        &["tenant_id", "profile_id", "viewed_at"],
    ),
    ("customers_gift_cards", "customers_gift_cards_owner", &["tenant_id", "owner_profile_id"]),
];

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
