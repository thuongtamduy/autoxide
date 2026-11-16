//! Migration của module `audit`, theo thứ tự áp dụng. Đã commit thì không sửa —
//! thêm file mới và một dòng vào cuối [`all`].

mod m0001_create_entry;
mod m0002_add_actor_kind;
mod m0003_add_module_index;
mod m0004_add_hash_chain;

use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::tenancy::migration::TenantTable;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_entry::Migration),
        Box::new(m0002_add_actor_kind::Migration),
        Box::new(m0003_add_module_index::Migration),
        Box::new(m0004_add_hash_chain::Migration),
        // Mỗi khách một chuỗi băm riêng (bước 33, D-276): đầu chuỗi khoá theo (khách, tên), số
        // thứ tự duy nhất trong khách; mọi index bắt đầu bằng `tenant_id`.
        autoxide_kernel::tenancy::migration::adopt(
            "m0005_tenancy",
            vec![
                TenantTable::new("audit_entry")
                    .index("audit_entry_actor_idx", ["tenant_id", "actor_user_id", "id"])
                    .index("audit_entry_subject_idx", ["tenant_id", "subject", "id"])
                    .index("audit_entry_action_idx", ["tenant_id", "action", "id"])
                    .index("audit_entry_module_idx", ["tenant_id", "module", "id"])
                    .unique("audit_entry_sequence_idx", ["tenant_id", "sequence"]),
                TenantTable::new("audit_chain").primary_key(["tenant_id", "name"]),
            ],
        ),
        // Danh sách theo khách sắp theo `id` cần index `(tenant_id, id)` (đo ở bước 33 C, D-302).
        autoxide_kernel::tenancy::migration::list_index(
            "m0006_tenant_list_index",
            vec!["audit_entry"],
        ),
    ]
}
