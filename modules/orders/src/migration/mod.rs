//! Migration của module `orders`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file mới và
//! một dòng vào cuối [`all`].

mod m0005_carts;
mod m0006_orders;
mod m0007_returns;
mod m0008_pickups;
mod m0009_preorders;

use autoxide_kernel::migration::prelude::*;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        // Bảng `orders_events` (transactional outbox) và `orders_jobs`, định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0001_create_events", "orders"),
        autoxide_kernel::event::outbox_actor_migration("m0002_add_event_actor", "orders"),
        autoxide_kernel::job::jobs_migration("m0003_create_jobs", "orders"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("orders"),
                autoxide_kernel::job::tenant_table("orders"),
            ],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "orders"),
        // Giỏ hàng và dòng giỏ (mục 2).
        Box::new(m0005_carts::Migration),
        // Lượt đặt, đơn, dòng, thuế, lịch sử, khoản thanh toán, hoãn hoàn tất (mục 3).
        Box::new(m0006_orders::Migration),
        // Hoàn tiền và đổi trả (mục 6).
        Box::new(m0007_returns::Migration),
        // Nhận tại cửa hàng (mục 5).
        Box::new(m0008_pickups::Migration),
        // Hàng đặt trước, giao nhiều lần (mục 5).
        Box::new(m0009_preorders::Migration),
    ]
}

/// Tạo index `name` trên `table`.
pub(crate) async fn index(
    manager: &SchemaManager<'_>,
    table: &str,
    name: &str,
    columns: &[&str],
    unique: bool,
) -> Result<(), DbErr> {
    let mut index = Index::create();
    index.name(name).table(Alias::new(table));
    if unique {
        index.unique();
    }
    for column in columns {
        index.col(Alias::new(*column));
    }
    manager.create_index(index.to_owned()).await
}
