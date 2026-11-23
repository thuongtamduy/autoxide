//! Migration của module `conversations`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file
//! mới và một dòng vào cuối [`all`].

mod m0001_create_conversations;
mod m0007_voice;
mod m0008_notify_window;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_conversations::Migration),
        // Bảng phụ trợ của kernel: outbox event, job, throttle (D-253).
        autoxide_kernel::event::outbox_migration("m0002_create_events", "conversations"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "conversations"),
        autoxide_kernel::job::jobs_migration("m0004_create_jobs", "conversations"),
        autoxide_kernel::throttle::throttle_migration("m0005_create_throttle", "conversations"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0006_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("conversations"),
                autoxide_kernel::job::tenant_table("conversations"),
                autoxide_kernel::throttle::tenant_table("conversations"),
            ],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "conversations"),
        // Gọi điện trong chat (D-232): đường dây, yêu cầu gọi lại, cuộc gọi, event webhook.
        Box::new(m0007_voice::Migration),
        // Khung báo tin chưa đọc bắt đầu từ tin chưa đọc đầu tiên.
        Box::new(m0008_notify_window::Migration),
    ]
}
