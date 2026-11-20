//! Migration của module `customers`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file mới và
//! một dòng vào cuối [`all`].

mod m0001_create_customers;
mod m0006_loyalty;
mod m0007_engagement;
mod m0008_stored_value;
mod m0011_wishlist_alerts;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        // Hồ sơ, lịch sử mua, nhóm khách (bước 47 mục 1–2).
        Box::new(m0001_create_customers::Migration),
        // Bảng `customers_events` (transactional outbox) và `customers_jobs`, định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "customers"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "customers"),
        autoxide_kernel::job::jobs_migration("m0004_create_jobs", "customers"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0005_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("customers"),
                autoxide_kernel::job::tenant_table("customers"),
            ],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "customers"),
        // Điểm thưởng, hạng, voucher tặng (bước 47 mục 3, 6).
        Box::new(m0006_loyalty::Migration),
        // Yêu thích, vừa xem, nhắc giỏ bỏ dở, giới thiệu bạn (bước 47 mục 5).
        Box::new(m0007_engagement::Migration),
        // Thẻ quà tặng và số dư cửa hàng, tắt mặc định (bước 47 mục 4).
        Box::new(m0008_stored_value::Migration),
        // Đếm lượt tra mã thẻ (chống dò mã).
        autoxide_kernel::throttle::throttle_migration("m0009_create_throttle", "customers"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0010_tenancy_throttle",
            vec![autoxide_kernel::throttle::tenant_table("customers")],
        ),
        // Báo yêu thích có hàng lại / giảm giá (bước 47, D-233).
        Box::new(m0011_wishlist_alerts::Migration),
    ]
}
