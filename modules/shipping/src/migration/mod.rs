//! Migration của module `shipping`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file mới
//! và một dòng vào cuối [`all`].

mod m0001_create_shipping;

use autoxide_kernel::migration::prelude::*;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_shipping::Migration),
        // Bảng `shipping_events` (transactional outbox) và `shipping_jobs`, định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "shipping"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "shipping"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_events",
            vec![autoxide_kernel::event::tenant_table("shipping")],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "shipping"),
        // Job: đặt với hãng, xử lý webhook, ghi sổ cái COD, nhập bảng kê.
        autoxide_kernel::job::jobs_migration("m0005_create_jobs", "shipping"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0006_tenancy_jobs",
            vec![autoxide_kernel::job::tenant_table("shipping")],
        ),
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

/// Bảng có khoá chính `id` (UUIDv7 sinh trong ứng dụng) và `tenant_id`.
pub(crate) fn table(name: &str) -> TableCreateStatement {
    Table::create()
        .table(Alias::new(name))
        .col(schema::pk_uuid("id"))
        .col(schema::uuid("tenant_id"))
        .to_owned()
}

/// Thêm các cột của một địa chỉ (`Address` của module `address`, cộng mã bưu chính cho địa chỉ
/// nước ngoài). Tên tỉnh / xã chụp lúc ghi: danh mục đổi sau không đổi địa chỉ đã lưu.
pub(crate) fn address_columns(table: &mut TableCreateStatement) {
    table
        .col(schema::string_len("country", 2))
        .col(schema::string_len("province_code", 64))
        .col(schema::string_len("province_name", 200))
        .col(schema::string_len("commune_code", 64))
        .col(schema::string_len("commune_name", 200))
        .col(schema::string_len("line", 300))
        .col(schema::string_len_null("postal_code", 20));
}
