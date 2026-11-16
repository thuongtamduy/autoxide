//! Migration của module `notes`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm
//! file mới và một dòng vào cuối [`all`].

mod m0001_create_note;
mod m0006_org;

use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::tenancy::migration::TenantTable;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_note::Migration),
        // Bảng `notes_events` (transactional outbox), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "notes"),
        // Cột `actor_kind` của bảng `notes_events` (người lạ khác hệ thống, D-122).
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "notes"),
        // Bảng phụ trợ của kernel (event, job, throttle, token, nonce) mang `tenant_id` (D-253).
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_helpers",
            vec![autoxide_kernel::event::tenant_table("notes")],
        ),
        // Ghi chú thuộc một khách (bước 33): `tenant_id` đứng đầu mọi index — tên trùng chỉ bị
        // chặn trong cùng khách.
        autoxide_kernel::tenancy::migration::adopt(
            "m0005_tenancy",
            vec![
                TenantTable::new("notes_note")
                    .unique("notes_note_owner_title_unique", ["tenant_id", "owner_id", "title"])
                    .index("notes_note_owner_idx", ["tenant_id", "owner_id", "id"]),
            ],
        ),
        // Ghi chú thuộc một nút của cây tổ chức (bước 34).
        Box::new(m0006_org::Migration),
        // Đường người làm vào khách (bước 34, D-310).
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "notes"),
    ]
}
