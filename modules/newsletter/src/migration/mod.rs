//! Migration của module `newsletter`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm file
//! mới và một dòng vào cuối [`all`].

mod m0001_create_newsletter;
mod m0009_delivery_unsubscribed;
mod m0012_subscriber_sort;
mod m0013_consent_ledger;

use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::tenancy::migration::TenantTable;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_newsletter::Migration),
        // Bảng `newsletter_events` (transactional outbox) và cột người gây ra event (người lạ
        // khác hệ thống, D-122), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "newsletter"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "newsletter"),
        // Bảng `newsletter_jobs` (job queue): chụp người nhận, gửi theo lô, dọn dẹp hằng ngày.
        autoxide_kernel::job::jobs_migration("m0004_create_jobs", "newsletter"),
        // Bảng `newsletter_throttle`: giới hạn đăng ký theo IP băm, email, người dùng; số mail
        // xác nhận gửi tới một địa chỉ (D-116).
        autoxide_kernel::throttle::throttle_migration("m0005_create_throttle", "newsletter"),
        // Bảng `newsletter_antispam_nonces`: nonce của token form — gửi lại là trả kết quả cũ
        // (D-119).
        autoxide_kernel::antispam::antispam_migration("m0006_create_antispam_nonces", "newsletter"),
        // Bảng `newsletter_tokens`: token xác nhận đăng ký (dùng một lần, có hạn — D-117).
        autoxide_kernel::tokens::tokens_migration("m0007_create_tokens", "newsletter"),
        // Bảng phụ trợ của kernel (event, job, throttle, token, nonce) mang `tenant_id` (D-253).
        autoxide_kernel::tenancy::migration::adopt(
            "m0008_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("newsletter"),
                autoxide_kernel::job::tenant_table("newsletter"),
                autoxide_kernel::throttle::tenant_table("newsletter"),
                autoxide_kernel::antispam::tenant_table("newsletter"),
                autoxide_kernel::tokens::tenant_table("newsletter"),
            ],
        ),
        // Lần huỷ đăng ký bằng link của từng thư (D-269).
        Box::new(m0009_delivery_unsubscribed::Migration),
        // Danh sách, người đăng ký, đăng ký, chiến dịch, lượt gửi và danh sách chặn thuộc một khách
        // (bước 33): cùng email / cùng `key` ở hai khách là hai dòng riêng. Mọi index bắt đầu bằng
        // `tenant_id`; job dọn dẹp tìm khách trên database rồi dọn theo từng khách, nên không cần
        // index quét cả database. Khoá chính của `newsletter_subscriptions` thêm `tenant_id` đầu
        // (helper đòi khoá chính ghép phải chứa `tenant_id`), ngữ nghĩa giữ nguyên.
        autoxide_kernel::tenancy::migration::adopt(
            "m0010_tenancy",
            vec![
                TenantTable::new("newsletter_lists")
                    .unique("newsletter_lists_key_unique", ["tenant_id", "key"])
                    .index("newsletter_lists_created_by_idx", ["tenant_id", "created_by"]),
                TenantTable::new("newsletter_subscribers")
                    .unique("newsletter_subscribers_email_unique", ["tenant_id", "email"])
                    .index("newsletter_subscribers_created_idx", ["tenant_id", "created_at"]),
                TenantTable::new("newsletter_subscriptions")
                    .primary_key(["tenant_id", "subscriber_id", "list_id"])
                    .index(
                        "newsletter_subscriptions_list_idx",
                        ["tenant_id", "list_id", "status", "subscriber_id"],
                    )
                    .index(
                        "newsletter_subscriptions_status_idx",
                        ["tenant_id", "status", "requested_at"],
                    ),
                TenantTable::new("newsletter_campaigns")
                    .unique(
                        "newsletter_campaigns_source_unique",
                        ["tenant_id", "source", "source_ref"],
                    )
                    .index("newsletter_campaigns_status_idx", ["tenant_id", "status", "id"])
                    .index("newsletter_campaigns_list_idx", ["tenant_id", "list_id", "id"])
                    .index("newsletter_campaigns_created_by_idx", ["tenant_id", "created_by"]),
                TenantTable::new("newsletter_deliveries")
                    .unique(
                        "newsletter_deliveries_unique",
                        ["tenant_id", "campaign_id", "subscriber_id"],
                    )
                    .unique(
                        "newsletter_deliveries_token_unique",
                        ["tenant_id", "unsubscribe_token_hash"],
                    )
                    .index(
                        "newsletter_deliveries_status_idx",
                        ["tenant_id", "campaign_id", "status", "id"],
                    )
                    .index(
                        "newsletter_deliveries_subscriber_idx",
                        ["tenant_id", "subscriber_id", "sent_at"],
                    ),
                TenantTable::new("newsletter_suppressions")
                    .unique("newsletter_suppressions_email_unique", ["tenant_id", "email"]),
            ],
        ),
        // Danh sách theo khách sắp theo `id` cần index `(tenant_id, id)` (đo ở bước 33 C, D-302).
        autoxide_kernel::tenancy::migration::list_index(
            "m0011_tenant_list_index",
            vec![
                "newsletter_lists",
                "newsletter_subscribers",
                "newsletter_campaigns",
                "newsletter_suppressions",
            ],
        ),
        // Đường người làm vào khách (bước 34, D-310).
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "newsletter"),
        // Sắp người đăng ký theo email / trạng thái như nhau trên hai database (W-058 của giao diện).
        Box::new(m0012_subscriber_sort::Migration),
        // Đăng ký trên sổ đồng ý chung của `compliance`, chiến dịch dạng mẫu (bước 37 C3, D-366).
        Box::new(m0013_consent_ledger::Migration),
    ]
}
