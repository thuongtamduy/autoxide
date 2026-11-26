//! Index cho danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): mỗi danh sách duyệt bằng keyset
//! có index cột lọc + cột sắp + id, để database nhảy thẳng tới điểm bắt đầu trang.
//!
//! - Lịch của một mục (`GET /entries/{id}/schedules`): `(tenant_id, entry_id, id)`. Index cũ
//!   `content_schedules_entry_idx` (`tenant_id, entry_id, status`) không phải tiền tố: giữ (job tìm lịch chờ).
//! - Menu theo khoá: `(tenant_id, key, id)`. Index duy nhất `(tenant_id, key, locale)` giữ (ràng buộc).
//! - Vị trí hiển thị theo khoá: không thêm — khoá duy nhất trong khách (`content_placements_key_unique`,
//!   `tenant_id, key`), nên hai dòng không bao giờ cùng khoá và index duy nhất đã cho đúng thứ tự trang.
//! - Feed công khai dùng `content_published_list_idx` sẵn có (`tenant_id, locale, kind, published_at, entry_id`).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0035_list_indexes"
    }
}

/// `(bảng, tên index, cột)` thêm mới.
const ADDED: [(&str, &str, &[&str]); 2] = [
    ("content_schedules", "content_schedules_entry_page_idx", &["tenant_id", "entry_id", "id"]),
    ("content_menus", "content_menus_page_idx", &["tenant_id", "key", "id"]),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in ADDED {
            let mut index = Index::create();
            index.name(name).table(Alias::new(table));
            for column in columns {
                index.col(Alias::new(*column));
            }
            manager.create_index(index.to_owned()).await?;
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
