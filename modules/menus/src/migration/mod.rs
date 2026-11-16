//! Migration của module `menus`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm
//! file mới và một dòng vào cuối [`all`].

mod m0001_create_menus;
mod m0005_add_updated_by;

use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::tenancy::migration::TenantTable;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_menus::Migration),
        // Bảng `menus_events` (transactional outbox), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "menus"),
        // Cột `actor_kind` của bảng `menus_events` (người lạ khác hệ thống, D-122).
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "menus"),
        // Bảng phụ trợ của kernel (event, job, throttle, token, nonce) mang `tenant_id` (D-253).
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_helpers",
            vec![autoxide_kernel::event::tenant_table("menus")],
        ),
        Box::new(m0005_add_updated_by::Migration),
        // Menu và mục menu thuộc một khách (bước 33, D-238): một menu cho mỗi (khách, `key`,
        // `locale`); bước 34 thêm `org_id` ngay sau `tenant_id`. Không có việc quét cả database.
        autoxide_kernel::tenancy::migration::adopt(
            "m0006_tenancy",
            vec![
                TenantTable::new("menus_menus")
                    .unique("menus_menus_key_locale_unique", ["tenant_id", "key", "locale"]),
                TenantTable::new("menus_items")
                    .index("menus_items_menu_position_idx", ["tenant_id", "menu_id", "position"]),
            ],
        ),
        // Danh sách theo khách sắp theo `id` cần index `(tenant_id, id)` (đo ở bước 33 C, D-302).
        autoxide_kernel::tenancy::migration::list_index(
            "m0007_tenant_list_index",
            vec!["menus_menus"],
        ),
    ]
}
