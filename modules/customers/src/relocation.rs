//! Chính sách tách / gộp khách của các bảng của `customers` (bước 38, D-20261118-step38-relocation):
//! hồ sơ và nhóm theo nút; lịch sử đơn, khoản hoàn và thành viên nhóm đi theo bản ghi cha.

use autoxide_kernel::tenancy::{Relocation, TablePolicy};

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .by_org(&["customers_profiles", "customers_groups"])
        .follow("customers_order_facts", "profile_id", "customers_profiles")
        .follow("customers_group_members", "group_id", "customers_groups")
        .follow("customers_points_balances", "profile_id", "customers_profiles")
        .follow("customers_points_lots", "profile_id", "customers_profiles")
        .follow("customers_points_entries", "profile_id", "customers_profiles")
        .follow("customers_point_holds", "profile_id", "customers_profiles")
        // Chương trình điểm chung của khách: tách khách thì khách mới có bản sao.
        .copy_on_split(&[
            "customers_loyalty_rules",
            "customers_tiers",
            "customers_voucher_programs",
        ])
        // Khoá của dòng đơn là `order_id` (không có cột `id`).
        .table(
            "customers_refund_facts",
            TablePolicy::FollowParent {
                parent_table: "customers_order_facts",
                column: "order_id",
                parent_column: "order_id",
            },
        )
}
