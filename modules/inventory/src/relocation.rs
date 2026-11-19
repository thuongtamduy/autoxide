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
        // Phiếu chuyển kho đi theo kho gửi.
        .table(
            "inventory_transfers",
            autoxide_kernel::tenancy::TablePolicy::ByOrg { column: "from_org_id" },
        )
        .follow("inventory_transfer_lines", "transfer_id", "inventory_transfers")
        .follow("inventory_count_lines", "count_id", "inventory_counts")
        // Serial: ở kho nào đi theo kho đó; serial đã bán (không kho) ở lại khách gốc.
        .table(
            "inventory_serials",
            autoxide_kernel::tenancy::TablePolicy::ByOrg { column: "org_id" },
        )
}
