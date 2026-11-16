//! Migration của module `webhooks`, theo thứ tự áp dụng. Đã commit thì không sửa —
//! thêm file mới và một dòng vào cuối [`all`].

mod m0001_create_webhooks;
mod m0007_widen_secret;
pub(crate) mod u0001_seal_secrets;

use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::tenancy::migration::TenantTable;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_webhooks::Migration),
        // Bảng `webhooks_events` (transactional outbox), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "webhooks"),
        // Bảng `webhooks_jobs` (job queue): mỗi lần gửi là một job, có retry.
        autoxide_kernel::job::jobs_migration("m0003_create_jobs", "webhooks"),
        // Cột `actor_kind` của bảng `webhooks_events` (người lạ khác hệ thống, D-122).
        autoxide_kernel::event::outbox_actor_migration("m0004_add_event_actor", "webhooks"),
        // Bảng phụ trợ của kernel (event, job, throttle, token, nonce) mang `tenant_id` (D-253).
        autoxide_kernel::tenancy::migration::adopt(
            "m0005_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("webhooks"),
                autoxide_kernel::job::tenant_table("webhooks"),
            ],
        ),
        // Endpoint và lần gửi thuộc một khách (bước 33, D-278); index dọn lần gửi cũ quét cả
        // database. Ngay sau đó upgrade `u0001_seal_secrets` mã hoá secret ký đã có.
        autoxide_kernel::tenancy::migration::adopt(
            "m0006_tenancy",
            vec![
                TenantTable::new("webhooks_endpoint"),
                TenantTable::new("webhooks_delivery")
                    .unique(
                        "webhooks_delivery_endpoint_event_unique",
                        ["tenant_id", "endpoint_id", "event_id"],
                    )
                    .index("webhooks_delivery_endpoint_idx", ["tenant_id", "endpoint_id", "id"])
                    .drop_index("webhooks_delivery_created_idx")
                    .sweep_index("webhooks_delivery_created_sweep_idx", ["created_at"]),
            ],
        ),
        // Secret đã mã hoá dài hơn cột cũ (Postgres).
        Box::new(m0007_widen_secret::Migration),
        // Danh sách theo khách sắp theo `id` cần index `(tenant_id, id)` (đo ở bước 33 C, D-302).
        autoxide_kernel::tenancy::migration::list_index(
            "m0008_tenant_list_index",
            vec!["webhooks_endpoint"],
        ),
    ]
}

/// Upgrade dữ liệu của module, theo thứ tự.
pub(crate) fn upgrades() -> Vec<Box<dyn autoxide_kernel::migration::DataUpgrade>> {
    vec![Box::new(u0001_seal_secrets::SealSecrets)]
}
