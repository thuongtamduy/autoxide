//! Index cho danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): mỗi danh sách duyệt bằng keyset
//! có index cột lọc + cột sắp + id, để database nhảy thẳng tới điểm bắt đầu trang. Index cũ là tiền tố của index
//! mới thì thừa: xoá (`down` tạo lại).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0009_list_indexes"
    }
}

/// `(bảng, tên index, cột)` thêm mới.
const ADDED: [(&str, &str, &[&str]); 3] = [
    (
        "bookings_exceptions",
        "bookings_exceptions_page_idx",
        &["tenant_id", "resource_id", "starts_at", "id"],
    ),
    // Dịch vụ công khai theo tên: không lọc nút, và lọc một nút.
    (
        "bookings_services",
        "bookings_services_public_page_idx",
        &["tenant_id", "active", "name", "id"],
    ),
    (
        "bookings_services",
        "bookings_services_public_org_page_idx",
        &["tenant_id", "org_id", "active", "name", "id"],
    ),
];

/// Index cũ là tiền tố của index mới cùng bảng: xoá khi lên, tạo lại khi xuống.
const REPLACED: [(&str, &str, &[&str]); 1] = [(
    "bookings_exceptions",
    "bookings_exceptions_resource_idx",
    &["tenant_id", "resource_id", "starts_at"],
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

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in ADDED {
            index(manager, table, name, columns).await?;
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
            index(manager, table, name, columns).await?;
        }
        for (table, name, _) in ADDED {
            manager
                .drop_index(Index::drop().name(name).table(Alias::new(table)).to_owned())
                .await?;
        }
        Ok(())
    }
}
