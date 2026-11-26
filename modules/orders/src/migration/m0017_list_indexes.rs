//! Index cho danh sách có phân trang (bước 70 P9 mục 3, D-20261125-list-pages): mỗi danh sách duyệt bằng keyset
//! có index cột lọc + cột sắp + id, để database nhảy thẳng tới điểm bắt đầu trang. Index cũ là tiền tố của index
//! mới thì thừa: xoá (`down` tạo lại).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0017_list_indexes"
    }
}

/// `(bảng, tên index, cột)` thêm mới.
const ADDED: [(&str, &str, &[&str]); 7] = [
    // `GET /warranties` (serial hay số điện thoại) và `GET /me/warranties`: theo thứ tự tạo.
    ("orders_warranties", "orders_warranties_serial_page_idx", &["tenant_id", "serial", "id"]),
    ("orders_warranties", "orders_warranties_key_page_idx", &["tenant_id", "buyer_key", "id"]),
    (
        "orders_warranties",
        "orders_warranties_buyer_page_idx",
        &["tenant_id", "buyer_user_id", "id"],
    ),
    // `GET /{id}/refunds`: theo thứ tự tạo.
    ("orders_refunds", "orders_refunds_order_page_idx", &["tenant_id", "order_id", "id"]),
    // `GET /me/orders/{id}/documents`, `GET /public/orders/{id}/documents`: theo thứ tự tạo.
    (
        "orders_invoice_links",
        "orders_invoice_links_order_page_idx",
        &["tenant_id", "order_id", "id"],
    ),
    // `GET /risk/cod-rules`: theo thứ tự tạo.
    ("orders_cod_rules", "orders_cod_rules_page_idx", &["tenant_id", "id"]),
    // `GET /risk/cod-blocks`: hết hạn sớm nhất trước.
    ("orders_cod_blocks", "orders_cod_blocks_page_idx", &["tenant_id", "expires_at", "id"]),
];

/// Index cũ là tiền tố của index mới cùng bảng: xoá khi lên, tạo lại khi xuống.
const REPLACED: [(&str, &str, &[&str]); 5] = [
    ("orders_warranties", "orders_warranties_serial_idx", &["tenant_id", "serial"]),
    ("orders_warranties", "orders_warranties_key_idx", &["tenant_id", "buyer_key"]),
    ("orders_warranties", "orders_warranties_buyer_idx", &["tenant_id", "buyer_user_id"]),
    ("orders_refunds", "orders_refunds_order_idx", &["tenant_id", "order_id"]),
    ("orders_invoice_links", "orders_invoice_links_order_idx", &["tenant_id", "order_id"]),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in ADDED {
            super::index(manager, table, name, columns, false).await?;
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
            super::index(manager, table, name, columns, false).await?;
        }
        for (table, name, _) in ADDED {
            manager
                .drop_index(Index::drop().name(name).table(Alias::new(table)).to_owned())
                .await?;
        }
        Ok(())
    }
}
