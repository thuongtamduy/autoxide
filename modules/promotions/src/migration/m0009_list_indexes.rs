//! Index cho danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): mỗi danh sách duyệt bằng
//! keyset có index cột lọc + cột sắp + id, để database nhảy thẳng tới điểm bắt đầu trang. Không index cũ
//! nào là tiền tố của index mới, nên không xoá index nào. Số liệu theo ngày và lịch sử phiên bản sắp theo
//! `day` / `version` — duy nhất trong một chương trình, nên index unique sẵn có đã đủ.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0009_list_indexes"
    }
}

/// `(bảng, tên index, cột)` thêm mới.
const ADDED: [(&str, &str, &[&str]); 1] = [
    // `GET /promotions/{id}/participations`: theo thứ tự tạo.
    (
        "promotions_participations",
        "promotions_participations_page_idx",
        &["tenant_id", "promotion_id", "id"],
    ),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in ADDED {
            super::index(manager, table, name, columns, false).await?;
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
