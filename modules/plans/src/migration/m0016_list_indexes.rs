//! Index cho danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): mỗi danh sách duyệt bằng
//! keyset có index cột lọc + cột sắp + id, để database nhảy thẳng tới điểm bắt đầu trang. Không index cũ
//! nào là tiền tố của index mới, nên không xoá index nào.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0016_list_indexes"
    }
}

/// `(bảng, tên index, cột)` thêm mới.
const ADDED: [(&str, &str, &[&str]); 5] = [
    // `GET /catalog`: gói đang bán theo mã.
    ("plans_plan", "plans_plan_catalog_idx", &["status", "code", "id"]),
    // `GET /tenants/{tenant_id}/overrides`: theo thứ tự tạo.
    ("plans_override", "plans_override_tenant_page_idx", &["tenant_id", "id"]),
    // `GET /billing-periods`, `GET /tenants/{tenant_id}/billing-periods`: theo thứ tự tạo.
    ("plans_billing_period", "plans_billing_period_tenant_page_idx", &["tenant_id", "id"]),
    // `GET /store/products`: theo mã sản phẩm của cửa hàng.
    ("plans_store_product", "plans_store_product_page_idx", &["product_id", "id"]),
    // `GET /store/purchases`: đổi gần nhất trước.
    ("plans_store_purchase", "plans_store_purchase_page_idx", &["tenant_id", "updated_at", "id"]),
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
