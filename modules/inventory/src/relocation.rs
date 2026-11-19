//! Chính sách tách / gộp khách của các bảng của `inventory` (bước 38, D-20261118-step38-relocation):
//! tồn, sổ biến động và chứng từ kho đi theo nút kho (`org_id`); bảng con đi theo dòng cha.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .by_org(&[
            "inventory_levels",
            "inventory_movements",
            "inventory_adjustments",
            "inventory_settings",
            "inventory_restocks",
            "inventory_counts",
            "inventory_lots",
            "inventory_suppliers",
            "inventory_purchase_orders",
            "inventory_receipts",
            "inventory_supplier_returns",
        ])
        // Kho phục vụ đi theo nút bán; chỗ giữ theo nút bán của đơn.
        .table(
            "inventory_sources",
            autoxide_kernel::tenancy::TablePolicy::ByOrg { column: "selling_org_id" },
        )
        .table(
            "inventory_reservations",
            autoxide_kernel::tenancy::TablePolicy::ByOrg { column: "selling_org_id" },
        )
        .follow("inventory_reservation_lines", "reservation_id", "inventory_reservations")
        .follow("inventory_fulfilments", "reservation_id", "inventory_reservations")
        .follow("inventory_line_releases", "reservation_id", "inventory_reservations")
        // Đăng ký báo có hàng lại là của người mua với cả khách (cửa hàng có thể là kho online):
        // ở lại khách gốc. Bộ đếm giới hạn và nonce chống spam chỉ là tạm.
        .stay(&["inventory_restock_alerts"])
        .ephemeral(&["inventory_throttle", "inventory_antispam_nonces"])
        // Suất đặt trước là của biến thể (cả khách): chép sang khách mới khi tách.
        .copy_on_split(&["inventory_preorder_counters"])
        // Phiếu chuyển kho đi theo kho gửi.
        .table(
            "inventory_transfers",
            autoxide_kernel::tenancy::TablePolicy::ByOrg { column: "from_org_id" },
        )
        .follow("inventory_transfer_lines", "transfer_id", "inventory_transfers")
        .follow("inventory_count_lines", "count_id", "inventory_counts")
        .follow("inventory_purchase_order_lines", "purchase_order_id", "inventory_purchase_orders")
        .follow("inventory_receipt_lines", "receipt_id", "inventory_receipts")
        .follow(
            "inventory_supplier_return_lines",
            "supplier_return_id",
            "inventory_supplier_returns",
        )
        // Serial: ở kho nào đi theo kho đó; serial đã bán (không kho) ở lại khách gốc.
        .table(
            "inventory_serials",
            autoxide_kernel::tenancy::TablePolicy::ByOrg { column: "org_id" },
        )
}
